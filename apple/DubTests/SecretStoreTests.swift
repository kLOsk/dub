import XCTest

@testable import Dub

/// R-42 — the Discogs token moved out of `UserDefaults` into the
/// Keychain. The Keychain call itself is one `SecItem` round trip and
/// needs a signed host to exercise; the part worth testing is the
/// *migration*, because that is where a user's credential can be lost:
/// clear the plaintext copy before the write lands and the token is
/// gone with nothing to recover it from.
final class SecretStoreTests: XCTestCase {

    /// In-memory stand-in for the Keychain, with a switch to make
    /// writes fail the way a locked or access-denied keychain does.
    private final class FakeSecretStore: SecretStore {
        var values: [String: String] = [:]
        var writesSucceed = true
        private(set) var writeCount = 0

        func secret(for account: String) -> String? { values[account] }

        @discardableResult
        func setSecret(_ value: String?, for account: String) -> Bool {
            writeCount += 1
            guard writesSucceed else { return false }
            if let value, !value.isEmpty {
                values[account] = value
            } else {
                values.removeValue(forKey: account)
            }
            return true
        }
    }

    private let account = "discogsToken"
    private let defaultsKey = "dub.discogsToken"
    private var defaults: UserDefaults!
    private var suiteName: String!

    override func setUp() {
        super.setUp()
        suiteName = "dub.tests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        defaults = nil
        super.tearDown()
    }

    private func migrate(_ store: SecretStore) -> String {
        SecretMigration.migrateFromDefaults(
            account: account,
            defaultsKey: defaultsKey,
            defaults: defaults,
            store: store)
    }

    func testMovesAPlaintextTokenIntoTheStore() {
        defaults.set("tok-abc", forKey: defaultsKey)
        let store = FakeSecretStore()

        XCTAssertEqual(migrate(store), "tok-abc")
        XCTAssertEqual(store.secret(for: account), "tok-abc")
        XCTAssertNil(
            defaults.string(forKey: defaultsKey),
            "the plaintext copy is the whole point of R-42 — it must not survive")
    }

    /// The failure that would cost a user their token: if the Keychain
    /// write does not land, the plaintext copy is all there is, so it
    /// stays. Plain text is a weaker outcome than the Keychain; losing
    /// the credential outright is a worse one.
    func testKeepsThePlaintextCopyWhenTheWriteFails() {
        defaults.set("tok-abc", forKey: defaultsKey)
        let store = FakeSecretStore()
        store.writesSucceed = false

        XCTAssertEqual(migrate(store), "tok-abc")
        XCTAssertNil(store.secret(for: account))
        XCTAssertEqual(defaults.string(forKey: defaultsKey), "tok-abc")
    }

    func testAStoredSecretWinsOverAStaleDefault() {
        defaults.set("tok-old", forKey: defaultsKey)
        let store = FakeSecretStore()
        store.values[account] = "tok-new"

        XCTAssertEqual(migrate(store), "tok-new")
        XCTAssertNil(
            defaults.string(forKey: defaultsKey),
            "a leftover plaintext copy is swept even when the store already answers")
        XCTAssertEqual(store.writeCount, 0, "nothing to write — the store already had it")
    }

    func testNothingStoredAnywhereResolvesToEmpty() {
        let store = FakeSecretStore()

        XCTAssertEqual(migrate(store), "")
        XCTAssertEqual(store.writeCount, 0)
        XCTAssertNil(defaults.string(forKey: defaultsKey))
    }

    /// An empty string in `UserDefaults` is what a user who typed a
    /// token and then cleared it leaves behind. It is not a credential,
    /// so it is swept without a write.
    func testAnEmptyDefaultIsSweptRatherThanStored() {
        defaults.set("", forKey: defaultsKey)
        let store = FakeSecretStore()

        XCTAssertEqual(migrate(store), "")
        XCTAssertEqual(store.writeCount, 0)
        XCTAssertNil(defaults.string(forKey: defaultsKey))
    }

    func testClearingTheTokenRemovesItFromTheStore() {
        let store = FakeSecretStore()
        store.setSecret("tok-abc", for: account)

        store.setSecret("", for: account)
        XCTAssertNil(store.secret(for: account))

        store.setSecret("tok-abc", for: account)
        store.setSecret(nil, for: account)
        XCTAssertNil(store.secret(for: account))
    }
}
