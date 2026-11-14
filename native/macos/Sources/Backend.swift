import Foundation
import Combine

enum BridgeFailure: Error, CustomStringConvertible, LocalizedError {
    case response(String), closed
    var description: String { switch self { case .response(let text): text; case .closed: "Bridge is closed" } }
    var errorDescription: String? { description }
}

// All blocking C calls stay on this non-main actor. Only Sendable JSON text
// crosses its boundary; Foundation's heterogeneous objects remain local.
actor Backend {
    private var handle: UInt64?
    private let beforeClose: (@Sendable () throws -> Void)?
    init(beforeClose: (@Sendable () throws -> Void)? = nil) { self.beforeClose = beforeClose }
    private func decode(_ pointer: UnsafeMutablePointer<CChar>?) throws -> String {
        guard let pointer else { throw BridgeFailure.response("Null bridge response") }
        defer { nolurix_macos_string_free(pointer) }
        let data = Data(String(cString: pointer).utf8)
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw BridgeFailure.response("Invalid bridge response")
        }
        guard root["ok"] as? Bool == true else { throw BridgeFailure.response(root["error"] as? String ?? "Backend error") }
        return String(decoding: try JSONSerialization.data(withJSONObject: root["value"] ?? NSNull(), options: [.fragmentsAllowed]), as: UTF8.self)
    }
    func open(optionsJSON: String = "{}") throws {
        guard handle == nil else { return }
        let raw = try optionsJSON.withCString { try decode(nolurix_macos_open($0)) }
        guard let number = try JSONSerialization.jsonObject(with: Data(raw.utf8), options: [.fragmentsAllowed]) as? NSNumber,
              number.uint64Value > 0 else { throw BridgeFailure.response("Bridge did not return a numeric handle") }
        handle = number.uint64Value
    }
    func send(_ json: String) throws -> String {
        guard let handle else { throw BridgeFailure.closed }
        return try json.withCString { try decode(nolurix_macos_request(handle, $0)) }
    }
    func poll() throws -> String {
        guard let handle else { throw BridgeFailure.closed }
        return try decode(nolurix_macos_poll(handle, 100))
    }
    func close() throws {
        guard let current = handle else { return }
        try beforeClose?()
        _ = try decode(nolurix_macos_close(current))
        handle = nil // Failed close retains the exact handle for retry.
    }
}

@MainActor final class Model: ObservableObject {
    @Published var snapshot: [String: Any] = [:]
    @Published var error: String?
    @Published var busy = false
    @Published var confirmStartupVPN = false
    @Published var ready = false
    @Published var tab = "profiles"
    @Published var language = "ru"
    @Published var operationNotice: String?
    @Published private(set) var shutdownPending = false
    private let backend: Backend
    init(backend: Backend = Backend()) { self.backend = backend }
    private var startTask: Task<Void, Never>?
    private var pollTask: Task<Void, Never>?
    private var didStart = false
    func tr(_ ru: String, _ en: String) -> String { language == "ru" ? ru : en }
    var profiles: [[String: Any]] { snapshot["profiles"] as? [[String: Any]] ?? [] }
    var groups: [[String: Any]] { snapshot["groups"] as? [[String: Any]] ?? [] }
    var settings: [String: Any] { snapshot["settings"] as? [String: Any] ?? [:] }
    var status: [String: Any] { snapshot["status"] as? [String: Any] ?? [:] }
    static func encode(_ value: [String: Any]) throws -> String {
        String(decoding: try JSONSerialization.data(withJSONObject: value), as: UTF8.self)
    }
    static func decode(_ json: String) throws -> Any {
        try JSONSerialization.jsonObject(with: Data(json.utf8), options: [.fragmentsAllowed])
    }
    @discardableResult func start(optionsJSON: String? = nil, allowAutoConnect: Bool = true) -> Task<Void, Never> {
        if let startTask { return startTask }
        if didStart { return Task {} }
        didStart = true
        startTask = Task { [weak self] in
            guard let self else { return }
            do {
                var options: [String: Any] = [:]
                let args = CommandLine.arguments
                if let flag = args.firstIndex(of: "--data-dir"), args.indices.contains(flag + 1) { options["data_dir"] = args[flag + 1] }
                if args.contains("--smoke"), options["data_dir"] == nil {
                    options["data_dir"] = FileManager.default.temporaryDirectory.appendingPathComponent("nolurix-macos-smoke-\(UUID().uuidString)").path
                }
                let text = try optionsJSON ?? Self.encode(options)
                try await backend.open(optionsJSON: text)
                try await refreshThrowing()
                beginPolling()
                if allowAutoConnect, !args.contains("--smoke"), settings["connect_last_on_start"] as? Bool == true {
                    if settings["mode"] as? String == "vpn" { confirmStartupVPN = true } else { command("connect") }
                }
            } catch { setError(error) }
        }
        return startTask!
    }
    private func beginPolling() {
        guard pollTask == nil, !shutdownPending else { return }
        pollTask = Task { [weak self, backend] in
            while !Task.isCancelled {
                do {
                    let raw = try await backend.poll()
                    guard !Task.isCancelled else { break }
                    if let event = try Self.decode(raw) as? [String: Any], event["kind"] as? String == "changed" { await self?.refresh() }
                } catch { if !Task.isCancelled { self?.setError(error) }; break }
            }
        }
    }
    func setError(_ failure: Error) { error = String(describing: failure); busy = false }
    private func refreshThrowing() async throws {
        let raw = try await backend.send("{\"op\":\"snapshot\"}")
        guard let value = try Self.decode(raw) as? [String: Any] else { throw BridgeFailure.response("Invalid snapshot") }
        snapshot = value; ready = true
        language = settings["language"] as? String == "en" ? "en" : "ru"
    }
    func refresh() async { do { try await refreshThrowing() } catch { setError(error) } }
    @discardableResult func perform(_ op: String, _ fields: [String: Any] = [:]) async throws -> Any {
        if shutdownPending, !["snapshot", "logs_clear"].contains(op) { throw BridgeFailure.response(tr("Остановка ещё не подтверждена; повторите выход.", "Shutdown is pending; retry quitting.")) }
        if op != "snapshot" { operationNotice = nil }
        let text = try Self.encode(["op": op].merging(fields) { _, new in new })
        let raw = try await backend.send(text)
        let value = try Self.decode(raw)
        try await refreshThrowing()
        if let result = value as? [String: Any] {
            switch op {
            case "import_links":
                operationNotice = tr("Импортировано профилей: ", "Profiles imported: ") + String(describing: result["profiles"] ?? 0)
                if let errors = result["errors"] as? [String], !errors.isEmpty { operationNotice! += "\n" + errors.joined(separator: "\n") }
            case "import_throne", "import_routes":
                operationNotice = [tr("Импорт завершён", "Import complete"),
                    tr("Группы", "Groups") + ": \(result["groups"] ?? 0)",
                    tr("Профили", "Profiles") + ": \(result["profiles"] ?? 0)",
                    tr("Правила", "Rules") + ": \(result["rules"] ?? 0)",
                    tr("Пропущено правил", "Skipped rules") + ": \(result["skipped_rules"] ?? 0)"].joined(separator: " · ")
            default: break
            }
        }
        return value
    }
    func command(_ op: String, _ fields: [String: Any] = [:], completion: ((Any?) -> Void)? = nil) {
        busy = true
        Task { do { let result = try await perform(op, fields); busy = false; completion?(result) } catch { setError(error) } }
    }
    func patch(_ fields: [String: Any]) { command("settings_patch", ["patch": fields]) }
    func recoverBridge(_ completion: @escaping (Bool) -> Void) {
        // Never open a replacement engine after an unconfirmed shutdown.
        Task { do { try await refreshThrowing(); completion(!shutdownPending) } catch { setError(error); completion(false) } }
    }
    func closeBridge() async throws {
        shutdownPending = true
        await startTask?.value
        pollTask?.cancel()
        await pollTask?.value
        pollTask = nil
        try await backend.close()
        ready = false; didStart = false; startTask = nil; shutdownPending = false
    }
    func shutdown(reply: @escaping (Bool) -> Void) {
        Task { do { try await closeBridge(); reply(true) } catch { setError(error); reply(false) } }
    }
}
