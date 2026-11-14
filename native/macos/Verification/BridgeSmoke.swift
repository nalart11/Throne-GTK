import Foundation

struct SmokeError: Error, CustomStringConvertible {
    let description: String
}

@main
struct BridgeSmoke {
    static func require(_ condition: Bool, _ message: String) throws {
        if !condition { throw SmokeError(description: message) }
    }

    static func decode(_ pointer: UnsafeMutablePointer<CChar>?) throws -> [String: Any] {
        guard let pointer else { throw SmokeError(description: "null ABI response") }
        defer { throne_macos_string_free(pointer) }
        let data = Data(String(cString: pointer).utf8)
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw SmokeError(description: "response is not an object")
        }
        return object
    }

    static func value(_ response: [String: Any]) throws -> Any {
        try require(response["ok"] as? Bool == true, "ABI failure: \(response)")
        return response["value"] ?? NSNull()
    }

    static func encoded(_ object: [String: Any]) throws -> String {
        String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
    }

    static func request(_ handle: UInt64, _ object: [String: Any]) throws -> Any {
        let text = try encoded(object)
        return try text.withCString { try value(decode(throne_macos_request(handle, $0))) }
    }

    static func snapshot(_ handle: UInt64) throws -> [String: Any] {
        guard let result = try request(handle, ["op": "snapshot"]) as? [String: Any] else {
            throw SmokeError(description: "invalid snapshot")
        }
        return result
    }

    static func main() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("throne-abi-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let options = try encoded(["data_dir": dir.path, "core_path": dir.appendingPathComponent("missing-core").path])
        func open() throws -> UInt64 {
            let result = try options.withCString { try value(decode(throne_macos_open($0))) }
            guard let number = result as? NSNumber else { throw SmokeError(description: "non-numeric handle") }
            return number.uint64Value
        }
        var live: UInt64? = try open()
        defer { if let handle = live { _ = try? decode(throne_macos_close(handle)) } }
        guard let first = live else { throw SmokeError(description: "missing handle") }
        _ = try value(decode(throne_macos_poll(first, 0))) // Consume initial change.
        let imported = try request(first, ["op": "import_links", "gid": 1,
            "text": "trojan://smoke-password@abi.example:443#Swift%20ABI"])
        try require((imported as? [String: Any])?["profiles"] as? Int == 1, "link import count")
        let changed = try value(decode(throne_macos_poll(first, 0))) as? [String: Any]
        try require(changed?["kind"] as? String == "changed", "missing changed event")
        let initial = try snapshot(first)
        let profiles = initial["profiles"] as? [[String: Any]]
        try require(profiles?.count == 1 && profiles?.first?["name"] as? String == "Swift ABI", "profile snapshot")
        for language in ["ru", "en"] {
            _ = try request(first, ["op": "settings_patch", "patch": ["language": language]])
            let settings = try snapshot(first)["settings"] as? [String: Any]
            try require(settings?["language"] as? String == language, "language save: \(language)")
        }
        let malformed = try "{broken".withCString { try decode(throne_macos_request(first, $0)) }
        try require(malformed["ok"] as? Bool == false && malformed["error"] is String, "malformed JSON accepted")
        let invalid = try "{\"op\":\"snapshot\"}".withCString {
            try decode(throne_macos_request(UInt64.max, $0))
        }
        try require(invalid["ok"] as? Bool == false, "invalid handle accepted")
        _ = try value(decode(throne_macos_close(first)))
        live = nil
        let closed = try decode(throne_macos_poll(first, 0))
        try require(closed["ok"] as? Bool == false, "closed handle accepted")
        let second = try open()
        live = second
        try require(second != first, "handle reused")
        let reopened = try snapshot(second)
        try require((reopened["settings"] as? [String: Any])?["language"] as? String == "en", "language not persisted")
        try require((reopened["profiles"] as? [[String: Any]])?.count == 1, "profiles not persisted")
        _ = try value(decode(throne_macos_close(second)))
        live = nil
        print("PASS: Swift/C/Rust ABI import, snapshots, polling, ru/en persistence, errors and ownership; no core or GUI calls")
    }
}
