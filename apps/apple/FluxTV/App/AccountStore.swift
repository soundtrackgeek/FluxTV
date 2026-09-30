import Foundation
import Security

/// An IPTV account as saved in the Keychain. The whole record (including the password) is one
/// Keychain item, so accounts survive reinstalls and, with iCloud Keychain, appear on the
/// user's other devices (add an account on iPhone, use it on Apple TV).
struct StoredAccount: Codable, Identifiable, Hashable, Sendable {
    var key: String
    var name: String
    var kind: String
    var url: String
    var username: String?
    var password: String?
    var epgUrl: String?
    var createdAt: Date

    var id: String { key }

    var sourceKind: SourceKind { kind == "m3u" ? .m3u : .xtream }

    init(validated v: ValidatedAccount, key: String = UUID().uuidString, createdAt: Date = .now) {
        self.key = key
        name = v.name
        kind = v.kind == .m3u ? "m3u" : "xtream"
        url = v.url
        username = v.username
        password = v.password
        epgUrl = v.epgUrl
        self.createdAt = createdAt
    }

    var saved: SavedAccount {
        SavedAccount(
            key: key, name: name, kind: sourceKind, url: url, username: username, password: password, epgUrl: epgUrl)
    }
}

enum KeychainError: LocalizedError {
    case status(OSStatus)

    var errorDescription: String? {
        switch self {
        case .status(let s):
            let text = SecCopyErrorMessageString(s, nil) as String? ?? "error \(s)"
            return "Keychain: \(text)"
        }
    }
}

/// Keychain persistence for accounts (generic passwords, one item per account).
enum AccountStore {
    static let service = "com.soundtrackgeek.fluxtv.account"

    private static func baseQuery() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrSynchronizable as String: kSecAttrSynchronizableAny,
        ]
    }

    static func loadAll() throws -> [StoredAccount] {
        var query = baseQuery()
        query[kSecMatchLimit as String] = kSecMatchLimitAll
        query[kSecReturnData as String] = true
        query[kSecReturnAttributes as String] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess else { throw KeychainError.status(status) }
        let items = result as? [[String: Any]] ?? []
        let decoder = JSONDecoder()
        var seen = Set<String>()
        return items
            .compactMap { $0[kSecValueData as String] as? Data }
            .compactMap { try? decoder.decode(StoredAccount.self, from: $0) }
            .filter { seen.insert($0.key).inserted }  // a synced and a local copy of the same account
            .sorted { $0.createdAt < $1.createdAt }
    }

    static func save(_ account: StoredAccount) throws {
        let data = try JSONEncoder().encode(account)
        var match = baseQuery()
        match[kSecAttrAccount as String] = account.key
        let attributes: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrLabel as String: "FluxTV – \(account.name)",
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlock,
        ]
        let status = SecItemUpdate(match as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            var add = attributes
            add[kSecClass as String] = kSecClassGenericPassword
            add[kSecAttrService as String] = service
            add[kSecAttrAccount as String] = account.key
            add[kSecAttrSynchronizable as String] = true
            let added = SecItemAdd(add as CFDictionary, nil)
            guard added == errSecSuccess else { throw KeychainError.status(added) }
        } else if status != errSecSuccess {
            throw KeychainError.status(status)
        }
    }

    static func delete(key: String) throws {
        var query = baseQuery()
        query[kSecAttrAccount as String] = key
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw KeychainError.status(status) }
    }
}
