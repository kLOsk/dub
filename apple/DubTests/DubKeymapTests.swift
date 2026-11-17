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

    // MARK: - The default map (Daniel, 2026-11-17)

    private func fire(
        _ code: UInt16, _ ch: String? = nil, cmd: Bool = false, shift: Bool = false,
        ctrl: Bool = false, opt: Bool = false
    ) -> DubAction? {
        DubKeymap.action(
            forKeyCode: code, character: ch, command: cmd, shift: shift, control: ctrl, option: opt)
    }

    /// The number row splits: 1–5 are deck A's first five cues, 6–0 deck
    /// B's. `⇧` on a cue's key still clears it — the plain key fires,
    /// with ⇧ left over as the argument.
    func testHotCuesSplitAcrossTheNumberRow() {
        let codes: [UInt16] = [18, 19, 20, 21, 23, 22, 26, 28, 25, 29]
        for (i, code) in codes.enumerated() {
            XCTAssertEqual(fire(code), .deckCue(i < 5 ? .a : .b, i % 5), "key \(i + 1)")
        }
        let shifted = DubKeymap.binding(forKeyCode: 18, character: "!", command: false, shift: true)
        XCTAssertEqual(shifted?.action, .deckCue(.a, 0))
        XCTAssertEqual(shifted?.requiresShift, false, "⇧ is the clear argument, not part of the key")
    }

    /// Loads and doubles ride the arrows: ⇧ loads, ⌃⇧ doubles onto that
    /// side's deck; the plain arrows browse tracks and ⌘⇧ the sidebar.
    func testTheArrows() {
        XCTAssertEqual(fire(123, shift: true), .loadInto(.a))
        XCTAssertEqual(fire(124, shift: true), .loadInto(.b))
        XCTAssertEqual(fire(123, shift: true, ctrl: true), .instantDouble(toDeckB: false))
        XCTAssertEqual(fire(124, shift: true, ctrl: true), .instantDouble(toDeckB: true))
        XCTAssertEqual(fire(125), .browse(next: true))
        XCTAssertEqual(fire(126), .browse(next: false))
        XCTAssertEqual(fire(125, cmd: true, shift: true), .sidebar(next: true))
        XCTAssertEqual(fire(126, cmd: true, shift: true), .sidebar(next: false))
        XCTAssertEqual(fire(124, cmd: true, shift: true), .folder(open: true))
        XCTAssertEqual(fire(123, cmd: true, shift: true), .folder(open: false))
        XCTAssertNil(fire(123), "a plain ← is free")
        XCTAssertNil(fire(123, ctrl: true), "⌃ never falls back to the plain key")
        XCTAssertEqual(DubKeymap.legend(for: .instantDouble(toDeckB: false)), "⌃⇧←")
    }

    /// The letters: tap and loop off on the outer row keys, echo in the
    /// middle, Quick Scratch on the home row, the sampler on the bottom.
    func testTheLetters() {
        XCTAssertEqual(fire(12), .tapTempo(.a))
        XCTAssertEqual(fire(35), .tapTempo(.b))
        XCTAssertEqual(fire(13), .loopOff(.a))
        XCTAssertEqual(fire(31), .loopOff(.b))
        XCTAssertEqual(fire(17), .echoOut(.a))
        XCTAssertEqual(fire(16), .echoOut(.b))
        let scratch: [UInt16] = [0, 1, 2, 3, 4, 38, 40, 37]
        for (i, code) in scratch.enumerated() {
            XCTAssertEqual(fire(code), .quickScratch(i < 4 ? .a : .b, i % 4))
        }
        let samples: [UInt16] = [6, 7, 8, 9, 11, 45, 46, 43]
        for (i, code) in samples.enumerated() {
            XCTAssertEqual(fire(code), .sampler(i))
        }
        XCTAssertEqual(fire(3, "f", cmd: true), .focusSearch, "⌘F and F are two keys")
        XCTAssertEqual(fire(43, ",", cmd: true), .openPreferences, "⌘, is not the eighth sample")
        XCTAssertNil(fire(12, "q", cmd: true), "⌘Q must fall through to Quit")
        XCTAssertNil(fire(14), "E is left free")
    }

    /// Space loads into the empty deck: A if both are, the other one if
    /// one is loaded, and the deck that isn't the master if both are.
    func testSpaceLoadsTheEmptyDeck() {
        XCTAssertEqual(fire(49, " "), .loadSelection)
        XCTAssertEqual(WaveformAppModel.spaceTarget(aLoaded: false, bLoaded: false, master: .a), .a)
        XCTAssertEqual(WaveformAppModel.spaceTarget(aLoaded: false, bLoaded: true, master: .b), .a)
        XCTAssertEqual(WaveformAppModel.spaceTarget(aLoaded: true, bLoaded: false, master: .a), .b)
        XCTAssertEqual(WaveformAppModel.spaceTarget(aLoaded: true, bLoaded: true, master: .a), .b)
        XCTAssertEqual(WaveformAppModel.spaceTarget(aLoaded: true, bLoaded: true, master: .b), .a)
    }

    /// A map saved before the defaults existed keeps its keys: a key the
    /// DJ bound beats the default that wants it, and that default goes
    /// unbound rather than both firing.
    func testAKeyTheDJBoundBeatsADefault() {
        let name = "dub.tests.keymap.legacy.\(UUID().uuidString)"
        let legacy = UserDefaults(suiteName: name)!
        legacy.removePersistentDomain(forName: name)
        // `1` on the master deck's cue, as map mode saved it in October.
        legacy.set(Data(#"{"cue.0":{"chord":{"code":18,"command":false,"legend":"1"}}}"#.utf8),
                   forKey: "dub.keymap.v1")
        DubKeymapStore.shared = DubKeymapStore(defaults: legacy)
        XCTAssertEqual(fire(18, "1"), .hotCue(0))
        XCTAssertNil(DubKeymap.legend(for: .deckCue(.a, 0)), "the default lost its key")
        XCTAssertEqual(DubKeymap.legend(for: .deckCue(.a, 1)), "2", "the rest of the map stands")
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
                + "|\(binding.requiresShift)|\(binding.requiresControl)|\(binding.requiresOption)"
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
        XCTAssertEqual(DubAction.echoHold(.b).id, "echo.hold.b")
        XCTAssertEqual(DubAction.fxHold.id, "fx.hold")
        XCTAssertEqual(DubAction.tapTempo(.b).id, "tempo.tap.b")
        XCTAssertEqual(DubAction.loadInto(.a).id, "transport.load.a")
        XCTAssertEqual(DubAction.loopOff(.a).id, "loop.off.a")
        XCTAssertEqual(DubAction.focusSearch.id, "library.search")
        XCTAssertEqual(DubAction.browse(next: false).id, "library.previous")
        XCTAssertEqual(DubAction.deckCue(.b, 4).id, "cue.b.4")
        XCTAssertEqual(DubAction.sidebar(next: true).id, "library.playlist.next")
        XCTAssertEqual(DubAction.folder(open: false).id, "library.folder.close")
    }

    // MARK: - Actions with no button (Preferences ▸ Keys)

    /// Each action Preferences lists binds and fires like a surface
    /// control, and a modifier held with a key is a key of its own once
    /// something is bound to it.
    func testButtonlessActionsBindAndFire() {
        DubKeymapStore.shared.bind(.browse(next: true), to: chord(121, "⇟"))
        XCTAssertEqual(
            DubKeymap.action(forKeyCode: 121, character: nil, command: false), .browse(next: true))
        XCTAssertNil(
            DubKeymap.action(forKeyCode: 125, character: nil, command: false), "↓ moved off")
        DubKeymapStore.shared.bind(.loopOff(.a), to: chord(5, "⇧G", shift: true))
        XCTAssertEqual(fire(5, "G", shift: true), .loopOff(.a), "⇧G is its own key once bound")
        XCTAssertNil(fire(5, "g"), "and plain G stays free")
    }

    /// Tap the BPM absorbed Tap the grid: ⇧ and ⌥ with its key reach it
    /// as arguments (halve / double), and the grid tap's ids are gone.
    func testTheBpmTapTakesHalveAndDouble() {
        let halve = DubKeymap.binding(forKeyCode: 12, character: "Q", command: false, shift: true)
        XCTAssertEqual(halve?.action, .tapTempo(.a))
        XCTAssertEqual(halve?.requiresShift, false, "⇧ is the halve argument")
        let double = DubKeymap.binding(forKeyCode: 35, character: "p", command: false, option: true)
        XCTAssertEqual(double?.action, .tapTempo(.b))
        XCTAssertNil(DubAction(id: "grid.tap.a"))
        XCTAssertFalse(DubKeymap.buttonless.contains { $0.id.hasPrefix("grid.") })
    }

    /// Next / previous track move one row in the list's order, start at
    /// an end when nothing is selected, and stop at the ends.
    func testBrowseTarget() {
        let ids = ["a", "b", "c"]
        XCTAssertEqual(LibraryView.browseTarget(from: nil, in: ids, next: true), "a")
        XCTAssertEqual(LibraryView.browseTarget(from: nil, in: ids, next: false), "c")
        XCTAssertEqual(LibraryView.browseTarget(from: "a", in: ids, next: true), "b")
        XCTAssertEqual(LibraryView.browseTarget(from: "b", in: ids, next: false), "a")
        XCTAssertEqual(LibraryView.browseTarget(from: "c", in: ids, next: true), "c")
        XCTAssertEqual(LibraryView.browseTarget(from: "a", in: ids, next: false), "a")
        XCTAssertEqual(
            LibraryView.browseTarget(from: "gone", in: ids, next: true), "a",
            "a selection no longer listed starts over")
        XCTAssertNil(LibraryView.browseTarget(from: nil, in: [String](), next: true))
    }

    /// Every id round-trips through the parser a stored profile uses,
    /// and an id no version issued is refused rather than guessed.
    func testActionIdsRoundTrip() {
        let all: [DubAction] = [
            .loadSelection, .openPreferences, .hotCue(5), .sirenPreset(0),
            .instantDouble(toDeckB: false), .instantDouble(toDeckB: true), .sampler(3),
            .quickScratch(.a, 0), .quickScratch(.b, 3), .fxToggle(3), .fxKick, .echoOut(.b),
            .echoHold(.a), .echoHold(.b), .fxHold, .fxTap, .keyLock(.a),
            .tapTempo(.a), .tapTempo(.b),
            .loadInto(.a), .loadInto(.b), .loopOff(.a), .loopOff(.b),
            .focusSearch, .browse(next: true), .browse(next: false),
            .sidebar(next: true), .sidebar(next: false), .folder(open: true), .folder(open: false),
            .deckCue(.a, 0), .deckCue(.b, 4),
        ]
        for action in all {
            XCTAssertEqual(DubAction(id: action.id), action, action.id)
        }
        XCTAssertEqual(Set(all.map(\.id)).count, all.count, "no two actions share an id")
        XCTAssertNil(DubAction(id: "siren.preset.x"))
        XCTAssertNil(DubAction(id: "grid.tap"), "the grid tap is gone: Tap the BPM does it")
        XCTAssertNil(DubAction(id: "midi.cc.7"))
        XCTAssertNil(DubAction(id: ""))
    }

    // MARK: - Map mode (M18)

    private func chord(
        _ code: UInt16, _ legend: String, command: Bool = false, shift: Bool = false
    ) -> DubKeyChord {
        DubKeyChord(code: code, command: command, shift: shift, legend: legend)
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
        XCTAssertEqual(DubKeymap.legend(for: .sampler(4)), "B", "back to the default map")
        XCTAssertEqual(DubKeymap.legend(for: .quickScratch(.a, 0)), "A")
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
