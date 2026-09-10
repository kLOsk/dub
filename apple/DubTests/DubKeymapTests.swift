import XCTest

@testable import Dub

/// The binding table.
///
/// Two jobs here. The first is a **regression fence**: this table replaced
/// three hand-written `[UInt16: Int]` dictionaries inside the key
/// monitor's event closure, and every keyCode in it has to still mean what
/// it meant before, or a DJ's muscle memory silently changes. Those are
/// asserted as literals on purpose — restating them is the point.
///
/// The second is the invariant the table exists for: a rendered cap and
/// the key that fires it come from one place, so they cannot disagree.
/// `SirenRackGroup`'s doc comment records the app shipping exactly that
/// bug — pads advertising keys that did nothing.
final class DubKeymapTests: XCTestCase {

    // MARK: - The bindings that existed before, unchanged

    /// Number row, layout-independent. `⇧` clears rather than selecting a
    /// different action, so it must not change the match.
    func testHotCuesKeepTheNumberRow() {
        // 1-8. Not contiguous past 4 on macOS: 5 and 6 are 23 and 22.
        let codes: [UInt16] = [18, 19, 20, 21, 23, 22, 26, 28]
        for (i, code) in codes.enumerated() {
            XCTAssertEqual(
                DubKeymap.action(forKeyCode: code, character: "\(i + 1)", command: false),
                .hotCue(i),
                "cue \(i + 1)")
        }
    }

    /// `Z X C V B N M ,` — the bottom letter row, in fire order.
    func testSirenPresetsKeepTheBottomRow() {
        let codes: [UInt16] = [6, 7, 8, 9, 11, 45, 46, 43]
        for (index, code) in codes.enumerated() {
            XCTAssertEqual(
                DubKeymap.action(forKeyCode: code, character: nil, command: false),
                .sirenPreset(index))
        }
    }

    /// `Q W E R`, unmodified only — `⌘Q` has to stay Quit.
    func testQuickScratchKeepsQWER() {
        let codes: [UInt16] = [12, 13, 14, 15]
        for (slot, code) in codes.enumerated() {
            XCTAssertEqual(
                DubKeymap.action(forKeyCode: code, character: nil, command: false),
                .quickScratch(slot))
        }
        XCTAssertNil(
            DubKeymap.action(forKeyCode: 12, character: "q", command: true),
            "⌘Q must fall through to Quit")
    }

    func testSpaceLoadsTheSelection() {
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 49, character: " ", command: false),
            .loadSelection)
    }

    /// `⌘→` duplicates onto B, `⌘←` the reverse. Unmodified arrows are not
    /// ours — they move a caret and a table selection.
    func testInstantDoublesNeedCommand() {
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 124, character: nil, command: true),
            .instantDouble(toDeckB: true))
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 123, character: nil, command: true),
            .instantDouble(toDeckB: false))
        XCTAssertNil(DubKeymap.action(forKeyCode: 124, character: nil, command: false))
    }

    /// The grid tap is matched by printed character, not position — the
    /// mnemonic is the `G` of "grid", so a non-QWERTY layout should follow
    /// the letter rather than the key.
    func testGridTapMatchesTheLetterNotThePosition() {
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 999, character: "g", command: false), .tapGrid)
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 999, character: "G", command: false), .tapGrid)
    }

    // MARK: - The sampler has no bindings

    /// `A S D F` used to be reserved for the sampler. They were dropped
    /// rather than extended to eight slots — the keys wait for map mode —
    /// so the letters must fall through to the rest of the app, and no
    /// binding may claim them.
    func testSamplerKeysAreNotBoundAndFallThrough() {
        for code in UInt16(0)...UInt16(3) {
            XCTAssertNil(
                DubKeymap.action(forKeyCode: code, character: nil, command: false),
                "an unbound key must not be consumed")
        }
        XCTAssertFalse(
            DubKeymap.bindings.contains { $0.action.id.hasPrefix("sampler.") },
            "the sampler's keys come back with map mode, not before")
    }

    // MARK: - The invariant the table exists for

    /// A rendered cap and the key that fires it are the same fact.
    func testEveryRenderedLegendComesFromTheTable() {
        XCTAssertEqual(sirenPresetKeys, ["Z", "X", "C", "V", "B", "N", "M", ","])
        XCTAssertEqual(QuickScratchSlots.keyLabels, ["Q", "W", "E", "R"])
    }

    /// No two live bindings can claim the same key. This is the check that
    /// makes a whole bug class unwritable rather than merely unlikely —
    /// adding a colliding binding fails here instead of silently shadowing
    /// whichever one the dispatcher happened to reach first.
    func testNoTwoLiveBindingsCollide() {
        var seen: Set<String> = []
        for binding in DubKeymap.bindings where binding.isLive {
            let key = "\(binding.transport.rawValue)|\(binding.match)|\(binding.requiresCommand)"
            XCTAssertTrue(
                seen.insert(key).inserted,
                "two live bindings claim \(key) — one of them will never fire")
        }
    }

    /// Action ids are what a saved profile will persist, so a rename is a
    /// silent rebind for anyone who already had one.
    func testActionIdsAreStable() {
        XCTAssertEqual(DubAction.hotCue(2).id, "cue.2")
        XCTAssertEqual(DubAction.sirenPreset(7).id, "siren.preset.7")
        XCTAssertEqual(DubAction.quickScratch(0).id, "quickScratch.0")
        XCTAssertEqual(DubAction.instantDouble(toDeckB: true).id, "deck.instantDouble.b")
        XCTAssertEqual(DubAction.loadSelection.id, "transport.loadSelection")
    }

    /// Every binding ships on the key transport today. MIDI and HID exist
    /// in the model so profiles do not need a migration later; if one
    /// appears here before its lane is dispatched, it would be a binding
    /// that renders and never fires.
    func testEverythingShippingIsOnTheKeyTransport() {
        XCTAssertTrue(DubKeymap.bindings.allSatisfy { $0.transport == .key })
    }
}
