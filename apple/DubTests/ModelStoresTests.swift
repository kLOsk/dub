import Combine
import XCTest

@testable import Dub

/// The model split (`ModelStores.swift`): a deck's state lives on its own
/// store so a change rebuilds only the views that show decks.
final class ModelStoresTests: XCTestCase {

    /// A deck change reaches the views observing the store.
    func testADeckStorePublishesItsChanges() {
        let store = DeckStore(label: "test")
        var fired = 0
        let sub = store.objectWillChange.sink { fired += 1 }
        var s = store.state
        s.isPlaying = true
        store.state = s
        XCTAssertEqual(fired, 1)
        sub.cancel()
    }

    /// The guard's report names what moved — the whole point of it: the
    /// churn it exists to catch (a live pitch, an input level) was
    /// invisible until a profile found it.
    func testThePublishGuardNamesTheFieldsThatMoved() {
        var a = DeckState.empty
        var b = a
        b.isPlaying = true
        b.pitchSettled = false
        XCTAssertEqual(
            Set(PublishRateGuard<DeckState>.changedFields(a, b)), ["isPlaying", "pitchSettled"])
        a = b
        XCTAssertEqual(PublishRateGuard<DeckState>.changedFields(a, b), [])
    }
}
