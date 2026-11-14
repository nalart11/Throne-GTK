import Foundation

// Configuration validation only: never connects a proxy, enables VPN, or invokes a helper.
@main
struct CoreSmoke {
    @MainActor static func main() async throws {
        let executable = URL(fileURLWithPath: CommandLine.arguments[0]).standardizedFileURL
        guard executable.lastPathComponent == "nolurix" else {
            throw BridgeFailure.response("Run the harness as nolurix to preserve core parent identity")
        }
        let core = executable.deletingLastPathComponent().appendingPathComponent("nolurix-core")
        guard FileManager.default.isExecutableFile(atPath: core.path) else {
            throw BridgeFailure.response("Missing adjacent built core: \(core.path)")
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("nolurix-core-check-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let options = try Model.encode(["data_dir": directory.path, "core_path": core.path])
        for cycle in 1...2 {
            let model = Model()
            await model.start(optionsJSON: options, allowAutoConnect: false).value
            do {
                guard model.ready, model.error == nil else {
                    throw BridgeFailure.response(model.error ?? "Bridge did not open")
                }
                guard model.snapshot["notice"] as? String == nil else {
                    throw BridgeFailure.response("Reopened bridge retained stale completion notice")
                }
                _ = try await model.perform("settings_patch", ["patch": ["language": "en", "mode": "proxy", "connect_last_on_start": false]])
                if model.profiles.isEmpty {
                    _ = try await model.perform("import_links", ["gid": model.groups.first?["id"] ?? 1,
                        "text": "socks5://127.0.0.1:9#Offline%20configuration%20fixture"])
                }
                guard let id = model.profiles.first?["id"] as? Int else {
                    throw BridgeFailure.response("Synthetic SOCKS profile was not imported")
                }
                _ = try await model.perform("check_config", ["id": id])
                let deadline = ContinuousClock.now.advanced(by: .seconds(30))
                while true {
                    await model.refresh()
                    if let error = model.error { throw BridgeFailure.response(error) }
                    if let notice = model.snapshot["notice"] as? String, !notice.isEmpty {
                        guard notice == "Configuration OK" else { throw BridgeFailure.response(notice) }
                        break
                    }
                    guard ContinuousClock.now < deadline else {
                        throw BridgeFailure.response("Timed out waiting for actual ConfigChecked success")
                    }
                    try await Task.sleep(for: .milliseconds(100))
                }
                try await model.closeBridge() // C ABI close joins the real engine/core lifecycle.
                print("PASS cycle \(cycle): actual Configuration OK, bridge closed/joined")
            } catch {
                // Keep isolated diagnostics if close cannot confirm process shutdown.
                do { try await model.closeBridge() }
                catch { throw BridgeFailure.response("Close failed; retained \(directory.path): \(error)") }
                try? FileManager.default.removeItem(at: directory)
                throw error
            }
        }
        try FileManager.default.removeItem(at: directory)
        print("PASS: two real-core offline configuration checks/reopens; no network/proxy/VPN/helper operation requested")
    }
}
