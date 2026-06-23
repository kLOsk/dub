//
//  DeckHeader.swift
//  Dub
//
//  Two-row strip above each deck's waveform region per PRD §9.2.
//  M10.5b adds a track-time line + a `MASTER` chip so the FS-browser
//  Space-load + master-deck semantics from §6.4 are visible at a
//  glance.
//
//  Row 1: deck label · source pill · MASTER chip · track title · artist · format chip
//  Row 2: pitch · BPM · key · FX chip
//  Row 3: (track loaded) Play/Pause · Restart · track time / total · remaining
//
//  M10.6a (Casual Play UI, PRD §6.1.3): the time row gains a left-
//  aligned transport-glyph cluster — Play/Pause toggle + Restart —
//  so the DJ can start file playback by mouse before a set begins
//  (or pause / restart it). Glyphs render exactly when `timeRow`
//  renders (i.e. a file track is loaded), which covers both the
//  Prep-mode single-deck shell and the Casual-Play-pre-Timecode
//  case in two-deck Timecode mode. Transport callbacks are passed
//  in from `PerformanceView` via a `DeckHeaderCallbacks` value.
//

import AppKit
import SwiftUI
import DubCore

/// Driving state for one deck header. Pure function of the model
/// (engine status + DeckState + master-deck flag) — the view does no
/// derivation of its own, which keeps it trivially snapshot-testable
/// in M18.
struct DeckHeaderState: Equatable {

    /// Whether the engine has an active source on this deck (Thru
    /// capture *or* a loaded File track). Drives the source pill's
    /// "ON / OFF" treatment.
    let isLive: Bool

    /// What the deck is currently sourcing audio from.
    let source: Source

    let trackTitle: String?
    let trackArtist: String?

    /// M10.5u. Estimated tempo for the loaded track, populated
    /// from `engine.beatGrid(deckIdx:)` after a successful
    /// `loadTrack`. `nil` when no track is loaded, when analysis
    /// bailed (silence / non-musical / too-short input), or when
    /// the estimator's confidence dropped to zero. The BPM stat
    /// column renders the dash in that case.
    let bpm: Double?

    /// Deck pitch as a signed percentage `(rate − 1) × 100`, from
    /// `engine.deckTelemetry`. `nil` when the deck isn't actively
    /// driven (paused / lifted / no source) — the PITCH column shows
    /// the dash. `var` with a default so the many `from(...)` call
    /// sites that don't set it keep compiling. (Turntables aren't
    /// mirrored — this column is in the same place on both decks.)
    var pitchPercent: Double? = nil

    /// Whether the pitch / live-BPM readout has finished its settling
    /// measurements (the once-per-rev wobble fit needs ~2 revolutions
    /// of locked play after the session's first needle drop). The
    /// PITCH and BPM columns render dimmed until then — the music
    /// plays immediately; only the *numbers* are still measuring.
    var pitchSettled: Bool = true

    /// Calibration / measurement progress [0, 1] from the engine
    /// (whitening capture + pitch stabilization, time-weighted). While
    /// `pitchSettled == false`, row 2 draws a progress line filling
    /// left → right so the DJ sees the session-start hold advancing
    /// and the exact moment the deck goes live.
    var measureProgress: Double = 1.0

    /// Timecode lock state for the source-pill tracking dot: 0 none ·
    /// 1 clean (green) · 2 degraded (amber) · 3 disengaged / scratching
    /// (red — which is *normal* while scratching, per Serato). `var`
    /// with a default so the existing `from(...)` call sites compile.
    var timecodeLockState: UInt8 = 0

    /// Source-control state for the row-3 Internal/Timecode switch
    /// (PRD §5.1.1). `nil` ⇒ the deck has no timecode input, so the
    /// switch is hidden (e.g. Prep mode / File-only). `var` default so
    /// existing `from(...)` call sites compile.
    var sourceControl: SourceControlStatus? = nil

    /// Whether the row-3 source-control mode is user-pinned (override)
    /// rather than auto-selected. Renders the "· PINNED" badge on the
    /// switch. `var` default so existing `from(...)` call sites compile.
    var sourceControlOverridden: Bool = false

    /// M11c.2. Canonical Camelot key of the loaded library track
    /// (e.g. `"8B"` for C major). `nil` when no track is loaded,
    /// when key analysis returned zero confidence, **or** when the
    /// load originated from a Finder drag (no library row means no
    /// `track_keys` association). The KEY stat column renders the
    /// dash in that case. The deck header always shows Camelot —
    /// the LibraryView's column-level click-to-toggle is the right
    /// surface for the musical-notation preference, not the deck
    /// header which DJs read at glance distance during a mix.
    let key: String?

    /// Format / SR caption ("MP3 · 44.1 kHz · stereo"). `nil` for
    /// Thru / off decks.
    let formatChip: String?

    /// File-mode time-row (elapsed / total + remaining). `nil` for
    /// Thru / off decks — no canonical playhead concept in Thru
    /// mode (timecode drives the rate).
    let timeRow: TimeRow?

    /// Whether this deck is the current master (PRD §6.4).
    let isMaster: Bool

    /// M10.6a: whether the deck is currently advancing the playhead.
    /// Drives the Play / Pause toggle in the transport-glyph cluster
    /// (PRD §6.1.3 Casual Play). Independent from `timeRow != nil`
    /// — `timeRow` says "a file is loaded so render the time
    /// indicators"; `isPlaying` says "the engine is advancing
    /// elapsed time right now". A paused-mid-track deck has
    /// `timeRow != nil` and `isPlaying == false` — the Play glyph
    /// shows.
    let isPlaying: Bool

    /// M10.6c: whether the engine has Panic Play engaged on this
    /// deck (PRD §6.1.2). When `true` the source pill flips to the
    /// `.tcHold` variant ("TC · HOLD" / amber dot) and the
    /// transport-cluster primary button renders the "re-engage
    /// timecode" icon. Authoritative source is the engine via
    /// `PositionInfo.isPanicPlay` (30 Hz poll); the model also sets
    /// it optimistically on `panic(side:)` for zero-frame UI
    /// latency.
    let isPanicPlay: Bool

    /// M10.6d / M11d.5: reserved for a future INT/ABS toggle
    /// surface. The primary transport button is now *always* a
    /// plain Play / Pause toggle (pre-M11d.5 it switched to a
    /// Serato-style INT/ABS toggle in Timecode mode, which felt
    /// broken to the user during pre-alpha dogfooding because no
    /// carrier was present and the deck paused on the next
    /// `DropoutHoldRate` render block). The Play action engages
    /// Panic Play internally in Performance mode so the deck
    /// actually advances; disengaging Panic to hand control back
    /// to a timecode driver moves to a separate affordance once
    /// the matching hardware UX lands. Kept in `DeckHeaderState`
    /// (rather than fully removed) so the equatable snapshot
    /// stays stable across the transition; setter callers pass
    /// `false`.
    let useTimecodeToggle: Bool

    /// M11d.7 — library grid lock + drift indicator plumbed at load.
    let gridLocked: Bool
    let gridDriftQuality: Float?

    /// M11d-history — display title of the track the DJ has most
    /// often mixed into from the loaded one (top `played_into`
    /// row, stamped at load). Renders as the "↝ usually: <track>"
    /// hint in row 3's spare middle region — Performance mode
    /// (`.remainingOnly`) only; Prep mode's row 3 is fully
    /// occupied by elapsed + remaining. `var` with a default so
    /// existing `from(...)` call sites keep compiling.
    var historyHint: String? = nil

    enum Source: Equatable {
        case off
        case thru
        case timecode
        case file
        /// M10.6c. Engine mode is Timecode, a file track is the
        /// audio source, but Panic Play is engaged so the deck is
        /// decoupled from its timecode input and holding the last-
        /// known velocity (PRD §6.1.2). Renders as `TC · HOLD` with
        /// an amber dot.
        case tcHold
        /// M10.5d. A `load_track` FFI call is in flight on this
        /// deck (decode + offline peaks running on a background
        /// `Task.detached`). Renders as `LOADING…` with an amber
        /// dot — supersedes `.file` / `.tcHold` while the load is
        /// running so the user sees the deck is busy.
        case loading
    }

    /// Time-row layout the deck header should render (M10.5r).
    ///
    /// Two variants. **Performance mode** shows only the remaining
    /// time — the DJ's "30 seconds left to mix" cue (PRD §6.1). The
    /// header is space-constrained in the two-deck split and the
    /// total + elapsed values aren't actionable mid-set, so we
    /// drop them. **Prep mode** shows both elapsed and remaining
    /// because the rehearsal surface has the screen real-estate
    /// and the DJ uses elapsed time for hot-cue placement.
    ///
    /// **M11d.5 round 5 — payload dropped.** The cases used to
    /// carry pre-formatted `elapsedText` / `remainingText` strings
    /// derived from `DeckState.elapsedSecs` / `.remainingSecs`. Those
    /// fields are gone (the time text now reads
    /// `engine.position(deckIdx:)` directly via the
    /// `LiveDeckTimeText` subview, keeping per-second updates
    /// confined to a `TimelineView` subtree). The enum survives
    /// as a *layout selector* — which slots the header should
    /// reserve and how to mirror them — without carrying the text
    /// itself. The `liveEngine` / `liveDeckIdx` params on
    /// `DeckHeader` supply the data; the preview / cold-launch
    /// path renders `"--:--"` placeholders via the live view's
    /// own zero-position fallback.
    enum TimeRow: Equatable {
        /// Performance-mode minimal display: `"-MM:SS"` only.
        case remainingOnly
        /// Prep-mode full display: `"MM:SS"` + `"-MM:SS"`.
        case elapsedAndRemaining

        /// True when the time row should render at all. Equivalent
        /// to the old `timeRow != nil` check; kept on the enum so
        /// callers don't have to pattern-match in three places.
        var hasTime: Bool {
            switch self {
            case .remainingOnly: return true
            case .elapsedAndRemaining: return true
            }
        }
    }

    /// Convenience: idle / cold-launch state.
    static let idle = DeckHeaderState(
        isLive: false, source: .off,
        trackTitle: nil, trackArtist: nil, bpm: nil, key: nil,
        formatChip: nil, timeRow: nil, isMaster: false, isPlaying: false,
        isPanicPlay: false, useTimecodeToggle: false,
        gridLocked: false, gridDriftQuality: nil)
}

/// M10.6a transport callbacks the deck header invokes when the user
/// clicks Play / Pause / Restart in the time row. Kept off
/// `DeckHeaderState` so the state value stays `Equatable` (closures
/// aren't). `PerformanceView` constructs an instance per render that
/// forwards into `WaveformAppModel.{play, pause, restart}(side:)`.
struct DeckHeaderCallbacks {
    /// Casual-Play start (Prep mode + track loaded + paused).
    var onPlay:    () -> Void = {}
    /// Casual-Play pause (Prep mode + track loaded + playing).
    var onPause:   () -> Void = {}
    /// M10.6d INT/ABS toggle. Used by the transport cluster when
    /// the engine is in Timecode mode with a track loaded: tap
    /// engages Panic Play (internal playback at last-known rate);
    /// tap-while-engaged cancels it (hand back to timecode driver).
    /// `PerformanceView` routes this to
    /// `WaveformAppModel.panicToggle(side:)`.
    var onPanicToggle: () -> Void = {}

    /// M11d.7 tap-to-grid on the BPM column.
    var onTapBpm: (() -> Void)? = nil

    /// Deck-header BPM context menu — "2×" entry. Doubles the
    /// active library beatgrid's BPM and re-installs it on the
    /// engine while keeping the visible downbeat anchored.
    var onDoubleBpm: (() -> Void)? = nil
    /// Deck-header BPM context menu — "½" entry. Halves the BPM
    /// with the same downbeat-preservation invariant as the "2×"
    /// entry.
    var onHalveBpm: (() -> Void)? = nil
    /// Deck-header BPM context menu — "Reset" entry. Drops user-tap
    /// edits and reverts to the original auto-analysis grid.
    var onResetBpm: (() -> Void)? = nil
    /// Deck-header BPM context menu — "Lock grid" / "Unlock grid"
    /// entry. The auto-analyse pass no longer auto-locks (user
    /// feedback: silent freezes after a tap are hostile), so this
    /// is now the only place a user blesses or unblesses a grid
    /// from the performance surface. Mirrors the library-row
    /// context menu's Lock/Unlock entry — same FFI call under the
    /// hood — but kept close to the BPM number where the
    /// "good to go" decision actually happens.
    var onToggleGridLocked: (() -> Void)? = nil

    /// Source-control switch (PRD §5.1.1): select Internal / Timecode /
    /// Thru for the deck, and manually recalibrate the needle.
    var onSetInternal: (() -> Void)? = nil
    var onSetTimecode: (() -> Void)? = nil
    var onSetThru: (() -> Void)? = nil
    var onRecalibrate: (() -> Void)? = nil

    /// M11d-history — click on the row-3 "↝ usually: <track>" hint.
    /// Reveals the suggested track in the library browser (PRD §9.5
    /// row 3); Space then loads the selection via the existing flow.
    var onHistoryHintTap: () -> Void = {}

    /// No-op fallback used by the cold-launch / preview state where
    /// no model is wired in yet.
    static let noop = DeckHeaderCallbacks()
}

/// The deck header. Stateless — caller supplies a `DeckHeaderState`
/// per render.
///
/// `mirrored` flips the row layouts horizontally so the deck-identity
/// cluster (`DECK A` / `DECK B` label + source pill + MASTER chip)
/// renders on the *outer* edge of the deck's pane rather than against
/// the inner divider. Every two-deck DJ application (Serato, Traktor,
/// rekordbox) does this — without it, deck A's label sits at the
/// window's left edge while deck B's label is pinned against the
/// divider in the middle of the window, which reads as "deck B is
/// misaligned" at glance distance. Performance/Timecode mode passes
/// `mirrored: true` for deck B; Prep mode (single deck) never
/// mirrors.
struct DeckHeader: View {

    let side: DeckSide
    let state: DeckHeaderState
    /// M10.6a Casual-Play transport callbacks. Defaults to no-op so
    /// the cold-launch / preview path doesn't have to wire anything.
    var callbacks: DeckHeaderCallbacks = .noop
    /// `true` to render this header as a horizontal mirror of the
    /// canonical layout. Deck identity moves to the trailing edge;
    /// format chip / FX chip / transport glyphs move to the leading
    /// edge. See the `DeckHeader` doc comment for why this matters.
    var mirrored: Bool = false
    /// Prep mode shows BPM to two decimals; Performance mode one.
    var prepMode: Bool = false
    /// Per-deck published surface for the open tap-tempo session.
    /// Drives the parenthesised count chip and the italic
    /// rolling-BPM preview in the BPM column. Deliberately
    /// observed as a separate `ObservableObject` so a tap during
    /// playback only invalidates this header's body — not the
    /// entire `WaveformAppModel`'s view tree. See
    /// `TapSessionViewModel` for the full rationale. Preview
    /// callers default to a fresh empty session so SwiftUI
    /// previews keep rendering without booting the app model.
    @ObservedObject var tapSession: TapSessionViewModel = TapSessionViewModel()
    /// M11d.5 round 5 — engine reference + deck index for the
    /// self-driven time-row text. When supplied, the time row
    /// renders a `LiveDeckTimeText` subview that reads
    /// `engine.position(deckIdx:)` directly inside its own
    /// `TimelineView` instead of consuming pre-formatted strings
    /// from `state.timeRow`. This is what stops `model.deckA` from
    /// republishing once a second when the M:SS rolls over — the
    /// time-text updates are confined to a small `TimelineView`
    /// subtree, and the rest of the deck pane (DeckHeader, parent
    /// `PerformanceView`, the Metal view's wrapper hierarchy)
    /// stays inert at that cadence.
    ///
    /// `nil` for the preview / cold-launch path: `state.timeRow`'s
    /// embedded strings render as fallback so the SwiftUI previews
    /// stay self-contained without spinning up an engine.
    var liveEngine: DubEngine? = nil
    var liveDeckIdx: UInt64? = nil

    var body: some View {
        // M11d.5 fix: header height is **fixed**, not min-heighted.
        // Pre-fix the header used `minHeight:` and added Row 3 only
        // when a track was loaded, so loading a track grew the
        // header by ~24 px and visually jumped the whole layout.
        // The user saw this as "header changes size when a track is
        // loaded". The fix: always reserve Row 3's vertical slot
        // (renders empty content when no track) so loaded vs idle
        // decks share the same header height inside the same HStack.
        VStack(alignment: mirrored ? .trailing : .leading,
               spacing: DubSpacing.sm) {
            row1
            row2
                .overlay(alignment: .bottom) {
                    // Overlay, not a row: the header has a fixed
                    // height and a new row would shift the layout
                    // (the documented header-height-jump bug).
                    if !state.pitchSettled {
                        calibrationProgressLine
                            .offset(y: 4)
                    }
                }
            row3Reserved
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.md)
        .frame(maxWidth: .infinity,
               alignment: mirrored ? .trailing : .leading)
        .frame(height: DubLayout.deckHeaderHeight)
        .background(DubColor.surface1)
        .clipped()
    }

    /// Row 3 wrapped so it always occupies the same vertical slot.
    /// When a track is loaded the actual transport + time content
    /// renders inside; when nothing is loaded an empty
    /// `Color.clear` placeholder of the same intrinsic height
    /// occupies the slot so the header height stays constant. The
    /// transport glyph's `frame(height: 20)` sets the intrinsic
    /// height we reserve.
    @ViewBuilder
    private var row3Reserved: some View {
        if let time = state.timeRow, time.hasTime {
            timeRow(time)
        } else if state.sourceControl != nil {
            // No track loaded yet, but the deck has a timecode input —
            // show the source switch on its own so the DJ can pick
            // INT / TC / THRU before (or without) loading a file.
            sourceSwitchRow
        } else {
            Color.clear
                .frame(height: 20)
                .frame(maxWidth: .infinity)
        }
    }

    /// The three-way source switch, when this deck has one. Pulled out
    /// so both the time row (track loaded) and the standalone row (no
    /// track) render the same control, on both deck orientations.
    @ViewBuilder
    private var sourceSwitchView: some View {
        if let sc = state.sourceControl {
            SourceControlView(
                status: sc, overridden: state.sourceControlOverridden,
                isPlaying: state.isPlaying, side: side,
                onInternal: { callbacks.onSetInternal?() },
                onPause: { callbacks.onPause() },
                onTimecode: { callbacks.onSetTimecode?() },
                onThru: { callbacks.onSetThru?() },
                onRecalibrate: { callbacks.onRecalibrate?() })
        }
    }

    /// Row-3 layout when no track is loaded: just the source switch,
    /// pinned to the deck's outer edge like the rest of the header.
    private var sourceSwitchRow: some View {
        HStack(spacing: 0) {
            if mirrored {
                Spacer(minLength: 0)
                sourceSwitchView
            } else {
                sourceSwitchView
                Spacer(minLength: 0)
            }
        }
        .frame(height: 20)
        .frame(maxWidth: .infinity, alignment: mirrored ? .trailing : .leading)
    }

    // MARK: - Row 1 — identity

    @ViewBuilder
    private var row1: some View {
        // M11d.5 fix: explicit `layoutPriority` on the identity
        // cluster (deckLabel + sourcePill + masterChip) so SwiftUI
        // never compresses them when the title / artist / format-
        // chip cluster runs out of horizontal room. Without these
        // priorities, a long title pushed `DECK A` to zero width
        // and the user saw "the deck label disappeared after I
        // loaded a track". The title-artist group keeps the
        // default priority so it's the one that truncates first
        // (its `.lineLimit(1).truncationMode(.middle)` is already
        // wired for that). The format chip gets a mid priority so
        // it survives mild squeezes but yields before identity.
        HStack(spacing: DubSpacing.md) {
            if mirrored {
                if let chip = state.formatChip {
                    formatChipView(chip).layoutPriority(1)
                }
                Spacer(minLength: 0)
                titleArtistGroup
                if state.isMaster, !prepMode {
                    masterChip.layoutPriority(2)
                }
                if showSourcePill {
                    sourcePill.layoutPriority(2)
                }
                deckLabel.layoutPriority(2)
            } else {
                deckLabel.layoutPriority(2)
                if showSourcePill {
                    sourcePill.layoutPriority(2)
                }
                if state.isMaster, !prepMode {
                    masterChip.layoutPriority(2)
                }
                titleArtistGroup
                Spacer(minLength: 0)
                if let chip = state.formatChip {
                    formatChipView(chip).layoutPriority(1)
                }
            }
        }
        .frame(maxWidth: .infinity,
               alignment: mirrored ? .trailing : .leading)
    }

    /// Title + artist text pair. Pulled out of `row1` so the mirror /
    /// non-mirror branches don't duplicate the rendering decisions.
    /// The artist text keeps its leading `"· "` separator in both
    /// orientations — even when the cluster is right-aligned the
    /// title still precedes the artist in reading order, so the
    /// separator stays a *post*-title glyph.
    @ViewBuilder
    private var titleArtistGroup: some View {
        if let title = state.trackTitle {
            Text(title)
                .font(DubFont.title)
                .foregroundStyle(DubColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.middle)
        } else {
            placeholderText("—", font: DubFont.title)
        }
        if let artist = state.trackArtist {
            Text("· \(artist)")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
        }
    }

    // MARK: - Row 2 — live stats
    //
    // Stat order (PITCH → BPM → KEY) stays the same on both decks —
    // reversing it would force the user to learn two reading orders
    // for the same labelled values, which is a worse cost than the
    // mild asymmetry of having the cluster be right-aligned on the
    // mirrored side. Only the FX chip's position swaps so it stays
    // on the *inner* (divider-adjacent) edge, mirroring Row 1's
    // format-chip behaviour.

    /// Thin left→right progress line shown while the deck's
    /// session-start measurements run (the playback hold): whitening
    /// capture + pitch stabilization. The engine publishes ONE ready
    /// predicate for the gate, this line, and the readout dimming, so
    /// the line completing IS the moment the gated track starts (the
    /// short fill animation keeps the visual within ~150 ms of the
    /// audio).
    private var calibrationProgressLine: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(DubColor.surface2)
                Capsule()
                    .fill(DubColor.deckTint(side))
                    .frame(width: geo.size.width
                        * max(0.02, min(1.0, state.measureProgress)))
                    .animation(.linear(duration: 0.05),
                               value: state.measureProgress)
            }
        }
        .frame(height: 3)
        .accessibilityLabel("Calibrating")
    }

    @ViewBuilder
    private var row2: some View {
        HStack(spacing: DubSpacing.lg) {
            if mirrored {
                fxChip
                Spacer(minLength: 0)
                statColumn(label: "PITCH", value: formattedPitch)
                    .opacity(state.pitchSettled ? 1.0 : 0.45)
                bpmStatColumn
                    .opacity(state.pitchSettled ? 1.0 : 0.45)
                statColumn(label: "KEY", value: formattedKey)
            } else {
                statColumn(label: "PITCH", value: formattedPitch)
                    .opacity(state.pitchSettled ? 1.0 : 0.45)
                bpmStatColumn
                    .opacity(state.pitchSettled ? 1.0 : 0.45)
                statColumn(label: "KEY", value: formattedKey)
                Spacer(minLength: 0)
                fxChip
            }
        }
        .frame(maxWidth: .infinity,
               alignment: mirrored ? .trailing : .leading)
    }

    /// Render the BPM column. The Stage-1 estimator delivers two
    /// decimals' worth of precision but DJs read tempo as a
    /// whole-number "around 128" cue at glance distance, so the
    /// column shows one decimal (e.g. `128.0`, `92.5`) — enough
    /// to disambiguate adjacent half-time tracks (88 vs 88.5)
    /// without forcing the eye to parse a `127.94` digit string
    /// every time. The dash falls out naturally when
    /// `state.bpm == nil`: no track / analysis bailed / zero
    /// confidence.
    ///
    /// PRD-BEATS §4.2 + gate 12: when a tap session is open and
    /// has emitted a rolling preview, render that preview value
    /// instead of the committed BPM so the user can see the
    /// running estimate converge in real time.
    /// The **live** tempo: the track's analyzed BPM scaled by the
    /// deck's current pitch, so the column reads the BPM the record is
    /// *actually playing at*. When the platter slows the BPM drops with
    /// it; this is the number you beatmatch against. Falls back to the
    /// nominal BPM when the deck isn't being driven (`pitchPercent ==
    /// nil`).
    private var liveBpm: Double? {
        guard let base = state.bpm, base > 0 else { return nil }
        guard let pitch = state.pitchPercent else { return base }
        return base * (1.0 + pitch / 100.0)
    }

    private var formattedBPM: String {
        // Tap-tempo preview wins while a tap session is rolling.
        let value = tapSession.rollingBpm ?? liveBpm
        guard let bpm = value, bpm > 0 else { return "—" }
        if prepMode {
            return String(format: "%.2f", bpm)
        }
        return String(format: "%.1f", bpm)
    }

    /// Render the KEY column. M11c.2: the active `track_keys` row
    /// is canonical Camelot. We surface it verbatim on the deck
    /// header — there's no per-deck "which notation" preference (the
    /// LibraryView's column-level toggle is the right surface for
    /// that). Em-dash for `nil` / empty / Finder-drag loads where
    /// the source isn't a library row.
    private var formattedKey: String {
        guard let k = state.key, !k.isEmpty else { return "—" }
        return k
    }

    /// Render the PITCH column: signed percentage of the deck's
    /// playback rate vs unity (`+0.0 %` at 1.0×). For timecode decks
    /// this tracks the platter; File playback reads `+0.0 %`. The dash
    /// shows when the deck isn't being driven (`state.pitchPercent ==
    /// nil`). Shown in the same place on both decks (headers aren't
    /// mirrored) so the eye always lands on it.
    private var formattedPitch: String {
        guard let p = state.pitchPercent else { return "—" }
        return String(format: "%+.1f %%", p)
    }

    // MARK: - Row 3 — track time + transport glyphs (track loaded)
    //
    // Mirror: when `mirrored == true`, transport glyphs move to the
    // trailing edge so they sit directly under the DECK B label in
    // Row 1. Elapsed/remaining swap correspondingly so the layout
    // reads "remaining ··· elapsed [transport]" on the mirrored
    // side. The numeric strings themselves are not reversed — only
    // their positions inside the HStack.

    @ViewBuilder
    private func timeRow(_ time: DeckHeaderState.TimeRow) -> some View {
        HStack(spacing: DubSpacing.md) {
            if mirrored {
                switch time {
                case .remainingOnly:
                    liveTime(slot: .remaining, textColor: DubColor.textPrimary)
                    Spacer(minLength: 0)
                    historyHintView
                case .elapsedAndRemaining:
                    liveTime(slot: .remaining, textColor: DubColor.textSecondary)
                    Spacer(minLength: 0)
                    liveTime(slot: .elapsed, textColor: DubColor.textPrimary)
                }
                sourceSwitchView
                // No separate Play button when the source switch is
                // present — the INT position is the play control. The
                // transport glyph stays only in Prep mode (no switch).
                if state.sourceControl == nil { transportGlyphs }
            } else {
                if state.sourceControl == nil { transportGlyphs }
                sourceSwitchView
                switch time {
                case .remainingOnly:
                    historyHintView
                    Spacer(minLength: 0)
                    liveTime(slot: .remaining, textColor: DubColor.textPrimary)
                case .elapsedAndRemaining:
                    liveTime(slot: .elapsed, textColor: DubColor.textPrimary)
                    Spacer(minLength: 0)
                    liveTime(slot: .remaining, textColor: DubColor.textSecondary)
                }
            }
        }
        .monospacedDigit()
    }

    /// M11d-history "↝ usually: <track>" hint (PRD §9.5 row 3).
    /// Fills the otherwise-empty middle region of the Performance
    /// time row with the most-mixed-into target for the loaded
    /// track, so the DJ sees their own past answer to "what do I
    /// reach for next?" the moment a track lands on the deck.
    /// Truncates tail-first and never competes with the remaining
    /// time (which is the §6.1 "30 seconds to mix" cue and keeps
    /// layout priority).
    @ViewBuilder
    private var historyHintView: some View {
        if let hint = state.historyHint, !hint.isEmpty {
            Button(action: callbacks.onHistoryHintTap) {
                Text("↝ usually: \(hint)")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            .buttonStyle(.plain)
            .help("Show “\(hint)” in the library — Space loads the selection.")
            .layoutPriority(-1)
        }
    }

    /// Render one time-slot. Production callers (Performance /
    /// Prep modes) supply `liveEngine` + `liveDeckIdx`, in which
    /// case we hand off to `LiveDeckTimeText`, a `TimelineView`-
    /// driven subview that reads `engine.position(deckIdx:)`
    /// directly. Preview callers leave the live params `nil`; we
    /// render an `"--:--"` placeholder so static SwiftUI previews
    /// still look correct without booting an engine.
    @ViewBuilder
    private func liveTime(
        slot: LiveDeckTimeText.Slot,
        textColor: Color
    ) -> some View {
        if let engine = liveEngine, let deckIdx = liveDeckIdx {
            LiveDeckTimeText(engine: engine, deckIdx: deckIdx, slot: slot)
                .font(DubFont.numericInline)
                .foregroundStyle(textColor)
        } else {
            Text(slot == .remaining ? "-00:00" : "00:00")
                .font(DubFont.numericInline)
                .foregroundStyle(textColor)
        }
    }

    /// Transport-cluster primary button (PRD §6.1).
    ///
    /// Sits left of the elapsed-time numbers in Row 3, and only
    /// renders because `timeRow(_:)` only renders when a track is
    /// loaded — no button in Thru mode where there's no canonical
    /// playhead. Branches on `useTimecodeToggle`:
    ///
    /// * Prep mode (`useTimecodeToggle == false`): classic
    ///   Play/Pause toggle. Drives `onPlay` / `onPause` per
    ///   `isPlaying`.
    /// * Timecode mode + track loaded (`useTimecodeToggle == true`):
    ///   Serato-style INT/ABS toggle. Drives `onPanicToggle` either
    ///   way; the icon flips between `play.fill` (currently following
    ///   platter — tap to play internally) and `opticaldisc.fill`
    ///   amber (currently internal — tap to re-engage timecode).
    ///   Subsumes Casual Play: a paused-in-Timecode deck still
    ///   shows `play.fill`, and tapping engages Panic Play which
    ///   starts internal playback at last-known rate — fixing the
    ///   "Play does nothing in Timecode mode" bug where the prior
    ///   `engine.play` call was instantly overwritten by the next
    ///   `DropoutHoldRate` block.
    ///
    /// The Restart button from the M10.6a draft is gone: the
    /// Track Overview strip's click-to-top handles seek-to-zero,
    /// and Panic Play handles "keep playing through a glitch", so
    /// we don't need a third glyph.
    private var transportGlyphs: some View {
        HStack(spacing: DubSpacing.sm) {
            primaryButton
        }
    }

    @ViewBuilder
    private var primaryButton: some View {
        // M11d.5: primary button is always Play / Pause. The
        // pre-fix INT/ABS toggle is kept as a private subview for
        // when the matching hardware UX ships (see
        // `useTimecodeToggle` doc comment).
        playPauseButton
    }

    /// Prep-mode Play/Pause toggle (PRD §6.1.3).
    private var playPauseButton: some View {
        transportButton(
            systemName: state.isPlaying ? "pause.fill" : "play.fill",
            accessibilityLabel: state.isPlaying ? "Pause" : "Play",
            tint: DubColor.textPrimary,
            background: DubColor.surface2,
            action: state.isPlaying ? callbacks.onPause : callbacks.onPlay)
    }

    /// Timecode-mode INT/ABS toggle (PRD §6.1.2 / M10.6d). Amber
    /// tint + background while panic is engaged so the button
    /// visually agrees with the `TC · HOLD` source-pill amber dot.
    private var timecodeToggleButton: some View {
        transportButton(
            systemName: state.isPanicPlay
                ? "opticaldisc.fill"
                : "play.fill",
            accessibilityLabel: state.isPanicPlay
                ? "Re-engage timecode"
                : "Play internally (disengage timecode)",
            tint: state.isPanicPlay
                ? DubColor.stateTentative
                : DubColor.textPrimary,
            background: state.isPanicPlay
                ? DubColor.stateTentative.opacity(0.15)
                : DubColor.surface2,
            action: callbacks.onPanicToggle)
    }

    @ViewBuilder
    private func transportButton(
        systemName: String,
        accessibilityLabel: String,
        tint: Color,
        background: Color,
        action: @escaping () -> Void
    ) -> some View {
        // Deck transport (Play / Pause / Panic / TC-engage) fires on
        // mouse-**down**, like a hardware deck button and the cue /
        // BPM-tap controls — pressing starts the action immediately
        // instead of waiting for release (see `View.onPressDown`).
        Image(systemName: systemName)
            .symbolRenderingMode(.monochrome)
            .font(.system(size: 12, weight: .medium))
            .foregroundStyle(tint)
            .frame(width: 20, height: 20)
            .background(background)
            .clipShape(RoundedRectangle(cornerRadius: 3, style: .continuous))
            .onPressDown(perform: action)
            .accessibilityLabel(accessibilityLabel)
            .accessibilityAddTraits(.isButton)
    }

    // MARK: - Subviews

    private var deckLabel: some View {
        Text(side.label)
            .font(DubFont.caps)
            .tracking(1.2)
            .foregroundStyle(DubColor.deckTint(side))
    }

    /// Pill: bullet + source name. Pill colour follows live state
    /// (locked green when capturing / playing, secondary grey when
    /// idle) — a quick at-a-distance "is the deck running?" tell.
    private var sourcePill: some View {
        HStack(spacing: DubSpacing.xs) {
            Circle()
                .fill(sourcePillDotColor)
                .frame(width: 6, height: 6)
            Text(sourcePillLabel)
                .font(DubFont.caps)
                .tracking(0.6)
                .foregroundStyle(DubColor.textSecondary)
        }
        .padding(.horizontal, DubSpacing.sm)
        .padding(.vertical, 3)
        .background(DubColor.surface2)
        .clipShape(Capsule())
    }

    /// MASTER chip — visible only on the master deck. Anchors the
    /// "which deck does Space-load avoid" UI affordance from PRD §6.4.
    private var masterChip: some View {
        Text("MASTER")
            .font(DubFont.caps)
            .tracking(0.8)
            .foregroundStyle(DubColor.deckTint(side))
            .padding(.horizontal, DubSpacing.sm)
            .padding(.vertical, 2)
            .overlay(
                Capsule(style: .continuous)
                    .stroke(DubColor.deckTint(side), lineWidth: 1)
            )
    }

    /// Prep mode hides the FILE / LOADING source pill and the
    /// MASTER chip — single-deck rehearsal doesn't need load-
    /// target or master-deck chrome at glance distance.
    private var shouldHideSourcePill: Bool {
        switch state.source {
        case .file, .loading: return true
        default: return false
        }
    }

    /// Whether row 1 shows its source pill.
    ///
    /// Two suppression rules, both about avoiding redundant chrome:
    ///
    /// * **Performance mode, timecode deck:** when the row-3
    ///   Internal/Timecode switch is present, it already names the
    ///   source *and* carries the tracking dot, so a second "FILE"
    ///   pill in row 1 is noise. Suppress it for the steady `.file`
    ///   state only — the transient `.loading` and panic `.tcHold`
    ///   states stay (they say something the switch doesn't).
    /// * **Prep mode:** hides the FILE / LOADING pill as before
    ///   (single-deck rehearsal needs no load-target chrome).
    private var showSourcePill: Bool {
        if state.sourceControl != nil, state.source == .file {
            return false
        }
        if prepMode, shouldHideSourcePill {
            return false
        }
        return true
    }

    private var sourcePillLabel: String {
        switch state.source {
        case .off:      return "OFF"
        case .thru:     return state.isLive ? "THRU · LIVE" : "THRU"
        case .timecode: return state.isLive ? "TIMECODE · LIVE" : "TIMECODE"
        case .file:     return "FILE"
        case .tcHold:   return "TC · HOLD"
        case .loading:  return "LOADING…"
        }
    }

    private var sourcePillDotColor: Color {
        guard state.isLive else { return DubColor.textPlaceholder }
        // When a timecode input is attached, the dot reports signal
        // health (PRD §5.4 tracking dot) rather than just source kind:
        // green clean · amber degraded · red disengaged (scratch/lift —
        // red is normal there). Falls through to source-based colour
        // when there's no timecode input.
        switch state.timecodeLockState {
        case 1:  return DubColor.stateLocked
        case 2:  return DubColor.stateTentative
        case 3:  return DubColor.stateError
        default: break
        }
        switch state.source {
        case .off:      return DubColor.textPlaceholder
        case .thru:     return DubColor.stateLocked
        case .timecode: return DubColor.stateLocked
        case .file:     return DubColor.stateTentative
        case .tcHold:   return DubColor.stateTentative
        case .loading:  return DubColor.stateTentative
        }
    }

    @ViewBuilder
    private func formatChipView(_ text: String) -> some View {
        Text(text)
            .font(DubFont.micro)
            .foregroundStyle(DubColor.textTertiary)
            .padding(.horizontal, DubSpacing.sm)
            .padding(.vertical, 2)
            .background(DubColor.surface2)
            .clipShape(RoundedRectangle(cornerRadius: 3, style: .continuous))
    }

    @ViewBuilder
    private var bpmStatColumn: some View {
        HStack(spacing: DubSpacing.sm) {
            Text("BPM")
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
            // Tap fires on mouse-**down**, not via a `Button` (mouse-
            // up): the tap captures the live playhead + a wall-clock
            // timestamp the instant you tap, and a `Button`'s mouse-up
            // delay landed both the click-hold duration too late
            // (visible as the tap sitting later than the keyboard's
            // key-down path). See `View.onPressDown`.
            HStack(spacing: 4) {
                Text(formattedBPM)
                    .font(
                        tapSession.rollingBpm != nil
                            ? DubFont.numericInline.italic()
                            : DubFont.numericInline
                    )
                    .foregroundStyle(bpmDisplayColor)
                if tapSession.tapCount > 0 {
                    Text("(\(tapSession.tapCount))")
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.deckTint(side))
                }
                if state.gridLocked {
                    Image(systemName: "lock.fill")
                        .font(.system(size: 9))
                        .foregroundStyle(DubColor.textSecondary)
                } else if let drift = state.gridDriftQuality,
                          abs(drift) >= 3
                {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .font(.system(size: 9))
                        .foregroundStyle(.orange)
                }
            }
            .underline(
                tapSession.tapCount > 0,
                color: DubColor.deckTint(side).opacity(0.55))
            .onPressDown(
                enabled: callbacks.onTapBpm != nil && state.bpm != nil && !state.gridLocked
            ) {
                callbacks.onTapBpm?()
            }
            .help(bpmColumnHelpText)
        }
        // Attach the context menu at the column level (not on the
        // tap target) so it stays reachable when the tap is disabled
        // by `state.gridLocked` — `onPressDown(enabled:)` ignores the
        // left-click then, but right-click must still reach "Unlock
        // grid", the only path to it from the performance surface.
        .contextMenu { bpmContextMenu }
    }

    /// Deck-header BPM right-click menu (PRD-BEATS §4 octave
    /// override + grid-lock toggle). The 2× / ½ / Reset entries
    /// write a `user_tap` row through the library so the change
    /// survives a reload and a future re-analyse can demote them;
    /// they're hidden when the grid is locked. The Lock / Unlock
    /// entry is ALWAYS shown — it's the only performance-surface
    /// path to flip the lock now that auto-analyse no longer
    /// auto-locks (user feedback: "for now disable auto grid lock
    /// instead right clicking the bpm should also have the option
    /// to lock a grid or to unlock it").
    @ViewBuilder
    private var bpmContextMenu: some View {
        if !state.gridLocked, state.bpm != nil {
            if let onDoubleBpm = callbacks.onDoubleBpm {
                Button("2×", action: onDoubleBpm)
            }
            if let onHalveBpm = callbacks.onHalveBpm {
                Button("½", action: onHalveBpm)
            }
            if callbacks.onDoubleBpm != nil || callbacks.onHalveBpm != nil {
                Divider()
            }
            if let onResetBpm = callbacks.onResetBpm {
                Button("Reset to auto", action: onResetBpm)
                Divider()
            }
        }
        if let onToggleGridLocked = callbacks.onToggleGridLocked {
            Button(state.gridLocked ? "Unlock grid" : "Lock grid",
                   action: onToggleGridLocked)
        }
    }

    /// PRD-BEATS §4.2 + gate 12: rolling preview takes the deck
    /// tint colour so the user can see at a glance that the
    /// displayed value is provisional. Otherwise the normal
    /// placeholder/primary split applies. Locked grids deliberately
    /// keep the primary colour (user feedback: "if a grid is
    /// locked the bpm shouldnt be dark. the dark lock is fine but
    /// bpm should be the regular colour") — the lock icon next to
    /// the number carries the lock-state signal on its own.
    private var bpmDisplayColor: Color {
        if tapSession.rollingBpm != nil {
            return DubColor.deckTint(side)
        }
        return state.bpm == nil ? DubColor.textPlaceholder : DubColor.textPrimary
    }

    /// Tooltip on the BPM column. Surfaces the dynamic-window
    /// rule and the lock semantics so the user can discover
    /// the contract without leaving the app. Adapts to the
    /// current lock state so a locked-grid hover explains why
    /// tap-tempo is greyed out.
    private var bpmColumnHelpText: String {
        if state.gridLocked {
            return "Grid is locked. Right-click to unlock before tapping."
        }
        return "Tap for downbeat (1–2 taps) or tempo recalc (3+ taps). "
            + "Tap window adapts to the tempo (1.5 s minimum). "
            + "Right-click for 2× / ½ / Reset / Lock."
    }

    @ViewBuilder
    private func statColumn(label: String, value: String) -> some View {
        HStack(spacing: DubSpacing.sm) {
            Text(label)
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
            Text(value)
                .font(DubFont.numericInline)
                .foregroundStyle(
                    value == "—" ? DubColor.textPlaceholder : DubColor.textPrimary
                )
        }
    }

    private var fxChip: some View {
        HStack(spacing: DubSpacing.xs) {
            Text("FX")
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
            Text("—")
                .font(DubFont.numericInline)
                .foregroundStyle(DubColor.textPlaceholder)
        }
        .padding(.horizontal, DubSpacing.sm)
        .padding(.vertical, 3)
        .overlay(
            RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                .stroke(DubColor.divider, lineWidth: 1)
        )
    }

    @ViewBuilder
    private func placeholderText(_ text: String, font: Font) -> some View {
        Text(text)
            .font(font)
            .foregroundStyle(DubColor.textPlaceholder)
    }
}

// MARK: - Time formatting

/// Format a duration in seconds as `MM:SS` (or `HH:MM:SS` for tracks
/// over 60 minutes — DJ mix-files do exist). Returns `"--:--"` for
/// negative / NaN inputs so we never crash on a transient bad poll.
enum DeckTimeFormat {
    static func format(_ secs: Double, signed: Bool = false) -> String {
        guard secs.isFinite, secs >= 0 else { return "--:--" }
        let total = Int(secs)
        let hh = total / 3600
        let mm = (total / 60) % 60
        let ss = total % 60
        let sign = signed ? "-" : ""
        if hh > 0 {
            return String(format: "%@%02d:%02d:%02d", sign, hh, mm, ss)
        }
        return String(format: "%@%02d:%02d", sign, mm, ss)
    }
}

// MARK: - LiveDeckTimeText (M11d.5 round 5)

/// Self-driven SwiftUI text view for one slot of the deck-header
/// time row.
///
/// Reads `engine.position(deckIdx:)` from inside a `TimelineView`
/// so the elapsed / remaining digits update on their own cadence,
/// independently of any `@Published` deck-state stream. The whole
/// point of the indirection is to keep the per-second M:SS
/// rollover from invalidating the parent `PerformanceView` and
/// triggering a full deck-pane body re-eval (which was the
/// residual cause of the "subtle leftward jump every second"
/// reported after M11d.5 round 3 — see SHIPPED for the bisection).
///
/// `TimelineView(.periodic(from: .now, by: 0.5))` is used over a
/// per-frame `.animation` schedule on purpose: the rendered text
/// only changes once a second (the integer-second floor of the
/// FFI's `elapsedSecs` / `remainingSecs`), so 2 Hz is more than
/// enough to keep the rollover visually fresh and the closure
/// stays cheap (one FFI position read, two integer divisions,
/// one `String(format:)` call per tick). The text widget already
/// carries `.monospacedDigit()` from the parent so per-tick layout
/// is byte-stable; the only main-thread work is a Text-node
/// content swap when the second crosses, which SwiftUI handles in
/// microseconds and which does **not** propagate up to invalidate
/// any ancestor that doesn't directly read `engine.position`.
///
/// The view is intentionally tiny: one `Text` widget per slot. Use
/// `.font(...)` and `.foregroundStyle(...)` modifiers on the
/// `LiveDeckTimeText` instance to customise appearance; both are
/// inherited by the inner `Text` per SwiftUI's environment rules.
struct LiveDeckTimeText: View {
    /// Which side of the time row this instance renders. Picks
    /// which field of `PositionInfo` to read and whether to
    /// prefix the rendered string with a `-` sign (per the
    /// `DeckTimeFormat.format(_:signed:)` convention).
    enum Slot {
        /// Wall-clock seconds from track start; rendered as
        /// `"01:23"` (unsigned).
        case elapsed
        /// Wall-clock seconds remaining until track end; rendered
        /// as `"-02:22"` (signed). Mirrors what
        /// `DeckTimeFormat.format(_:signed: true)` produces
        /// elsewhere so the visual cue is unchanged from the
        /// pre-decoupling code path.
        case remaining
    }

    let engine: DubEngine
    let deckIdx: UInt64
    let slot: Slot

    var body: some View {
        // 2 Hz timeline is enough — the integer-second-floor of the
        // position only changes once a second, and a 0.5 s tick keeps
        // the M:SS rollover visually fresh without paying the cost of
        // a per-display-refresh closure body re-eval. The Apple shell
        // never asks for sub-second precision on this surface (per the
        // M11d.5 round 3 user sign-off: "nothing is relevant sub
        // seconds" on the deck-header time display).
        TimelineView(.periodic(from: .now, by: 0.5)) { _ in
            let pos = engine.positionSnapshot(deckIdx: deckIdx)
            Text(formattedText(for: pos))
        }
    }

    private func formattedText(for pos: PositionInfo) -> String {
        // Defensive against unloaded / cold-launch decks where
        // `has_track` is false: the engine reports zero for both
        // fields, which renders as "00:00" / "-00:00". Same
        // behaviour as the historical pre-decoupling path.
        switch slot {
        case .elapsed:
            return DeckTimeFormat.format(pos.elapsedSecs)
        case .remaining:
            return DeckTimeFormat.format(pos.remainingSecs, signed: true)
        }
    }
}

// MARK: - Derivation from DeckState

extension DeckHeaderState {

    /// Build a header state from the model's per-deck snapshot plus
    /// the engine-wide flags. Keeps all derivation in one place so
    /// the view stays declarative.
    ///
    /// `prepMode` controls the time-row variant (M10.5r): Prep mode
    /// gets `elapsedAndRemaining`, Performance mode gets
    /// `remainingOnly`. The DJ asked for the minimal "-MM:SS" cue
    /// in Performance because the two-deck split is space-tight,
    /// and the full elapsed-vs-remaining split in Prep because the
    /// single-deck rehearsal surface has the screen real-estate.
    /// Resolve the row-3 Internal/Timecode switch state from the deck's
    /// telemetry. `nil` (switch hidden) when the deck has no timecode
    /// input. Calibration in progress wins; otherwise the mode + class.
    static func sourceControl(from d: DeckState) -> SourceControlStatus? {
        guard d.hasTimecodeInput else { return nil }
        // Reflect the user-selected mode directly (0 internal · 1
        // timecode · 2 thru). Calibration is a transient sub-state of
        // Timecode. No auto-detection / `detecting` state any more.
        switch d.controlMode {
        case 0: return .internalPlay
        case 2: return .thru
        default: return d.calibrating ? .calibrating : .timecode
        }
    }

    static func from(
        side: DeckSide,
        deckState: DeckState,
        engineRunning: Bool,
        deckEnabled: Bool,
        thruMode: Bool,
        isMaster: Bool,
        prepMode: Bool
    ) -> DeckHeaderState {
        guard engineRunning, deckEnabled else { return .idle }

        // Title comes from container tag metadata when present,
        // falling back to the file stem (DeckState.displayName) so
        // an untagged file still reads as "what did I just load".
        // Artist is tag-only — no "Artist Unknown" placeholder; the
        // header just hides the chip on untagged files.
        let resolvedTitle = deckState.trackTitle ?? deckState.displayName
        let resolvedArtist = deckState.trackArtist

        // M10.5d: cold load (no previous track) — render the
        // header with the new title + LOADING pill but no time row
        // (duration is unknown until decode completes). The
        // transport-toggle is gated off until `hasTrack` flips
        // true.
        if deckState.isLoading, !deckState.hasTrack {
            return DeckHeaderState(
                isLive: true,
                source: .loading,
                trackTitle: resolvedTitle,
                trackArtist: nil,
                bpm: nil,
                key: nil,
                formatChip: nil,
                timeRow: nil,
                isMaster: isMaster,
                isPlaying: false,
                isPanicPlay: false,
                useTimecodeToggle: false,
                gridLocked: false, gridDriftQuality: nil)
        }

        if deckState.hasTrack {
            // M11d.5 round 5: time-row payload is a layout selector
            // only — the actual M:SS strings come from
            // `LiveDeckTimeText` inside the header, reading
            // `engine.position(deckIdx:)` on its own timeline. See
            // `TimeRow`'s doc comment.
            let time: DeckHeaderState.TimeRow =
                prepMode ? .elapsedAndRemaining : .remainingOnly
            // M10.6c: in Timecode mode + Panic Play engaged, the
            // source pill flips from FILE → TC · HOLD (PRD §6.1.2).
            // M10.5d: a replace-load (new file decoded while the
            // previous one is still resident) shows the LOADING
            // pill but keeps the old time row + transport-toggle
            // available — the previous track stays audible /
            // visible until the new peaks swap in at decode
            // completion (one frame after the engine bumps
            // `peak_generation_seq`).
            let inPanic = thruMode && deckState.isPanicPlay
            let source: Source
            if deckState.isLoading {
                source = .loading
            } else if inPanic {
                source = .tcHold
            } else {
                source = .file
            }
            return DeckHeaderState(
                isLive: true,
                source: source,
                trackTitle: resolvedTitle,
                trackArtist: resolvedArtist,
                bpm: deckState.bpm,
                pitchPercent: deckState.pitchPercent,
                pitchSettled: deckState.pitchSettled,
                measureProgress: deckState.measureProgress,
                timecodeLockState: deckState.timecodeLockState,
                sourceControl: Self.sourceControl(from: deckState),
                sourceControlOverridden: deckState.controlOverridden,
                key: deckState.key,
                formatChip: deckState.formatChip,
                timeRow: time,
                isMaster: isMaster,
                isPlaying: deckState.isPlaying,
                isPanicPlay: inPanic,
                // M11d.5: primary transport button is always
                // plain Play / Pause. Panic Play state is folded
                // into the Play action in the model. See
                // `useTimecodeToggle` doc comment for rationale.
                useTimecodeToggle: false,
                gridLocked: deckState.gridLocked,
                gridDriftQuality: deckState.gridDriftQuality,
                historyHint: deckState.historyHint)
        }

        if thruMode {
            // Timecode engine mode + no file loaded yet. The deck is
            // armed: show the source switch (INT / TC / THRU) so the DJ
            // can pick a mode — including THRU to pass a real record
            // straight through — before loading anything. No transport
            // glyphs (those need a file); the standalone switch row
            // renders via `sourceSwitchRow`.
            return DeckHeaderState(
                isLive: true,
                source: .timecode,
                trackTitle: nil,
                trackArtist: nil,
                bpm: nil,
                timecodeLockState: deckState.timecodeLockState,
                sourceControl: Self.sourceControl(from: deckState),
                sourceControlOverridden: deckState.controlOverridden,
                key: nil,
                formatChip: nil,
                timeRow: nil,
                isMaster: isMaster,
                isPlaying: false,
                isPanicPlay: false,
                useTimecodeToggle: false,
                gridLocked: false, gridDriftQuality: nil)
        }

        return DeckHeaderState(
            isLive: false,
            source: .off,
            trackTitle: nil,
            trackArtist: nil,
            bpm: nil,
            key: nil,
            formatChip: nil,
            timeRow: nil,
            isMaster: false,
            isPlaying: false,
            isPanicPlay: false,
            useTimecodeToggle: false,
            gridLocked: false, gridDriftQuality: nil)
    }
}

#Preview("Deck A — idle") {
    DeckHeader(side: .a, state: .idle)
        .frame(width: 720)
        .background(DubColor.surface0)
        .padding()
}

#Preview("Deck A — live Thru, master") {
    DeckHeader(side: .a, state: DeckHeaderState(
        isLive: true, source: .thru,
        trackTitle: "Real Record", trackArtist: "capturing live",
        bpm: nil, key: nil,
        formatChip: nil, timeRow: nil,
        isMaster: true, isPlaying: false,
        isPanicPlay: false, useTimecodeToggle: false,
        gridLocked: false, gridDriftQuality: nil))
        .frame(width: 720)
        .background(DubColor.surface0)
        .padding()
}

#Preview("Deck B — File, mid-track (Performance, mirrored)") {
    DeckHeader(side: .b, state: DeckHeaderState(
        isLive: true, source: .file,
        trackTitle: "Stakes Is High",
        trackArtist: "De La Soul",
        bpm: 92.5, key: "8B",
        formatChip: "MP3 · 44.1 kHz · stereo",
        timeRow: .remainingOnly,
        isMaster: false, isPlaying: true,
        isPanicPlay: false, useTimecodeToggle: true,
        gridLocked: true, gridDriftQuality: nil),
        mirrored: true)
        .frame(width: 720)
        .background(DubColor.surface0)
        .padding()
}

#Preview("Deck A — File, mid-track (Prep)") {
    DeckHeader(side: .a, state: DeckHeaderState(
        isLive: true, source: .file,
        trackTitle: "Stakes Is High",
        trackArtist: "De La Soul",
        bpm: 92.5, key: "8B",
        formatChip: "MP3 · 44.1 kHz · stereo",
        timeRow: .elapsedAndRemaining,
        isMaster: true, isPlaying: true,
        isPanicPlay: false, useTimecodeToggle: false,
        gridLocked: false, gridDriftQuality: nil))
        .frame(width: 720)
        .background(DubColor.surface0)
        .padding()
}

#Preview("Deck B — Timecode, Panic Play engaged (mirrored)") {
    DeckHeader(side: .b, state: DeckHeaderState(
        isLive: true, source: .tcHold,
        trackTitle: "Stakes Is High",
        trackArtist: nil,
        bpm: 92.5, key: "8B",
        formatChip: "MP3 · 44.1 kHz · stereo",
        timeRow: .remainingOnly,
        isMaster: true, isPlaying: true,
        isPanicPlay: true, useTimecodeToggle: true,
        gridLocked: false, gridDriftQuality: 4.5),
        mirrored: true)
        .frame(width: 720)
        .background(DubColor.surface0)
        .padding()
}
