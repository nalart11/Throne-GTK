import Foundation

private func response(_ pointer: UnsafeMutablePointer<CChar>?) throws -> Any {
    guard let pointer else { throw NSError(domain: "GuiFixture", code: 1) }
    defer { throne_macos_string_free(pointer) }
    let data = Data(String(cString: pointer).utf8)
    let root = try JSONSerialization.jsonObject(with: data) as! [String: Any]
    guard root["ok"] as? Bool == true else {
        throw NSError(domain: "GuiFixture", code: 2, userInfo: [NSLocalizedDescriptionKey: root["error"] ?? "bridge error"])
    }
    return root["value"] ?? NSNull()
}

private func seedGuiFixture(dataDirectory: String) throws {
    let options: [String: Any] = ["data_dir": dataDirectory, "core_path": "/nonexistent-throne-core"]
    let optionsData = try JSONSerialization.data(withJSONObject: options)
    let optionsText = String(decoding: optionsData, as: UTF8.self)
    let rawHandle = try optionsText.withCString { try response(throne_macos_open($0)) }
    guard let handle = (rawHandle as? NSNumber)?.uint64Value else { throw NSError(domain: "GuiFixture", code: 3) }
    defer { _ = try? response(throne_macos_close(handle)) }

    func send(_ fields: [String: Any]) throws -> Any {
        let data = try JSONSerialization.data(withJSONObject: fields)
        let text = String(decoding: data, as: UTF8.self)
        return try text.withCString { try response(throne_macos_request(handle, $0)) }
    }

    _ = try send(["op": "settings_patch", "patch": ["language": "ru", "connect_last_on_start": false]])
    let groupValue = try send(["op": "group_save", "group": ["id": 0, "name": "Fixture Group", "url": "", "info": "", "archive": false, "skip_auto_update": false, "sub_last_update": 0, "created_at": 0, "updated_at": 0]])
    guard let groupID = (groupValue as? [String: Any])?["id"] else { throw NSError(domain: "GuiFixture", code: 4) }
    _ = try send(["op": "profile_save", "profile": ["id": 0, "gid": groupID, "name": "Example profile", "kind": "shadowsocks", "outbound": ["type": "shadowsocks", "server": "fixture.invalid", "server_port": 443, "method": "none", "password": "fixture"], "raw": "", "created_at": 0, "updated_at": 0]])
    _ = try send(["op": "settings_patch", "patch": ["route_rules": [["name": "Fixture domain", "enabled": true, "conditions": [["kind": "domain_suffix", "values": ["example.invalid"]]], "match_mode": "any", "action": "proxy"]]]])
}

@main
struct GuiFixture {
    static func main() throws {
        guard CommandLine.arguments.count == 2 else {
            throw NSError(domain: "GuiFixture", code: 5, userInfo: [NSLocalizedDescriptionKey: "Usage: seed-fixture DATA_DIR"])
        }
        try seedGuiFixture(dataDirectory: CommandLine.arguments[1])
        print("Seeded isolated GUI fixture at \(CommandLine.arguments[1])")
    }
}
