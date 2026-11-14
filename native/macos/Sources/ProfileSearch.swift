import Foundation

enum ProfileSearch {
    static func reconciledGroupID(currentGroupID: Int, groups: [[String: Any]], profiles: [[String: Any]], selectedID: Int) -> Int {
        let groupIDs = groups.compactMap { $0["id"] as? Int }
        if groupIDs.contains(currentGroupID) { return currentGroupID }
        return initialGroupID(groups: groups, profiles: profiles, selectedID: selectedID)
    }

    static func initialGroupID(groups: [[String: Any]], profiles: [[String: Any]], selectedID: Int) -> Int {
        let groupIDs = groups.compactMap { $0["id"] as? Int }
        if selectedID < 0, selectedID != Int.min, groupIDs.contains(-selectedID) {
            return -selectedID
        }
        if selectedID > 0,
           let profile = profiles.first(where: { ($0["id"] as? Int) == selectedID }),
           let groupID = profile["gid"] as? Int, groupIDs.contains(groupID) {
            return groupID
        }
        return groupIDs.first(where: { groupID in profiles.contains { ($0["gid"] as? Int) == groupID } })
            ?? groupIDs.first ?? 0
    }

    static func matches(_ profile: [String: Any], groupID: Int, query: String) -> Bool {
        guard (profile["gid"] as? Int ?? 0) == groupID else { return false }
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return true }
        let outbound = profile["outbound"] as? [String: Any] ?? [:]
        let address = profile["address"] as? String
            ?? "\(outbound["server"] as? String ?? outbound["protocol"] as? String ?? "") \(outbound["server_port"] as? Int ?? 0)"
        return ((profile["name"] as? String ?? "") + " " + address)
            .localizedCaseInsensitiveContains(query)
    }
}
