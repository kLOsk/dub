import Foundation
import Security

/// Somewhere to keep a user credential (R-42).
///
/// A protocol rather than a bare Keychain call so the migration below
/// can be tested without a signed host and a real keychain prompt.
protocol SecretStore {
    /// The stored secret, or `nil` when there is none (or the store
    /// could not be read).
    func secret(for account: String) -> String?

    /// Store `value`, or remove the entry when it is `nil` or empty.
    /// Returns `false` when the store refused — a locked keychain, a
    /// denied prompt — so callers can decide what to do rather than
    /// silently losing the credential.
    @discardableResult
    func setSecret(_ value: String?, for account: String) -> Bool
}

/// The macOS login keychain, one generic-password item per account.
///
/// **The file-based keychain, deliberately.** The modern
/// data-protection keychain (`kSecUseDataProtectionKeychain`) wants a
/// `keychain-access-groups` entitlement, which needs a real Team ID;
/// Dub is ad-hoc signed with the sandbox off (`project.yml`,
/// `CODE_SIGN_IDENTITY: "-"`), so that call would fail with
/// `errSecMissingEntitlement`. The login keychain works under both that
/// posture and the Developer ID signing that arrives with M20.
///
/// One consequence worth knowing while Dub is ad-hoc signed: keychain
/// ACLs are bound to the code signature, and an ad-hoc signature
/// changes on every rebuild, so macOS may ask permission again after a
/// fresh `make app`. Signing properly settles that; until then a denied
/// prompt reads as "no token", which turns Discogs enrichment off
/// rather than breaking anything.
struct KeychainSecretStore: SecretStore {
    /// Keychain service name — the bundle id, so Dub's items are
    /// distinguishable in Keychain Access.
    let service: String

    init(service: String = "com.klos.dub") {
        self.service = service
    }

    func secret(for account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess,
            let data = item as? Data,
            let value = String(data: data, encoding: .utf8),
            !value.isEmpty
        else {
            return nil
        }
        return value
    }

    @discardableResult
    func setSecret(_ value: String?, for account: String) -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        guard let value, !value.isEmpty else {
            let status = SecItemDelete(query as CFDictionary)
            return status == errSecSuccess || status == errSecItemNotFound
        }
        guard let data = value.data(using: .utf8) else { return false }

        let update: [String: Any] = [kSecValueData as String: data]
        let updated = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if updated == errSecSuccess { return true }
        guard updated == errSecItemNotFound else { return false }

        var insert = query
        insert[kSecValueData as String] = data
        // The token is only ever read while the DJ is using the app, so
        // the strictest accessibility that still survives a reboot is
        // the right one.
        insert[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlocked
        return SecItemAdd(insert as CFDictionary, nil) == errSecSuccess
    }
}

/// One-time move of a credential out of plaintext `UserDefaults`
/// (R-42).
enum SecretMigration {
    /// Resolve the secret, moving a legacy plaintext copy into `store`
    /// on the way. Returns the empty string when there is none.
    ///
    /// The ordering is the load-bearing part: the plaintext copy is
    /// removed **only** once the store has accepted the value.
    /// Migrating into a keychain that refuses the write and then
    /// clearing the default would destroy a credential the user cannot
    /// get back.
    @discardableResult
    static func migrateFromDefaults(
        account: String,
        defaultsKey: String,
        defaults: UserDefaults,
        store: SecretStore
    ) -> String {
        let legacy = defaults.string(forKey: defaultsKey)

        if let stored = store.secret(for: account), !stored.isEmpty {
            if legacy != nil { defaults.removeObject(forKey: defaultsKey) }
            return stored
        }

        guard let legacy, !legacy.isEmpty else {
            // An empty leftover is not a credential; sweep it so the
            // key stops appearing in the preferences plist at all.
            if legacy != nil { defaults.removeObject(forKey: defaultsKey) }
            return ""
        }

        if store.setSecret(legacy, for: account) {
            defaults.removeObject(forKey: defaultsKey)
        }
        return legacy
    }
}
