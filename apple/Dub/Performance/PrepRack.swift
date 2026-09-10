//
//  PrepRack.swift
//  Dub
//
//  Prep's control surface.
//
//  ## These are three different things, and they are drawn as three
//  different things
//
//  The first attempt at this file gave CUE, LOOP and SAMPLES a shared
//  module wrapper — same border, same header row, same padding — which is
//  the flat-hierarchy mistake one level up from where it started: the
//  contents differed and the containers made them look alike again. There
//  is no `RackModule` here on purpose. Each section owns its own chrome,
//  its own placement and its own idea of what an item is:
//
//  * **CUE** is a *list of marks*. Unboxed — the rows are the objects, and
//    a frame around them would just be a second border inside a pad bar
//    inside a deck pane. Full width of its column, four rows deep.
//  * **LOOP** is an *instrument*. Boxed, recessed, panel-like, built
//    around a numeral in a well. It is the only thing here that looks like
//    a unit, because it is the only thing here that behaves like one.
//  * **SAMPLES** is a *drop target over a pad bank*. Its border is dashed
//    when empty because that is what a drop zone looks like, and solid
//    once it holds something. Neither of the other two has a state where
//    its own outline changes. It is `SampleShelf`, the same view
//    Performance fires from — Prep is where it gets loaded and tried.
//
//  Cover the labels: a list, a panel, and a drop zone are still three
//  recognisable objects. That is the test this file has to keep passing.
//
//  ## Why only these three
//
//  Prep is couch work with no rig attached. The FX are not here — echo
//  out, the siren and its Expert panel, Quick Scratch and the vintage rack
//  are things you do to a record playing in front of people. Prep owns
//  preparing marks and getting sounds into the bank.
//
//  There is **no beatgrid editor**, and there is not going to be one.
//  Setting the 1 — the deck-header BPM tap, which re-anchors the grid to
//  the visible kick — plus Analyze covers what a DJ actually does to a
//  grid. The six FFI calls behind a full editor stay unmounted rather than
//  becoming a surface nobody asked for. Prep is these three sections.
//
//  ## Values in, closures out
//
//  No section takes the model. That is what makes the surface
//  snapshot-testable, which `PrepPadSnapshotTests` could not be until
//  `SirenExpertPanel` stopped holding `WaveformAppModel`.
//

import AppKit
import SwiftUI

// MARK: - State

/// Everything Prep's surface draws.
struct PrepRackState: Equatable {
    var cues: [CueSlotState] = (0..<8).map { CueSlotState(index: $0) }
    /// Beats of the loop currently running; `nil` when none is.
    var activeLoopBeats: Double?
    var loopEngaged: Bool = false
    /// The sampler. Fixed slots rather than a growing list, because a
    /// slot is what a pad is.
    var samples: SampleShelfState = .empty
    /// Cue and loop controls do nothing without a deck loaded, and should
    /// say so rather than fail quietly.
    var hasTrack: Bool = false
    /// A paused deck previews a cue while the pad is held; a running one
    /// jumps and keeps going.
    var isPlaying: Bool = false
}

/// Everything it does.
struct PrepRackCallbacks {
    var onCue: (_ index: Int, _ clear: Bool) -> Void = { _, _ in }
    /// Mouse-down on a set cue while the deck is paused: play from it.
    var onPreviewDown: (_ index: Int) -> Void = { _ in }
    /// Mouse-up: stop and return to the mark.
    var onPreviewUp: () -> Void = {}
    var onRenameCue: (_ index: Int) -> Void = { _ in }
    var onColorCue: (_ index: Int, _ token: String?) -> Void = { _, _ in }
    var onLoop: (_ beats: Double) -> Void = { _ in }
    /// Halve or double the running loop, keeping its start.
    var onScaleLoop: (_ double: Bool) -> Void = { _ in }
    /// Pressing the lit length again leaves the loop.
    var onExitLoop: () -> Void = {}
    /// The sampler's gestures — fire, stop, drop, unload.
    var samples = SampleShelfCallbacks()
}

// MARK: - The surface

struct PrepRack: View {
    let state: PrepRackState
    var callbacks = PrepRackCallbacks()

    var body: some View {
        HStack(alignment: .top, spacing: DubSpacing.xl) {
            CueRowBank(
                slots: state.cues,
                hasTrack: state.hasTrack,
                isPlaying: state.isPlaying,
                onCue: callbacks.onCue,
                onPreviewDown: callbacks.onPreviewDown,
                onPreviewUp: callbacks.onPreviewUp,
                onRename: callbacks.onRenameCue,
                onColor: callbacks.onColorCue,
                columns: 4,
                contentHeight: DubLayout.prepSectionContent)
                .frame(width: DubLayout.prepCueColumn, alignment: .leading)


            // Takes the rest. There is no fourth section coming, so
            // reserving trailing space would just be the dead area this
            // redesign exists to remove — and the shelf is the one thing
            // here that genuinely improves with width, because a filename
            // is the only string on the surface whose length is not ours
            // to choose.
            SampleShelf(state: state.samples, callbacks: callbacks.samples)
                .frame(
                    minWidth: DubLayout.prepSampleShelfMin,
                    maxWidth: .infinity,
                    alignment: .leading)
        }
    }
}

/// `m:ss.t` — tenths, because a cue set by ear is placed to about that
/// precision and more digits would read as false confidence.
enum CueTimecode {
    static func format(_ secs: Double) -> String {
        guard secs.isFinite, secs >= 0 else { return "—" }
        // Round to tenths *first*, then split. Truncating the remainder
        // renders 102.3 as `1:42.2`, because 102.3 - 102 is 0.2999… in
        // binary — an off-by-a-tenth on every cue whose position lands
        // near a boundary, which is most of them.
        let tenthsTotal = (secs * 10).rounded()
        let whole = Int(tenthsTotal / 10)
        let tenths = Int(tenthsTotal) % 10
        return String(format: "%d:%02d.%d", whole / 60, whole % 60, tenths)
    }
}
