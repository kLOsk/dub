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

    /// Every test runs against an empty, throwaway map — the suite is
    /// hosted in the app, where the shared store reads the DJ's real one.
    private var savedStore: DubKeymapStore?
    private var suite: UserDefaults?

    override func setUp() {
        super.setUp()
        savedStore = DubKeymapStore.shared
        let name = "dub.tests.keymap.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)
        defaults?.removePersistentDomain(forName: name)
        suite = defaults
        DubKeymapStore.shared = DubKeymapStore(defaults: defaults ?? .standard)
    }

    override func tearDown() {
        if let saved = savedStore { DubKeymapStore.shared = saved }
        super.tearDown()
    }

    // MARK: - Nothing you play has a default key

    /// The number row used to fire the hot cues. It falls through now —
    /// a DJ maps their own keys (2026-09-15) — and `⇧` on a mapped cue
    /// still clears rather than selecting a different action.
    func testHotCuesHaveNoDefaultKey() {
        // 1-8. Not contiguous past 4 on macOS: 5 and 6 are 23 and 22.
        let codes: [UInt16] = [18, 19, 20, 21, 23, 22, 26, 28]
        for (i, code) in codes.enumerated() {
            XCTAssertNil(
                DubKeymap.action(forKeyCode: code, character: "\(i + 1)", command: false),
                "cue \(i + 1) must not be pre-assigned")
        }
        DubKeymapStore.shared.bind(.hotCue(0), to: DubKeyChord(code: 18, legend: "1"))
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 18, character: "1", command: false), .hotCue(0))
    }

    /// Only the two app conventions ship bound.
    func testOnlySpaceAndPreferencesAreDefaults() {
        XCTAssertEqual(
            Set(DubKeymap.defaults.map(\.action)), [.loadSelection, .openPreferences])
    }

    /// The siren's bottom row `Z X C V B N M ,` is unbound until map
    /// mode: every one of those keys must fall through, and no binding
    /// may claim the action.
    func testSirenKeysAreNotBoundAndFallThrough() {
        for code: UInt16 in [6, 7, 8, 9, 11, 45, 46, 43] {
            XCTAssertNil(
                DubKeymap.action(forKeyCode: code, character: nil, command: false),
                "an unbound key must not be consumed")
        }
        XCTAssertFalse(
            DubKeymap.bindings.contains { $0.action.id.hasPrefix("siren.") },
            "the siren's keys come back with map mode, not before")
    }

    /// `Q W E R` used to fire Quick Scratch. Its pads are per deck now
    /// (PRD §7.2), which the old four keys could not address, so they
    /// wait for map mode too — and `⌘Q` has to stay Quit regardless.
    func testQuickScratchKeysAreNotBoundAndFallThrough() {
        for code: UInt16 in [12, 13, 14, 15] {
            XCTAssertNil(
                DubKeymap.action(forKeyCode: code, character: nil, command: false),
                "an unbound key must not be consumed")
        }
        XCTAssertNil(
            DubKeymap.action(forKeyCode: 12, character: "q", command: true),
            "⌘Q must fall through to Quit")
        XCTAssertFalse(
            DubKeymap.bindings.contains { $0.action.id.hasPrefix("quickScratch.") })
    }

    func testSpaceLoadsTheSelection() {
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 49, character: " ", command: false),
            .loadSelection)
    }

    /// `⌘→` / `⌘←` used to be instant doubles and `G` the grid tap. Both
    /// wait for the DJ now, and the arrows and the letter fall through.
    func testInstantDoublesAndGridTapHaveNoDefaultKey() {
        XCTAssertNil(DubKeymap.action(forKeyCode: 124, character: nil, command: true))
        XCTAssertNil(DubKeymap.action(forKeyCode: 123, character: nil, command: true))
        XCTAssertNil(DubKeymap.action(forKeyCode: 5, character: "g", command: false))
        DubKeymapStore.shared.bind(.instantDouble(toDeckB: true), to: DubKeyChord(code: 124, command: true, legend: "⌘→"))
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 124, character: nil, command: true),
            .instantDouble(toDeckB: true))
        XCTAssertNil(
            DubKeymap.action(forKeyCode: 124, character: nil, command: false),
            "a ⌘ chord does not fire on the bare key")
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

    /// A rendered cap and the key that fires it are the same fact — and
    /// with nothing bound, the siren's caps print nothing.
    func testEveryRenderedLegendComesFromTheTable() {
        XCTAssertEqual(sirenPresetKeys, ["", "", "", "", ""])
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
        XCTAssertEqual(DubAction.sirenPreset(4).id, "siren.preset.4")
        XCTAssertEqual(DubAction.instantDouble(toDeckB: true).id, "deck.instantDouble.b")
        XCTAssertEqual(DubAction.loadSelection.id, "transport.loadSelection")
        XCTAssertEqual(DubAction.sampler(7).id, "sampler.7")
        XCTAssertEqual(DubAction.quickScratch(.b, 3).id, "scratch.b.3")
        XCTAssertEqual(DubAction.fxToggle(2).id, "fx.unit.2")
        XCTAssertEqual(DubAction.fxKick.id, "fx.kick")
        XCTAssertEqual(DubAction.echoOut(.a).id, "echo.a")
    }

    /// Every id round-trips through the parser a stored profile uses,
    /// and an id no version issued is refused rather than guessed.
    func testActionIdsRoundTrip() {
        let all: [DubAction] = [
            .loadSelection, .openPreferences, .tapGrid, .hotCue(5), .sirenPreset(0),
            .instantDouble(toDeckB: false), .instantDouble(toDeckB: true), .sampler(3),
            .quickScratch(.a, 0), .quickScratch(.b, 3), .fxToggle(3), .fxKick, .echoOut(.b),
        ]
        for action in all {
            XCTAssertEqual(DubAction(id: action.id), action, action.id)
        }
        XCTAssertNil(DubAction(id: "siren.preset.x"))
        XCTAssertNil(DubAction(id: "midi.cc.7"))
        XCTAssertNil(DubAction(id: ""))
    }

    // MARK: - Map mode (M18)

    private func chord(_ code: UInt16, _ legend: String, command: Bool = false) -> DubKeyChord {
        DubKeyChord(code: code, command: command, legend: legend)
    }

    /// Turn MAP on, click the siren's LASER key, press Z: Z fires LASER
    /// and the key prints Z. Nothing else moved.
    func testBindingASirenShotMakesItsKeyFireAndPrint() {
        DubKeymapStore.shared.bind(.sirenPreset(3), to: chord(6, "Z"))
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 6, character: "z", command: false), .sirenPreset(3))
        XCTAssertEqual(DubKeymap.legend(for: .sirenPreset(3)), "Z")
        XCTAssertEqual(sirenPresetKeys, ["", "", "", "Z", ""])
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 49, character: " ", command: false), .loadSelection,
            "the defaults are still there")
    }

    /// One key, one action: binding a sampler slot to `1` takes `1` from
    /// the hot cue that had it, which is then unbound — its cap prints
    /// nothing rather than a key that fires something else. Space, a
    /// default, is stolen the same way.
    func testBindingStealsTheKeyFromItsPreviousAction() {
        DubKeymapStore.shared.bind(.hotCue(0), to: chord(18, "1"))
        DubKeymapStore.shared.bind(.sampler(0), to: chord(18, "1"))
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 18, character: "1", command: false), .sampler(0))
        XCTAssertNil(DubKeymap.legend(for: .hotCue(0)))
        XCTAssertTrue(DubKeymapStore.shared.isOverridden(.hotCue(0)))
        // Rebinding the cue elsewhere gives it a key back; the slot keeps 1.
        DubKeymapStore.shared.bind(.hotCue(0), to: chord(12, "Q"))
        XCTAssertEqual(DubKeymap.legend(for: .hotCue(0)), "Q")
        XCTAssertEqual(DubKeymap.legend(for: .sampler(0)), "1")

        DubKeymapStore.shared.bind(.echoOut(.a), to: chord(49, "SPACE"))
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 49, character: " ", command: false), .echoOut(.a))
        XCTAssertNil(DubKeymap.legend(for: .loadSelection), "Space moved off the loader")
    }

    /// `⌘,` is the way back to everything: it cannot be taken or rebound.
    func testPreferencesCannotBeRebound() {
        DubKeymapStore.shared.bind(.openPreferences, to: chord(6, "Z"))
        XCTAssertNil(DubKeymap.legend(for: .sirenPreset(0)))
        XCTAssertEqual(DubKeymap.legend(for: .openPreferences), "⌘,")
        DubKeymapStore.shared.bind(.sampler(1), to: chord(43, "⌘,", command: true))
        XCTAssertEqual(DubKeymap.legend(for: .openPreferences), "⌘,", "not stolen either")
    }

    /// ⌫ on an armed control leaves it unbound; reset brings every
    /// default back and forgets every override.
    func testClearAndReset() {
        DubKeymapStore.shared.clear(.loadSelection)
        XCTAssertNil(DubKeymap.legend(for: .loadSelection))
        XCTAssertNil(DubKeymap.action(forKeyCode: 49, character: " ", command: false))
        DubKeymapStore.shared.bind(.sampler(4), to: chord(0, "A"))
        DubKeymapStore.shared.reset()
        XCTAssertEqual(DubKeymap.legend(for: .loadSelection), "SPACE")
        XCTAssertNil(DubKeymap.legend(for: .sampler(4)))
        XCTAssertFalse(DubKeymapStore.shared.isOverridden(.loadSelection))
    }

    /// The map survives a relaunch: a second store over the same suite
    /// resolves the same table.
    func testOverridesPersist() {
        DubKeymapStore.shared.bind(.quickScratch(.b, 2), to: chord(14, "E"))
        DubKeymapStore.shared.clear(.hotCue(7))
        let reopened = DubKeymapStore(defaults: suite ?? .standard)
        XCTAssertEqual(reopened.resolvedByAction[.quickScratch(.b, 2)]?.legend, "E")
        XCTAssertNil(reopened.resolvedByAction[.hotCue(7)])
        XCTAssertEqual(reopened.resolved.count, DubKeymapStore.shared.resolved.count)
    }

    /// The cap prints what the DJ's keyboard says: letters upper-case,
    /// the unprintable keys by their macOS symbols, a ⌘ chord with its glyph.
    func testChordLegends() {
        XCTAssertEqual(DubKeyChord.legend(code: 6, characters: "z", command: false), "Z")
        XCTAssertEqual(DubKeyChord.legend(code: 49, characters: " ", command: false), "SPACE")
        XCTAssertEqual(DubKeyChord.legend(code: 124, characters: "\u{F703}", command: false), "→")
        XCTAssertEqual(DubKeyChord.legend(code: 122, characters: "\u{F704}", command: false), "F1")
        XCTAssertEqual(DubKeyChord.legend(code: 12, characters: "q", command: true), "⌘Q")
        XCTAssertEqual(DubKeyChord.legend(code: 999, characters: nil, command: false), "#999")
    }

    /// Every binding ships on the key transport today. MIDI and HID exist
    /// in the model so profiles do not need a migration later; if one
    /// appears here before its lane is dispatched, it would be a binding
    /// that renders and never fires.
    func testEverythingShippingIsOnTheKeyTransport() {
        XCTAssertTrue(DubKeymap.bindings.allSatisfy { $0.transport == .key })
    }
}
