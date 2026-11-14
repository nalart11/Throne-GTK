import Foundation

struct FrontendSmokeError: Error { let message: String }
// Inject one transport failure without launching a core or altering the Rust ABI.
final class CloseGate: @unchecked Sendable {
    private let lock = NSLock()
    private var first = true
    func check() throws {
        lock.lock(); defer { lock.unlock() }
        if first { first = false; throw FrontendSmokeError(message: "controlled close transport failure") }
    }
}

@main
struct FrontendSmoke {
    @MainActor static func checkProfileSearch() throws {
        let groups: [[String: Any]] = [["id": 1], ["id": 2]]
        let populated: [[String: Any]] = [["id": 7, "gid": 2, "name": "Subscription"]]
        for selectedID in [0, 99, -99, Int.min] {
            try require(ProfileSearch.initialGroupID(groups: groups, profiles: populated, selectedID: selectedID) == 2,
                        "initial group hid populated subscription for absent/stale selection")
        }
        try require(ProfileSearch.initialGroupID(groups: groups, profiles: populated, selectedID: 7) == 2,
                    "selected profile group lost")
        try require(ProfileSearch.initialGroupID(groups: groups, profiles: populated, selectedID: -1) == 1,
                    "explicit automatic group selection lost")
        try require(ProfileSearch.initialGroupID(groups: groups, profiles: [], selectedID: 0) == 1,
                    "empty store first group fallback lost")
        try require(ProfileSearch.initialGroupID(groups: [], profiles: [], selectedID: 0) == 0,
                    "empty store default fallback lost")
        let expandedGroups: [[String: Any]] = [["id": 1], ["id": 2], ["id": 3]]
        try require(ProfileSearch.reconciledGroupID(currentGroupID: 2, groups: expandedGroups, profiles: populated, selectedID: 7) == 2,
                    "group-list update replaced a valid user selection")
        try require(ProfileSearch.reconciledGroupID(currentGroupID: 0, groups: expandedGroups, profiles: populated, selectedID: 0) == 2,
                    "groups arriving after readiness did not choose a populated group")
        try require(ProfileSearch.reconciledGroupID(currentGroupID: 3, groups: Array(expandedGroups.dropLast()), profiles: populated, selectedID: 0) == 2,
                    "removed selected group did not recover to a populated group")
        try require(ProfileSearch.reconciledGroupID(currentGroupID: 3, groups: expandedGroups, profiles: populated, selectedID: 7) == 3,
                    "valid user group selection was not preserved over backend selection")
        let profiles: [[String: Any]] = [
            ["gid": 1, "name": "Example profile", "outbound": ["server": "Edge.Example", "server_port": 443]],
            ["gid": 2, "name": "Other group", "outbound": ["server": "other.invalid"]]
        ]
        for query in ["", " \n\t", "EXAMPLE PROFILE", "edge.example", "443"] {
            try require(profiles.filter { ProfileSearch.matches($0, groupID: 1, query: query) }.count == 1,
                        "profile search lost group/name/address match: \(query)")
        }
        try require(profiles.filter { ProfileSearch.matches($0, groupID: 1, query: "missing") }.isEmpty,
                    "profile search returned unmatched profiles")
        try require(profiles.filter { ProfileSearch.matches($0, groupID: 3, query: "") }.isEmpty,
                    "empty search ignored group filter")
    }
    @MainActor static func checkRoutingSerialization() throws {
        let regex = #"^edge[0-9]{1,3}\.example,(alpha|beta)$"#
        let fixture: [[String: Any]] = ["any", "all"].map { mode in
            ["name": "Roundtrip \(mode)", "enabled": true, "match_mode": mode, "action": "proxy",
             "conditions": [
                ["kind": "domain_regex", "values": [regex, #"^cdn\.(one|two)\.example$"#]],
                ["kind": "port", "values": ["443", "8000:8100"]]
             ]]
        }
        let rules = try RouteRuleDraft.decode(fixture)
        try require(rules.count == 2 && rules[0].matchMode == .any && rules[1].matchMode == .all, "routing any/all modes lost")
        for rule in rules {
            try require(rule.conditions.count == 2, "routing conditions lost")
            try require(rule.conditions[0].kind == .domainRegex && rule.conditions[0].values == [regex, #"^cdn\.(one|two)\.example$"#], "regex comma/full values lost")
            try require(rule.conditions[1].values == ["443", "8000:8100"], "routing port values lost")
        }
        let encoded = RouteRuleDraft.encode(rules)
        let again = RouteRuleDraft.encode(try RouteRuleDraft.decode(encoded))
        let originalData = try JSONSerialization.data(withJSONObject: fixture, options: [.sortedKeys])
        try require(originalData == JSONSerialization.data(withJSONObject: encoded, options: [.sortedKeys]), "routing encode changed fields")
        try require(originalData == JSONSerialization.data(withJSONObject: again, options: [.sortedKeys]), "routing roundtrip changed fields")
        var malformedRejected = false
        do { _ = try RouteRuleDraft.decode([["conditions": [["kind": "unknown", "values": ["x"]]]]]) }
        catch { malformedRejected = true }
        try require(malformedRejected, "malformed routing schema accepted")
    }
    @MainActor static func require(_ condition: Bool, _ message: String) throws {
        if !condition { throw FrontendSmokeError(message: message) }
    }
    @MainActor static func main() async throws {
        let backendReason = "route import rejected: unsupported source format"
        try require(BridgeFailure.response(backendReason).localizedDescription.contains(backendReason),
                    "localized bridge error hid the backend reason")
        try checkRoutingSerialization()
        try checkProfileSearch()
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("nolurix-frontend-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let options = try Model.encode(["data_dir": directory.path, "core_path": directory.appendingPathComponent("missing-core").path])
        let gate = CloseGate()
        let model = Model(backend: Backend(beforeClose: { try gate.check() }))
        let firstStart = model.start(optionsJSON: options, allowAutoConnect: false)
        let repeatedStart = model.start(optionsJSON: options, allowAutoConnect: false)
        await firstStart.value
        await repeatedStart.value
        try require(model.ready && model.error == nil, "Model failed numeric-handle open: \(model.error ?? "")")
        _ = try await model.perform("import_links", ["gid": 1, "text": "trojan://secret@frontend.example:443#Frontend%20Smoke\ntrojan://broken"])
        try require(model.profiles.count == 1 && model.profiles.first?["name"] as? String == "Frontend Smoke", "Model import/snapshot failed")
        try require(model.operationNotice?.contains("\n") == true, "partial import errors were discarded")
        for language in ["ru", "en"] {
            _ = try await model.perform("settings_patch", ["patch": ["language": language]])
            try require(model.language == language, "Model language save failed")
        }
        var closeFailed = false
        do { try await model.closeBridge() } catch { closeFailed = true }
        try require(closeFailed && model.shutdownPending && model.ready, "failed close lost frontend state")
        let recovered = await withCheckedContinuation { continuation in
            model.recoverBridge { continuation.resume(returning: $0) }
        }
        try require(!recovered && model.shutdownPending, "unconfirmed close pretended to recover")
        try await model.closeBridge() // Same retained handle; never opens another engine.
        try require(!model.ready && !model.shutdownPending, "Model close failed")
        await model.start(optionsJSON: options, allowAutoConnect: false).value
        try require(model.ready && model.language == "en" && model.profiles.count == 1, "Model reopen persistence failed")
        try await model.closeBridge()
        // Exercise malformed JSON through the actual actor serializer/error path.
        let backend = Backend()
        try await backend.open(optionsJSON: options)
        var rejected = false
        do { _ = try await backend.send("{malformed") } catch { rejected = true }
        try require(rejected, "Backend accepted malformed JSON")
        let raw = try await backend.send("{\"op\":\"snapshot\"}")
        try require(try Model.decode(raw) is [String: Any], "Backend snapshot JSON failed")
        try await backend.close()
        try await backend.close() // Idempotent frontend close.
        print("PASS: routing any/all multi-condition roundtrip, regex comma retention, malformed rejection; actual Backend/Model Swift 6 transport, idempotent start, partial import notice, ru/en, failed-close retry, close/reopen; no GUI/core/helper")
    }
}
