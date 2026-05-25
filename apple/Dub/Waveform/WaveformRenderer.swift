//
//  WaveformRenderer.swift
//  Dub
//
//  Metal renderer for the deck waveform — Serato-faithful baseline.
//
//  Owns:
//    • `MTLDevice` + `MTLCommandQueue`
//    • one render pipeline state (vertex + fragment from `Shaders.metal`)
//    • triple-buffered uniforms (one slot per inflight frame, two
//      regions packed per slot for the past + future draws)
//    • two append-only ring buffers — `chunks` (broadband peaks)
//      and `bandChunks` (8 × f32 RMS per FFT hop) — written each
//      frame from `DubEngine.peaksExtend` / `bandPeaksExtend`
//
//  Renders directly to the MTKView drawable in a single render pass.
//  No HDR, no bloom, no tonemap, no onset confidence, no filtered-
//  peaks gate: that was the M10.5h–p stack which made kicks read
//  worse than Serato. The Rust DSP machinery for those features
//  (onset detection in `dub-peaks`, LF/MF/HF filter in
//  `dub-peaks::filtered`) is still alive for a future polish phase —
//  this renderer simply doesn't bind it.
//
//  Threading: all renderer work runs on the main thread. `MTKView`
//  invokes `draw(in:)` on the main thread when `isPaused == false`
//  and `enableSetNeedsDisplay == false` (our configuration in
//  `WaveformView`).
//

import AppKit
import Foundation
import Metal
import MetalKit
import simd

import DubCore

/// CPU-side mirror of `Shaders.metal`'s `Uniforms` struct. Field
/// order, type, and padding match exactly so we `memcpy` it into
/// the uniforms buffer with no per-frame allocation.
///
/// Eleven 4-byte fields = 44 bytes total. Padded out to 64 bytes by
/// the per-region stride below (see `uniformStridePerRegion`).
private struct WaveformUniforms {
    /// First *raw* broadband chunk for this region (ring offset).
    /// The shader multiplies `chunkInWindow × chunksPerColumn` and
    /// adds this base to address its aggregation window.
    var chunkOffset: UInt32
    /// Number of *drawn columns* in this region's strip. Each
    /// drawn column emits 2 vertices and aggregates
    /// `chunksPerColumn` raw chunks under the hood.
    var chunksVisible: UInt32
    /// `> 0` ⇒ past region draw; `0` ⇒ future region draw. Mirrors
    /// `chunksAbove` on the host so the shader can pick the right
    /// time-→-NDC mapping without an extra flag.
    var chunksAbovePlayhead: UInt32
    var yScale: Float
    var samplesPerPeakChunk: UInt32
    /// First *raw* band chunk for this region (ring offset).
    var bandChunkOffset: UInt32
    var samplesPerBandChunk: UInt32
    var bandCapacity: UInt32
    /// 0 = vertical (PRD §9.1 default), 1 = horizontal (Prep mode).
    var orientation: UInt32
    /// Raw broadband chunks aggregated into one drawn column. ≥ 1.
    /// Set to `chunksPerPixel` (= 2) so each drawn column maps to
    /// exactly one drawable pixel along the time axis — the
    /// trapezoidal strip slices are then ≥ 1 px tall and don't
    /// stair-step into a sub-pixel comb pattern.
    var chunksPerColumn: UInt32
    /// **Band-slot phase offset, in samples**, of the visible
    /// region's first peak chunk relative to its containing band
    /// chunk's left edge. Computed as
    /// `(pastFirstGlobalChunk × samplesPerPeakChunk) mod
    /// samplesPerBandChunk` (the Euclidean-mod variant for
    /// negative empty-groove indices).
    ///
    /// The shader uses this to compute the correct band slot for
    /// any column inside the region:
    ///
    ///   bandIdx = bandChunkOffset
    ///           + (bandStartPhaseSamples + localSampleIdx)
    ///             / samplesPerBandChunk
    ///
    /// Without this field, columns past the first
    /// `(samplesPerBandChunk − bandStartPhaseSamples) /
    /// samplesPerPeakChunk` boundary inside the visible region
    /// read the band slot *one earlier* than the one that actually
    /// contains them, which on consecutive frames produced the
    /// visible colour flicker on transients — the same audio
    /// position alternated between two band slots whose
    /// dominant-band ratios differed.
    var bandStartPhaseSamples: UInt32
    /// **Sub-chunk geometry shift, in NDC units**, that slides the
    /// entire region's vertices to compensate for the chunk-pair
    /// snap of `playheadChunkSigned`.
    ///
    /// **Motivation.** The data ring is addressed by integer peak-
    /// chunk indices, and `playheadChunkSigned` is pair-snapped so
    /// the column-to-chunk pair stays stable across frames (needed
    /// for stable peak-amplitude aggregation). The natural side
    /// effect is that the displayed playhead moves in integer-chunk
    /// jumps of 0/2/4/... per frame, and at 60 fps + 44.1 kHz the
    /// per-frame jump alternates 10 → 12 → 10 → 12 chunks (5/6
    /// columns each). The eye reads that 5/6-column-per-frame
    /// stepped motion as a low-refresh-rate scroll even though
    /// the Metal pipeline runs at the full display refresh.
    ///
    /// **What this offset does.** The host computes the *continuous*
    /// playhead chunk (`pos.playheadSecsUnclamped / peakDur`) and
    /// subtracts the snapped integer playhead, yielding a fractional
    /// ε ∈ [0, chunksPerColumn). Converted into NDC for this region
    /// (using its own `chunksVisible`/`NDC span` ratio so past and
    /// future shift by the same physical amount), this offset is
    /// added to every vertex's `timeNDC` in the shader. Net effect:
    /// the data still snaps to integer-chunk positions inside the
    /// ring (so colours and amplitude are stable, see the bandStart
    /// PhaseSamples field for the colour story) but the *visible*
    /// position of every chunk is rendered at its true continuous
    /// playhead-relative location. Per-frame visual advance becomes
    /// the constant continuous rate (~5.75 logical pixels at 60 fps
    /// + 44.1 kHz on a Retina deck column) instead of alternating
    /// 5 ↔ 6 logical-pixel jumps, which is what the eye wants for
    /// "smooth at display refresh rate" scrolling.
    var subChunkOffsetNDC: Float
}

/// Renderer orientation. Vertical is the Performance-mode default
/// (PRD §9.1, time → y, playhead 25 % from the top); horizontal is
/// Prep mode (M10.8, time → x, playhead 25 % from the left).
public enum WaveformOrientation: UInt32 {
    case vertical = 0
    case horizontal = 1
}

/// Visible palette options. Collapsed to a single Serato-faithful
/// look for the post-strip-down baseline. The enum is kept (vs.
/// removing the field entirely from the public API) so a future
/// polish phase can add variants back without churning every call
/// site that passes a `WaveformPalette` parameter through.
public enum WaveformPalette: UInt32, CaseIterable, Identifiable {
    case serato = 0

    public var id: UInt32 { rawValue }
    public var displayName: String { "Serato-faithful" }
}

/// **B-24 / M11d.5 — shared synchronization snapshot between the
/// Metal renderer and the SwiftUI beat-grid overlay.**
///
/// Two independent draw paths used to walk their own way to the
/// engine for state (`engine.position`, `engine.peaksChunkDuration
/// Secs`, `engine.beatGrid`):
///
///   • The Metal renderer reads them inside `WaveformRenderer.draw`
///     from MTKView's `CADisplayLink` callback.
///   • The SwiftUI Canvas overlay reads them inside `drawBeatGrid`
///     from a TimelineView tick.
///
/// CADisplayLink and TimelineView are tied to the same display
/// refresh rate, but their *phase* inside the main-thread runloop
/// is unspecified and may alternate frame-to-frame. When they
/// alternate, the two layers end up reading `playheadSecsUnclamped`
/// at points one chunk apart in audio time, and the beat ticks
/// wobble against the waveform by ±1 chunk = ±0.5 logical pixel
/// at 60 fps on Retina. Snapping the overlay's playhead to the
/// chunk grid (the prior fix) made each layer self-consistent
/// frame-by-frame but didn't resolve the inter-layer disagreement.
///
/// This snapshot makes the renderer the sole source of truth for
/// the values both layers care about. After computing `chunkF` for
/// a Metal draw the renderer writes `lastDrawnPlayheadSecsUn
/// clamped`, `peakDurSecs`, `hasTrack`, and (on track-load
/// generation bumps) the cached `beats` / `beatsPerBar` /
/// `beatsConfidence`. The Canvas reads from the snapshot only —
/// it never calls back into the engine. Worst case the Canvas
/// renders one Metal-frame stale; that is a *constant* one-frame
/// offset, not a wobbling 0-or-1-frame offset, and 16.67 ms at
/// 60 fps is far below the visual jitter threshold.
///
/// **Why the beats are cached here too.** Every Canvas frame used
/// to call `engine.beatGrid(deckIdx:)`, which clones a `Vec<f64>`
/// of up to ~900 beat positions across the UniFFI boundary. At
/// 60 Hz that's ~280 KB/s of allocation churn on the same main
/// thread that drives the Metal pipeline. Folding the beats into
/// the snapshot collapses the FFI traffic to "once per track load"
/// (gated on the engine's peaks-generation counter) and removes
/// a noticeable contributor to the per-frame work, which the user
/// observed as the waveform "blinking/jittering a bit" on Prep-
/// mode horizontal layout.
///
/// Race safety: every read and write is `@MainActor`; the main
/// actor is single-threaded so the writes done by Metal's `draw`
/// and the reads done by SwiftUI's Canvas closure cannot
/// interleave. We deliberately do *not* `@Published` any of the
/// fields — the consumer (Canvas inside a `TimelineView`) re-runs
/// every refresh-tick on its own, so a property-wrapper-driven
/// re-render would cost a redundant pass per frame.
@MainActor
final class WaveformRenderSnapshot: ObservableObject {
    /// Continuous, sample-accurate playhead in track-seconds, as
    /// captured by the most recent `WaveformRenderer.draw` call.
    /// `0` if no draw has run yet.
    ///
    /// Holds `pos.playheadSecsUnclamped` directly — not the
    /// chunk-pair-snapped variant the renderer uses to address
    /// peak columns. The snap is an *addressing* choice (which
    /// chunk-pair the column shader reads); the *visible* playhead
    /// in the Metal output is the snap plus the continuous
    /// `subChunkOffsetNDC` slide, which together resolve to this
    /// same `pos.playheadSecsUnclamped`. Publishing the continuous
    /// value here lets the SwiftUI beat-grid overlay's
    /// `(beat - playhead) / secsPerLogicalPx` math land on the
    /// exact pixel the renderer's geometry slides to, so the
    /// grid lines and the waveform move byte-identically rather
    /// than the grid drifting in chunk-pair steps. See the
    /// snapshot-write block in `WaveformRenderer.draw` for the
    /// matched comment.
    var lastDrawnPlayheadSecsUnclamped: Double = 0
    /// Peak-chunk duration in seconds. Cadenced in **track**
    /// sample-rate frames per the engine — see
    /// `WaveformRenderer.peakChunkDurationSecs`. `0` until the
    /// first non-empty peak ingest reports a cadence.
    var peakDurSecs: Double = 0
    /// Whether the deck reports a loaded file source. Mirrors
    /// `engine.position(deckIdx:).hasTrack`. Drives the early-out
    /// in the Canvas overlay.
    var hasTrack: Bool = false
    /// Beats in track-seconds from sample 0. Refreshed when the
    /// renderer observes a peaks-generation bump (= a fresh
    /// `loadTrack`). Empty when no track is loaded or the BPM
    /// estimator returned `confidence == 0`.
    var beats: [Double] = []
    /// Beats per bar from the BPM estimator. v0 always reports
    /// `4` but a future time-signature-aware estimator may emit
    /// other values; the overlay's downbeat phase reads this
    /// rather than hard-coding 4.
    var beatsPerBar: Int = 4
    /// PRD-BEATS C2 (round 4) — explicit bar-phase scalar. The
    /// downbeat is the beat at index `i` such that
    /// `i % beatsPerBar == barPhase`. Mirrors the Rust
    /// `dub_bpm::BeatGrid::bar_phase` field. Updated on every
    /// `set the 1` tap without moving `beats[]`, so the grid
    /// lines stay locked to the audible kick while the yellow
    /// downbeat rotates with the user's tap.
    var barPhase: Int = 0
    /// `confidence > 0` ⇒ draw the grid; `confidence == 0` ⇒ the
    /// estimator didn't lock and the overlay paints nothing
    /// (B-24 spec point 4).
    var beatsConfidence: Float = 0
    /// `peaksGeneration` value at the time `beats` was refreshed.
    /// Compared against the engine's current generation on every
    /// draw; a mismatch triggers a re-fetch.
    var beatsGeneration: UInt64 = 0
}

/// 12-byte mirror of `PeakChunk` for memory-layout assertions.
/// Generated UniFFI bindings return chunks as `Data`; we treat that
/// Data as `[PeakChunk]` via `withUnsafeBytes(_:)`.
private struct PeakChunkLayout {
    var minSample: Float
    var maxSample: Float
    var rms: Float
}

/// 32-byte mirror of `BandPeakChunk`. Matches
/// `#[repr(C)] pub struct BandPeakChunk { pub rms_per_band: [f32; 8] }`.
private struct BandPeakChunkLayout {
    var b0: Float; var b1: Float; var b2: Float; var b3: Float
    var b4: Float; var b5: Float; var b6: Float; var b7: Float
}

/// CPU-side mirror of `Shaders.metal`'s `BeatGridVertex`.
private struct BeatGridVertexLayout {
    var position: SIMD2<Float>
    var color: SIMD4<Float>
}

@MainActor
final class WaveformRenderer: NSObject {

    // MARK: Configuration

    /// Maximum broadband / band chunks copied from the engine into
    /// the GPU ring per draw call. A whole-track ingest after
    /// `reset()` was blocking scrub clicks for hundreds of ms on
    /// load / jump; the visible column only needs the neighbourhood
    /// around the playhead anyway.
    static let maxChunksIngestPerFrame: Int = 8192

    /// Power-of-two number of broadband chunks the GPU ring buffer
    /// can hold. 2^20 ≈ 23 min of audio at 48 kHz / 64-sample
    /// chunks — sized so the entire offline-decoded peak set of
    /// any realistically-long DJ track fits without head-wrap
    /// collisions during a seek back to start. Power-of-two so the
    /// shader's modulo compiles to a bitmask. **Keep in sync with
    /// the `(1048576u - 1u)` mask in `Shaders.metal`.**
    static let chunkCapacity: Int = 1_048_576

    /// Power-of-two number of band chunks. 2^17 → ~1 400 s at
    /// 48 kHz / 512-sample band chunks, matching the broadband
    /// ring's coverage.
    static let bandChunkCapacity: Int = 131_072

    /// Standard Metal "three frames in flight" CPU queue depth.
    static let maxFramesInFlight: Int = 3

    /// Amplitude scale in NDC. 0.95 leaves a small gutter so peaks
    /// don't kiss the deck-column edge.
    private static let yScale: Float = 0.95

    /// Fraction of the deck column reserved for the *past* region
    /// (above the playhead per PRD §9.1).
    static let pastRegionFraction: Double = 0.25

    /// Raw broadband chunks per drawable pixel along the time axis.
    /// ~2.67 ms / px at 48 kHz / 64-sample chunks → a typical
    /// ~640 px-tall deck column shows ≈ 1.7 s of audio.
    nonisolated private static let chunksPerPixel: Double = 2.0

    /// Raw chunks aggregated into one drawn column. Set equal to
    /// `chunksPerPixel` so the geometry emits one trapezoidal
    /// slice per drawable pixel — the Mixxx-style per-pixel `max()`
    /// over a `chunksPerColumn`-sized data window happens in the
    /// vertex shader. This eliminates the sub-pixel comb pattern
    /// the un-aggregated strip produced when the raw chunk
    /// cadence is finer than 1 chunk per pixel.
    ///
    /// **Public for the B-24 beat-grid overlay** so it can mirror
    /// the renderer's exact secs-per-drawable-pixel cadence without
    /// duplicating the constant. Any zoom-level change must update
    /// both this constant *and* `pixelsPerDrawnColumn` together so
    /// the overlay's `drawBeatGrid` stays in lockstep with the
    /// Metal pipeline.
    nonisolated public static let chunksPerColumn: UInt32 = 2

    /// Drawable pixels spanned by one drawn column along the time
    /// axis. M10.5f set this to 2 (the "2× zoom-in") so each
    /// trapezoidal slice covers 2 drawable pixels and the total
    /// visible time halves to ≈ 0.93 s (≈ 2 beats at 128 BPM).
    /// Mirror of the `pixelsPerDrawnColumn` local inside
    /// `draw(in:)`; promoted to a static so the B-24 beat-grid
    /// overlay (`WaveformView.drawBeatGrid`) can derive the same
    /// secs-per-drawable-pixel cadence the Metal pipeline produces.
    /// **Keep in sync with the local in `draw(in:)`** — the local
    /// is preserved for code locality next to its consumer; this
    /// static is the public contract.
    nonisolated public static let pixelsPerDrawnColumn: Int = 2

    /// Prep-mode horizontal waveform shows 20 % more audio than the
    /// performance default (`timeAxisZoom = 1.0`). Values > 1.0
    /// divide `pixelsPerDrawnColumn` so more drawn columns fit in
    /// the same viewport.
    nonisolated public static let prepModeTimeAxisZoom: Double = 1.2

    /// Effective drawable pixels per drawn column for a given
    /// time-axis zoom. Values > 1.0 zoom out (more seconds visible).
    nonisolated public static func effectivePixelsPerDrawnColumn(
        timeAxisZoom: Double
    ) -> Double {
        let zoom = max(1.0, timeAxisZoom)
        return max(1.0, Double(pixelsPerDrawnColumn) / zoom)
    }

    /// Default broadband samples-per-chunk emitted by `dub-peaks`'s
    /// stream tap (M9.5b). Used for the host-side gesture-→-secs
    /// helper; the actual cadence the renderer uses is captured
    /// lazily from the first non-empty FFI payload.
    nonisolated public static let defaultSamplesPerPeakChunk: UInt32 = 64

    /// 4× MSAA on the drawable. The waveform geometry is a stack of
    /// trapezoid slices with sub-pixel edge slopes at high zoom;
    /// MSAA stops them stair-stepping into a "venetian blind"
    /// pattern. 4 samples is cheap on Apple Silicon.
    nonisolated public static let sampleCount: Int = 4

    /// Audio seconds represented by one pixel along the time axis,
    /// given the engine's current sample rate. Mirror of the
    /// renderer's `chunksPerPixel × samplesPerPeakChunk / sampleRate`
    /// formula so a click-scrub gesture lands on the same chunk
    /// the user clicked.
    nonisolated public static func secsPerPixel(
        sampleRate: UInt32,
        samplesPerPeakChunk: UInt32 = defaultSamplesPerPeakChunk
    ) -> Double {
        let sr = max(1.0, Double(sampleRate))
        return chunksPerPixel * Double(samplesPerPeakChunk) / sr
    }

    /// Byte stride between the past-region and future-region
    /// uniform slots inside one per-frame uniform buffer. The
    /// `Uniforms` struct itself is 36 bytes naturally; we round to
    /// 64 to satisfy the 32-byte `setVertexBuffer(offset:)`
    /// constant-buffer alignment Metal guarantees on every Apple
    /// GPU family.
    nonisolated public static let uniformStridePerRegion: Int = 64

    // MARK: Dependencies

    let device: MTLDevice
    private let commandQueue: MTLCommandQueue

    /// Single render pipeline: `waveformVertex` + `waveformFragment`
    /// writing straight to the MTKView's `bgra8Unorm` drawable
    /// (with MSAA resolve).
    private let waveformPipeline: MTLRenderPipelineState

    /// Alpha-blended line quads for the B-24 beat grid. Drawn in
    /// the same render pass after the waveform geometry so the
    /// ticks share the waveform's exact NDC mapping without a
    /// second SwiftUI Canvas on the main thread.
    private let beatGridPipeline: MTLRenderPipelineState

    /// Dynamic upload buffer for beat-grid tick quads (6 vertices
    /// per tick × up to a few dozen visible ticks per frame).
    private var beatGridVertexBuffer: MTLBuffer?
    private var beatGridVertexCapacity: Int = 0

    /// Reused each frame to avoid `[BeatGridVertexLayout]` heap
    /// churn on the main thread during playback.
    private var beatGridScratchVertices: [BeatGridVertexLayout] = []

    /// Bounded queue depth via semaphore. Prevents the CPU from
    /// writing into a uniform buffer the GPU is still reading.
    private let inflightSemaphore = DispatchSemaphore(value: maxFramesInFlight)

    /// Triple-buffered uniforms — one slot per inflight frame, two
    /// regions (past + future) packed per buffer at offsets 0 and
    /// `uniformStridePerRegion`.
    private var uniformBuffers: [MTLBuffer] = []
    private var uniformIndex: Int = 0

    /// Append-only ring buffer of `PeakChunk`s. Shared storage so
    /// we memcpy directly from the FFI `Data` blob — zero-copy on
    /// Apple Silicon.
    private let chunksBuffer: MTLBuffer

    /// Append-only ring buffer of `BandPeakChunk`s. Parallel to
    /// `chunksBuffer`; the vertex shader looks up the matching band
    /// chunk for each broadband instance.
    private let bandChunksBuffer: MTLBuffer

    // MARK: Engine binding

    private let engine: DubEngine
    private let deckIdx: UInt64

    /// Cached `peaks_len()` from the previous poll.
    private var lastSeenPeaksLen: UInt64 = 0

    /// Cached `band_peaks_len()` from the previous poll. Tracked
    /// independently because the two streams advance at different
    /// cadences (one band chunk per 8 broadband chunks).
    private var lastSeenBandPeaksLen: UInt64 = 0

    /// Cached `peaks_generation()` from the previous poll. When the
    /// engine swaps a deck's `PeakSource` (Thru → File on load,
    /// File → File on reload) this bumps; the renderer wipes its
    /// ring + cadence cache and re-ingests from chunk 0.
    private var lastSeenPeaksGeneration: UInt64 = 0

    /// Cached chunk cadences (samples per chunk). Read once on the
    /// first non-empty poll; broadband / band lookup ratio in the
    /// shader depends on these.
    ///
    /// **Unit warning**: peak chunks are originally cadenced in
    /// **track** frames (e.g. 64 frames at 44.1 kHz). When the
    /// engine SR ≠ track SR, `round(peakDurSecs × engineSR)`
    /// introduces a ~0.5 %-per-chunk systematic error that
    /// compounds over the track length. We avoid that by using
    /// `peakChunkDurationSecs` (f64, exact) directly for the
    /// cumulative `elapsed_secs → playhead_chunk` mapping. The
    /// integer fields below are kept only for the band cross-ref
    /// math, which sees small visible-region chunk indices where
    /// the rounded error stays imperceptible.
    private var samplesPerPeakChunk: UInt32 = 64
    private var samplesPerBandChunk: UInt32 = 512

    /// Real-time duration of one broadband peak chunk in seconds —
    /// the **exact** value as reported by the engine. The
    /// authoritative source for `elapsed_secs → playhead_chunk`.
    private var peakChunkDurationSecs: Double = 0.0

    /// Active palette. Single-valued in the post-strip-down baseline
    /// but kept as a property so a future polish phase can add
    /// branches without re-plumbing the view.
    var palette: WaveformPalette = .serato

    /// Orientation. `.vertical` is Performance mode (PRD §9.1);
    /// `.horizontal` is Prep mode (M10.8).
    var orientation: WaveformOrientation = .vertical

    /// Shared draw snapshot for legacy SwiftUI overlay consumers.
    /// The Metal beat grid does not depend on this object.
    var renderSnapshot: WaveformRenderSnapshot?

    /// Which deck column this renderer belongs to. Drives beat-grid tint.
    var side: DeckSide = .a

    /// Time-axis zoom multiplier. `1.0` = performance default;
    /// `prepModeTimeAxisZoom` (1.2) shows 20 % more audio in prep.
    var timeAxisZoom: Double = 1.0

    /// When `true`, beat ticks render in the Metal pass after waveform
    /// geometry each frame.
    var beatGridEnabled: Bool = true

    /// Cached beat grid from the engine. Refreshed on peaks-generation
    /// bumps and until `confidence > 0` latches.
    private var cachedBeats: [Double] = []
    private var cachedBpm: Double = 0
    private var cachedBeatsPerBar: Int = 4
    /// PRD-BEATS C2 (round 4) — see `WaveformRenderSnapshot.barPhase`.
    private var cachedBarPhase: Int = 0
    private var cachedBeatsConfidence: Float = 0
    private var cachedBeatsGeneration: UInt64 = 0
    private var cachedBeatGridGeneration: UInt64 = 0

    /// Countdown between `engine.beatGrid` FFI calls while analysis
    /// is still pending. Avoids cloning the beat `Vec` at 60 Hz.
    private var beatGridFetchCooldown: Int = 0

    private var debugLastFrameLogUptime: TimeInterval = 0
    private var debugLastVisibleBeatLogUptime: TimeInterval = 0
    private var debugLastGridSummaryLogUptime: TimeInterval = 0
    private var debugLastGridStabilityLogUptime: TimeInterval = 0
    private var debugStableBeatIdx: Int?
    private var debugStableBeatPixel: Double = 0
    private var debugStableBeatPlayheadSecs: Double = 0
    private var debugBeatPixelMaxError: Double = 0
    private var debugBeatPixelLastError: Double = 0
    private var debugBeatPixelErrorSamples: Int = 0

    /// Frames since the last peak-source swap. Beat-grid Metal
    /// work is skipped briefly so the first post-load scrubs aren't
    /// queued behind tick vertex generation.
    private var framesSinceSourceSwap: Int = 0

    /// Skip redundant FFI + `memcpy` when the playhead hasn't
    /// crossed a chunk boundary since the last frame.
    private var lastBroadbandIngestPhChunk: UInt64 = UInt64.max
    private var lastBandIngestPhChunk: UInt64 = UInt64.max

    /// Set when a draw is skipped because the GPU queue is full;
    /// the completion handler schedules one catch-up repaint.
    private var pendingRedraw = false

    // MARK: Init

    init(device: MTLDevice, engine: DubEngine, deckIdx: UInt64 = 0) throws {
        self.device = device
        self.engine = engine
        self.deckIdx = deckIdx

        guard let queue = device.makeCommandQueue() else {
            throw NSError(
                domain: "WaveformRenderer", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Metal command queue allocation failed"])
        }
        self.commandQueue = queue
        self.commandQueue.label = "dub.waveform.cmdqueue"

        let library: MTLLibrary
        do {
            library = try device.makeDefaultLibrary(bundle: Bundle.main)
        } catch {
            throw NSError(
                domain: "WaveformRenderer", code: 2,
                userInfo: [
                    NSLocalizedDescriptionKey: "Default Metal library load failed: \(error)"
                ])
        }
        guard let vertexFn = library.makeFunction(name: "waveformVertex"),
              let fragmentFn = library.makeFunction(name: "waveformFragment")
        else {
            throw NSError(
                domain: "WaveformRenderer", code: 3,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        "Metal functions (waveformVertex / waveformFragment) not found in default library"
                ])
        }

        // Single pass: waveform → MTKView drawable (bgra8Unorm,
        // 4× MSAA). MTKView allocates the multisample texture
        // when its `sampleCount` matches this pipeline's
        // `rasterSampleCount` and `framebufferOnly == false`.
        let waveformDescriptor = MTLRenderPipelineDescriptor()
        waveformDescriptor.label = "dub.waveform.pipeline"
        waveformDescriptor.vertexFunction = vertexFn
        waveformDescriptor.fragmentFunction = fragmentFn
        waveformDescriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
        waveformDescriptor.colorAttachments[0].isBlendingEnabled = false
        waveformDescriptor.rasterSampleCount = WaveformRenderer.sampleCount
        self.waveformPipeline = try device.makeRenderPipelineState(
            descriptor: waveformDescriptor)

        guard let beatGridVertexFn = library.makeFunction(name: "beatGridVertex"),
              let beatGridFragmentFn = library.makeFunction(name: "beatGridFragment")
        else {
            throw NSError(
                domain: "WaveformRenderer", code: 6,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        "Metal functions (beatGridVertex / beatGridFragment) not found"
                ])
        }
        let beatGridDescriptor = MTLRenderPipelineDescriptor()
        beatGridDescriptor.label = "dub.beatgrid.pipeline"
        beatGridDescriptor.vertexFunction = beatGridVertexFn
        beatGridDescriptor.fragmentFunction = beatGridFragmentFn
        beatGridDescriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
        beatGridDescriptor.colorAttachments[0].isBlendingEnabled = true
        beatGridDescriptor.colorAttachments[0].rgbBlendOperation = .add
        beatGridDescriptor.colorAttachments[0].alphaBlendOperation = .add
        beatGridDescriptor.colorAttachments[0].sourceRGBBlendFactor = .sourceAlpha
        beatGridDescriptor.colorAttachments[0].destinationRGBBlendFactor = .oneMinusSourceAlpha
        beatGridDescriptor.colorAttachments[0].sourceAlphaBlendFactor = .one
        beatGridDescriptor.colorAttachments[0].destinationAlphaBlendFactor = .oneMinusSourceAlpha
        beatGridDescriptor.rasterSampleCount = WaveformRenderer.sampleCount
        self.beatGridPipeline = try device.makeRenderPipelineState(
            descriptor: beatGridDescriptor)

        let chunkBytes = WaveformRenderer.chunkCapacity * MemoryLayout<PeakChunkLayout>.stride
        guard let chunks = device.makeBuffer(length: chunkBytes, options: .storageModeShared)
        else {
            throw NSError(
                domain: "WaveformRenderer", code: 4,
                userInfo: [NSLocalizedDescriptionKey: "Chunks MTLBuffer allocation failed"])
        }
        chunks.label = "dub.waveform.chunks"
        chunks.contents().initializeMemory(as: UInt8.self, repeating: 0, count: chunkBytes)
        self.chunksBuffer = chunks

        let bandChunkBytes =
            WaveformRenderer.bandChunkCapacity * MemoryLayout<BandPeakChunkLayout>.stride
        guard let bandChunks = device.makeBuffer(
            length: bandChunkBytes, options: .storageModeShared)
        else {
            throw NSError(
                domain: "WaveformRenderer", code: 4,
                userInfo: [
                    NSLocalizedDescriptionKey: "Band chunks MTLBuffer allocation failed"
                ])
        }
        bandChunks.label = "dub.waveform.bandChunks"
        bandChunks.contents().initializeMemory(
            as: UInt8.self, repeating: 0, count: bandChunkBytes)
        self.bandChunksBuffer = bandChunks

        let uniformStride = WaveformRenderer.uniformStridePerRegion
        let uniformBytesPerBuffer = uniformStride * 2
        var uniforms: [MTLBuffer] = []
        for idx in 0..<WaveformRenderer.maxFramesInFlight {
            guard let buf = device.makeBuffer(
                length: uniformBytesPerBuffer, options: .storageModeShared)
            else {
                throw NSError(
                    domain: "WaveformRenderer", code: 5,
                    userInfo: [NSLocalizedDescriptionKey: "Uniform MTLBuffer allocation failed"])
            }
            buf.label = "dub.waveform.uniforms[\(idx)]"
            uniforms.append(buf)
        }
        self.uniformBuffers = uniforms

        super.init()
    }

    /// Drop cached beat-grid state on source swap. GPU ring slots
    /// are overwritten on the next playhead-centred ingest — no
    /// full-buffer zero (that was ~16 MB on the main thread and
    /// caused the post-load scrub click lag).
    func reset() {
        lastSeenPeaksLen = 0
        lastSeenBandPeaksLen = 0
        lastSeenPeaksGeneration = 0
        samplesPerPeakChunk = 64
        samplesPerBandChunk = 512
        peakChunkDurationSecs = 0.0
        cachedBeats = []
        cachedBpm = 0
        cachedBeatsPerBar = 4
        cachedBarPhase = 0
        cachedBeatsConfidence = 0
        cachedBeatsGeneration = 0
        cachedBeatGridGeneration = 0
        beatGridFetchCooldown = 0
        debugLastFrameLogUptime = 0
        debugLastVisibleBeatLogUptime = 0
        debugLastGridSummaryLogUptime = 0
        debugLastGridStabilityLogUptime = 0
        debugStableBeatIdx = nil
        debugStableBeatPixel = 0
        debugStableBeatPlayheadSecs = 0
        debugBeatPixelMaxError = 0
        debugBeatPixelLastError = 0
        debugBeatPixelErrorSamples = 0
        framesSinceSourceSwap = 0
        lastBroadbandIngestPhChunk = UInt64.max
        lastBandIngestPhChunk = UInt64.max
        pendingRedraw = false
    }

    // MARK: MTKViewDelegate-style entry points

    func drawableSizeWillChange(_ size: CGSize) {
        _ = size
    }

    /// Per-frame work. Polls the engine for new chunks, uploads
    /// them into the rings, and records a single render pass to
    /// the MTKView drawable.
    func draw(in view: MTKView) {
        // Yield to queued clicks before touching the GPU semaphore.
        if Self.hasPendingPointerInput() {
            return
        }

        // Never block the main thread waiting for a prior frame to
        // retire. When the GPU is busy, mark one catch-up draw and
        // return — the completion handler repaints so we don't drop
        // every other vsync (visible stutter).
        if inflightSemaphore.wait(timeout: .now()) == .timedOut {
            pendingRedraw = true
            return
        }
        pendingRedraw = false
        let releaseSemaphore: () -> Void = { [weak self] in
            self?.inflightSemaphore.signal()
        }

        // 0. Detect a PeakSource swap on this deck. When the engine
        //    swaps Thru → File (drag-and-drop load) or File → File
        //    (reload) the per-deck generation counter bumps; we
        //    wipe the ring and re-ingest the new source from
        //    chunk 0. Doing this before the ingest pull is critical:
        //    the new source's chunk count is typically smaller than
        //    what we last observed from Thru, so the length-
        //    monotonicity check in `ingestNewChunks` would
        //    otherwise silently no-op and the renderer would keep
        //    drawing stale Thru capture forever.
        let currentGeneration = engine.peaksGeneration(deckIdx: deckIdx)
        let beatGridGeneration = engine.beatGridGeneration(deckIdx: deckIdx)
        if currentGeneration != lastSeenPeaksGeneration {
            reset()
            lastSeenPeaksGeneration = currentGeneration
        }
        framesSinceSourceSwap &+= 1

        // Read playhead once up front — drives playhead-centred
        // peak ingest so a post-load / post-seek scrub doesn't wait
        // for a whole-track GPU upload.
        let pos = engine.position(deckIdx: deckIdx)
        ingestNewChunks(playheadSecs: pos.playheadSecsUnclamped)
        ingestNewBandChunks(playheadSecs: pos.playheadSecsUnclamped)

        // 2. Compute the visible window.
        //
        // Piecewise layout: the time axis is whichever drawable
        // dimension time flows along. Vertical → height; Horizontal
        // → width. The playhead lives at 25 % from the leading edge
        // (top in vertical, left in horizontal) with a past region
        // covering 25 % of the axis and a future region the
        // remaining 75 %.
        let drawableSize = view.drawableSize
        let timeAxisPixels: Int
        switch orientation {
        case .vertical:
            timeAxisPixels = max(1, Int(drawableSize.height))
        case .horizontal:
            timeAxisPixels = max(1, Int(drawableSize.width))
        }
        let pastPixels =
            max(1, Int((Double(timeAxisPixels) * WaveformRenderer.pastRegionFraction).rounded()))
        let futurePixels =
            max(0, Int((Double(timeAxisPixels)
                * (1.0 - WaveformRenderer.pastRegionFraction)).rounded()))
        // Zoom: each drawn column spans `effectivePixelsPerDrawnColumn`
        // drawable pixels along the time axis. Performance mode uses
        // the base `pixelsPerDrawnColumn` (= 2); prep mode divides
        // by `timeAxisZoom` (1.2) so ~20 % more audio is visible.
        let pixelsPerDrawnColumn =
            Self.effectivePixelsPerDrawnColumn(timeAxisZoom: timeAxisZoom)
        let drawnAbovePixels = max(
            0, Int((Double(pastPixels) / pixelsPerDrawnColumn).rounded(.down)))
        let drawnBelowPixels = max(
            0, Int((Double(futurePixels) / pixelsPerDrawnColumn).rounded(.down)))
        let agg = Int(WaveformRenderer.chunksPerColumn)

        // Playhead chunk + chunks past it. In File mode this is
        // computed off the *unclamped* playhead seconds so a hard
        // mouse-scratch can push the playhead into the empty-groove
        // region before `t = 0` or after `t = duration_secs` and
        // the bars slide past the playhead naturally (PRD §9.6
        // "Empty-groove rendering at track edges"). The variable
        // is signed so that the ring-offset math below stays
        // well-defined on negative global chunk indices.
        let peaksLenGlobal = engine.peaksLen(deckIdx: deckIdx)
        let hasFuture = pos.hasTrack

        let playheadChunkSigned: Int64
        if hasFuture {
            if peakChunkDurationSecs > 0 {
                // **Pair-aligned chunk index.** The fragment shader
                // colours each drawn column from the per-band max
                // over `chunksPerColumn` (= 2) consecutive raw
                // chunks anchored at `chunkOffset`. `chunkOffset`
                // is derived directly from `playheadChunkSigned`
                // (via `pastFirstGlobalSigned`), so its **parity**
                // tracks `playheadChunkSigned`'s parity.
                //
                // Audio-frame cadence at 60 fps + 44.1 kHz / 64-
                // sample broadband chunks lands the playhead-chunk
                // advance around 11.4 chunks/frame on average,
                // alternating roughly 11 → 12 → 11 → 12 across
                // frames. With agg = 2 this flips the parity of
                // `chunkOffset` every other frame, which in turn
                // re-pairs every transient chunk K with `K-1` or
                // `K+1` alternately. The neighbor chunk dominates
                // the mid / high band max for that column, so a
                // sharp kick whose mid/high tails decay across
                // ±1 chunk flickers between a low+mid mix (orange/
                // yellow) and a pure low (red) at frame rate.
                //
                // The fix is to round `playheadChunkSigned` down
                // to the nearest multiple of `chunksPerColumn`.
                // The visible playhead position then advances in
                // 2-chunk steps (≈ 2.9 ms at 44.1 kHz = 2 drawable
                // px = 1 logical px on Retina, well below the
                // perceptual motion threshold at 60 fps) but each
                // transient chunk is always paired with the same
                // neighbor, so the column's band aggregate is
                // stable across frames and the colour stops
                // flickering.
                //
                // `floor` semantics (not "round to nearest even")
                // are essential: the existing chunk-ring math
                // downstream uses `floor`-style indexing too, and
                // the empty-groove negative-chunk path (PRD §9.6)
                // relies on the unchanged floor monotonicity.
                let chunkFRaw =
                    (pos.playheadSecsUnclamped / peakChunkDurationSecs).rounded(.down)
                let aggD = Double(WaveformRenderer.chunksPerColumn)
                let chunkFSnapped = (chunkFRaw / aggD).rounded(.down) * aggD
                playheadChunkSigned = Int64(max(-Double(Int64.max / 2),
                                                min(Double(Int64.max / 2), chunkFSnapped)))
            } else {
                playheadChunkSigned = peaksLenGlobal == 0 ? 0 : Int64(peaksLenGlobal &- 1)
            }
        } else {
            playheadChunkSigned = peaksLenGlobal == 0 ? 0 : Int64(peaksLenGlobal &- 1)
        }

        // Publish this draw's playhead + cadence into the shared
        // snapshot so the SwiftUI beat-grid overlay can render in
        // lockstep with the Metal output.
        //
        // **Critical:** publish the **continuous, sample-accurate**
        // `pos.playheadSecsUnclamped`, not the chunk-pair-snapped
        // `playheadChunkSigned * peakChunkDurationSecs`. The Metal
        // waveform's *visible* playhead is
        // `snapped + epsChunks_subpixel_offset` (see
        // `subChunkOffsetNDC` below) — the snapped chunk picks the
        // data column and the eps shift slides the geometry
        // continuously within the chunk-pair quantum. The overlay
        // must match that combined position, not just the snap,
        // otherwise the grid drifts versus the waveform by up to
        // one logical pixel within each chunk-pair quantum and
        // jumps forward in 1-px steps at the snap boundaries —
        // exactly the "grid going left/right minimally" wobble
        // the user reports during smooth waveform scroll. With
        // the continuous value published, the overlay's
        // `(beat - playhead) / secsPerLogicalPx` math lands on the
        // same logical pixel as the renderer's `epsChunks` slide,
        // and the two layers move byte-identically. The previous
        // draft snapped the publication and then re-snapped on
        // read, which double-locked the grid to the chunk-pair
        // grid while the geometry was free to slide. See
        // `WaveformRenderSnapshot` doc for the full sync story.
        if let snapshot = renderSnapshot {
            snapshot.lastDrawnPlayheadSecsUnclamped = pos.playheadSecsUnclamped
            snapshot.peakDurSecs = peakChunkDurationSecs
            snapshot.hasTrack = pos.hasTrack
        }
        if beatGridEnabled || renderSnapshot != nil {
            refreshBeatGridIfNeeded(
                peaksGeneration: currentGeneration,
                beatGridGeneration: beatGridGeneration,
                hasTrack: pos.hasTrack,
                mirrorIntoSnapshot: renderSnapshot)
        }

        // Legacy unsigned variant for code paths that only ever
        // address inside `[0, peaksLen)` (e.g. the
        // chunksAvailableBehind / Ahead fall-back). Clamped here
        // so the legacy paths never see a negative chunk index.
        let playheadChunkClamped: UInt64
        if playheadChunkSigned <= 0 {
            playheadChunkClamped = 0
        } else if UInt64(playheadChunkSigned) >= peaksLenGlobal && peaksLenGlobal > 0 {
            playheadChunkClamped = peaksLenGlobal &- 1
        } else {
            playheadChunkClamped = UInt64(playheadChunkSigned)
        }

        let chunksAvailableBehind: Int
        let chunksAvailableAhead: Int
        if peaksLenGlobal == 0 {
            chunksAvailableBehind = 0
            chunksAvailableAhead = 0
        } else {
            chunksAvailableBehind =
                min(Int(playheadChunkClamped) + 1, WaveformRenderer.chunkCapacity)
            if hasFuture {
                chunksAvailableAhead = min(
                    Int(peaksLenGlobal &- 1 &- playheadChunkClamped),
                    WaveformRenderer.chunkCapacity)
            } else {
                chunksAvailableAhead = 0
            }
        }

        // Empty-groove headroom (PRD §9.6): when the ring buffer
        // has enough unused (zero-initialised) slots past the
        // loaded peak range, let the past and future regions draw
        // their full pixel extent regardless of how close the
        // playhead is to a track edge. Slots ≥ `peaksLenGlobal`
        // are guaranteed zero by `reset()` + the bounded
        // `ingestNewChunks` writer, and so are the slots reached
        // by wrapping the past region's chunk index below 0 (they
        // fall in the same `[peaksLen, chunkCapacity)` zero zone
        // when there is enough headroom). The shader then renders
        // those columns as flat silence — the visual "empty
        // groove" before a track's first frame and after its
        // last, mirroring a real platter's lead-in / lead-out and
        // letting the user push the playhead off the edges of the
        // track without the past/future regions collapsing.
        //
        // The condition needs to keep both edges' worth of
        // headroom clear; tracks long enough to violate it
        // (typically > ~25 min at the broadband 64-sample chunk
        // cadence) fall back to the pre-existing collapse-at-
        // edges behaviour rather than risk wrapping a column back
        // into real loaded data.
        let drawnAboveFull = max(0, drawnAbovePixels)
        let drawnBelowFull = max(0, drawnBelowPixels)
        let neededHeadroom =
            UInt64(drawnAboveFull * agg + drawnBelowFull * agg) &+ peaksLenGlobal
        let zeroPadFits =
            hasFuture
            && peaksLenGlobal > 0
            && neededHeadroom < UInt64(WaveformRenderer.chunkCapacity)

        let drawnAbove: Int
        let drawnBelow: Int
        if zeroPadFits {
            drawnAbove = drawnAboveFull
            drawnBelow = drawnBelowFull
        } else {
            drawnAbove = max(0, min(drawnAbovePixels, chunksAvailableBehind / agg))
            drawnBelow = max(0, min(drawnBelowPixels, chunksAvailableAhead / agg))
        }
        // Raw chunk count behind the playhead — needed to derive
        // the past region's `chunkOffset`. The future region starts
        // at `playheadChunk + 1` regardless of aggregation, so no
        // analogous quantity is needed there.
        let rawAbove = drawnAbove * agg
        let hasContent = peaksLenGlobal > 0 && (drawnAbove + drawnBelow) > 0

        guard let drawable = view.currentDrawable,
              let passDescriptor = view.currentRenderPassDescriptor
        else {
            releaseSemaphore()
            return
        }

        // Per-region chunk + band offsets. The shader's vertex
        // stage addresses raw chunks as `chunkOffset + chunkInWindow
        // × chunksPerColumn`, so `chunkOffset` is in *raw* units
        // here. The signed-Int64 arithmetic + Euclidean modulo lets
        // negative global chunk indices (playhead pushed past the
        // start of the file) and overshoot indices (playhead past
        // the end) wrap into the zero-initialised tail of the ring
        // buffer rather than aliasing into loaded peak data. See
        // §9.6 "Empty-groove rendering at track edges" + the
        // `zeroPadFits` headroom check above for why this is safe
        // for tracks ≤ ~25 min at the broadband chunk cadence.
        let capacitySigned = Int64(WaveformRenderer.chunkCapacity)
        let bandCapacitySigned = Int64(WaveformRenderer.bandChunkCapacity)
        let bandPerSampleSigned = max(Int64(samplesPerBandChunk), 1)
        let samplesPerPeakSigned = Int64(samplesPerPeakChunk)

        func euclideanMod(_ value: Int64, _ modulus: Int64) -> Int {
            let raw = value % modulus
            return Int(raw < 0 ? raw + modulus : raw)
        }

        let pastFirstGlobalSigned: Int64 =
            (playheadChunkSigned &+ 1) &- Int64(rawAbove)
        let pastFirstRingOffset =
            euclideanMod(pastFirstGlobalSigned, capacitySigned)
        let futureFirstGlobalSigned: Int64 = playheadChunkSigned &+ 1
        let futureFirstRingOffset =
            euclideanMod(futureFirstGlobalSigned, capacitySigned)

        let pastFirstSampleSigned =
            pastFirstGlobalSigned &* samplesPerPeakSigned
        // Signed `/` rounds toward zero, but we need floor for the
        // band index so a negative `pastFirstSampleSigned` maps to
        // the band chunk *containing* it (the slot the ring would
        // hold if the band stream extended into negative time).
        // Without the floor adjustment we'd round toward 0 and
        // sample one slot *closer* to 0 than the broadband region
        // does, breaking the band ↔ broadband alignment by one
        // chunk on the very first column of the empty-groove
        // region.
        let pastFirstBandGlobalSigned: Int64 = {
            let q = pastFirstSampleSigned / bandPerSampleSigned
            let r = pastFirstSampleSigned % bandPerSampleSigned
            return (r < 0) ? (q - 1) : q
        }()
        let pastFirstBandRingOffset =
            euclideanMod(pastFirstBandGlobalSigned, bandCapacitySigned)
        let futureFirstSampleSigned =
            futureFirstGlobalSigned &* samplesPerPeakSigned
        let futureFirstBandGlobalSigned: Int64 = {
            let q = futureFirstSampleSigned / bandPerSampleSigned
            let r = futureFirstSampleSigned % bandPerSampleSigned
            return (r < 0) ? (q - 1) : q
        }()
        let futureFirstBandRingOffset =
            euclideanMod(futureFirstBandGlobalSigned, bandCapacitySigned)

        // Band-slot phase offset (samples) for each region. This
        // is the misalignment between the region's first peak-
        // chunk-aligned sample and its containing band chunk's
        // left edge. The shader uses it to correctly compute the
        // band slot for every column in the region (vs. only the
        // first one). See the doc comment on
        // `WaveformUniforms.bandStartPhaseSamples` for the bug
        // this closes — without this offset, columns past the
        // first internal band-chunk boundary read one band slot
        // earlier than they should, and the off-by-one alternates
        // frame-to-frame as `pastFirstSampleSigned mod
        // samplesPerBandChunk` cycles. That alternation was the
        // root cause of the per-frame colour flicker on
        // transients (e.g. purple ↔ light-blue on hats).
        //
        // Euclidean mod so negative empty-groove starts (PRD §9.6)
        // map to a positive offset inside `[0, samplesPerBandChunk)`.
        let pastBandPhase =
            euclideanMod(pastFirstSampleSigned, bandPerSampleSigned)
        let futureBandPhase =
            euclideanMod(futureFirstSampleSigned, bandPerSampleSigned)

        // Sub-chunk geometry shift. The pair-snap (above) keeps the
        // column-to-chunk-pair mapping stable across frames, which
        // means the playhead's displayed position advances in
        // integer multiples of `chunksPerColumn` peak chunks per
        // frame. On a 44.1 kHz track at 60 fps the per-frame advance
        // alternates between 10 and 12 peak chunks (5 and 6 columns,
        // ~5 and 6 logical pixels on Retina), which the eye perceives
        // as low-rate stepped motion regardless of the actual draw
        // refresh rate. The fix here is to render the data at its
        // chunk-quantized position (preserves stable colours and
        // amplitude) *and* shift the entire region's geometry by
        // the sub-chunk fraction of the continuous playhead so each
        // displayed frame lands at exactly the continuous true
        // position. See `WaveformUniforms.subChunkOffsetNDC`.
        //
        // ε is the continuous-vs-snap residual in *peak chunks*,
        // always ≥ 0 because the snap rounds down. Bounded above by
        // `chunksPerColumn`. We translate ε into NDC per region by
        // multiplying by that region's NDC-per-chunk factor; the
        // factors differ between past (0.5 NDC span) and future
        // (1.5 NDC span) but the *physical* shift in drawable pixels
        // works out the same as long as the past/future column
        // counts respect the 0.25/0.75 PRD §9.1 split.
        //
        // We use the continuous playhead (`pos.playheadSecsUn
        // clamped / peakChunkDurationSecs`) here, not any post-snap
        // value. That keeps ε in lockstep with the audio engine's
        // actual playback position so the visual scroll velocity
        // matches the audio's playback velocity exactly, even when
        // the engine's rate is non-1.0× (e.g. timecode scratch,
        // hi-tempo / lo-tempo dial). The Float cast at the end is
        // intentional — the resulting NDC offset is well within
        // single-precision range (≤ 2 NDC units worst case at very
        // narrow deck columns) and uniforms are Float-typed.
        let continuousChunkF =
            pos.playheadSecsUnclamped / peakChunkDurationSecs
        let epsChunks = max(0.0, continuousChunkF - Double(playheadChunkSigned))
        let pastChunksDenom = max(Double(drawnAbove - 1) * Double(agg), 1.0)
        let futureChunksDenom = max(Double(drawnBelow - 1) * Double(agg), 1.0)
        let pastSubChunkOffsetNDC = Float(epsChunks * 0.5 / pastChunksDenom)
        let futureSubChunkOffsetNDC = Float(epsChunks * 1.5 / futureChunksDenom)

        let debugNow = ProcessInfo.processInfo.systemUptime
        if debugNow - debugLastFrameLogUptime >= 1.0 {
            debugLastFrameLogUptime = debugNow
            // #region agent log
            agentDebugLog(
                hypothesisId: "H2,H3,H4",
                location: "WaveformRenderer.swift:draw(in:)",
                message: "beatgrid frame geometry",
                data: [
                    "deckIdx": Int(deckIdx),
                    "orientation": orientation == .vertical ? "vertical" : "horizontal",
                    "hasTrack": pos.hasTrack,
                    "isPlaying": pos.isPlaying,
                    "playheadSecsUnclamped": pos.playheadSecsUnclamped,
                    "elapsedSecs": pos.elapsedSecs,
                    "durationSecs": pos.durationSecs,
                    "engineSampleRate": Int(engine.sampleRate()),
                    "peakDurSecs": peakChunkDurationSecs,
                    "peaksLen": Int(peaksLenGlobal),
                    "samplesPerPeakChunk": Int(samplesPerPeakChunk),
                    "timeAxisPixels": timeAxisPixels,
                    "pixelsPerDrawnColumn": pixelsPerDrawnColumn,
                    "drawnAbove": drawnAbove,
                    "drawnBelow": drawnBelow,
                    "continuousChunkF": continuousChunkF,
                    "snappedChunkF": Double(playheadChunkSigned),
                    "epsChunks": epsChunks,
                    "pastSubChunkOffsetNDC": Double(pastSubChunkOffsetNDC),
                    "futureSubChunkOffsetNDC": Double(futureSubChunkOffsetNDC),
                    "zeroPadFits": zeroPadFits
                ])
            // #endregion
        }

        // 3. Fill both region slots in the per-frame uniform buffer.
        //
        // Past draw sets `chunksAbovePlayhead = chunksAbove` (> 0).
        // Future draw sets `chunksAbovePlayhead = 0`. The shader
        // picks the right time-axis mapping from this flag.
        let pastUniforms = WaveformUniforms(
            chunkOffset: UInt32(pastFirstRingOffset),
            chunksVisible: UInt32(drawnAbove),
            chunksAbovePlayhead: UInt32(drawnAbove),
            yScale: WaveformRenderer.yScale,
            samplesPerPeakChunk: samplesPerPeakChunk,
            bandChunkOffset: UInt32(pastFirstBandRingOffset),
            samplesPerBandChunk: samplesPerBandChunk,
            bandCapacity: UInt32(WaveformRenderer.bandChunkCapacity),
            orientation: orientation.rawValue,
            chunksPerColumn: WaveformRenderer.chunksPerColumn,
            bandStartPhaseSamples: UInt32(pastBandPhase),
            subChunkOffsetNDC: pastSubChunkOffsetNDC)
        let futureUniforms = WaveformUniforms(
            chunkOffset: UInt32(futureFirstRingOffset),
            chunksVisible: UInt32(drawnBelow),
            chunksAbovePlayhead: 0,
            yScale: WaveformRenderer.yScale,
            samplesPerPeakChunk: samplesPerPeakChunk,
            bandChunkOffset: UInt32(futureFirstBandRingOffset),
            samplesPerBandChunk: samplesPerBandChunk,
            bandCapacity: UInt32(WaveformRenderer.bandChunkCapacity),
            orientation: orientation.rawValue,
            chunksPerColumn: WaveformRenderer.chunksPerColumn,
            bandStartPhaseSamples: UInt32(futureBandPhase),
            subChunkOffsetNDC: futureSubChunkOffsetNDC)
        let uniformBuffer = uniformBuffers[uniformIndex]
        let uniformStride = WaveformRenderer.uniformStridePerRegion
        let bufBase = uniformBuffer.contents()
        bufBase.withMemoryRebound(to: WaveformUniforms.self, capacity: 1) { ptr in
            ptr.pointee = pastUniforms
        }
        bufBase.advanced(by: uniformStride).withMemoryRebound(
            to: WaveformUniforms.self, capacity: 1) { ptr in
            ptr.pointee = futureUniforms
        }

        // 4. Record one render pass.
        guard let commandBuffer = commandQueue.makeCommandBuffer() else {
            releaseSemaphore()
            return
        }
        commandBuffer.label = "dub.waveform.commandBuffer"
        commandBuffer.addCompletedHandler { [weak self, weak view] _ in
            releaseSemaphore()
            guard let self else { return }
            let needsCatchUp = self.pendingRedraw
            self.pendingRedraw = false
            guard needsCatchUp else { return }
            // Don't schedule a catch-up repaint mid-scrub — let the
            // gesture + scratch path own the main thread until release.
            if NSEvent.pressedMouseButtons != 0 {
                self.pendingRedraw = true
                return
            }
            DispatchQueue.main.async {
                view?.setNeedsDisplay(view?.bounds ?? .zero)
            }
        }

        // The MTKView's pass descriptor already has the correct
        // colour attachment (the drawable with MSAA resolve when
        // `view.sampleCount == 4`). Force `.clear` + dark deck
        // background so a frame with no chunks still renders the
        // base colour.
        passDescriptor.colorAttachments[0].loadAction = .clear
        passDescriptor.colorAttachments[0].storeAction =
            (WaveformRenderer.sampleCount > 1) ? .multisampleResolve : .store
        passDescriptor.colorAttachments[0].clearColor =
            MTLClearColor(red: 0.07, green: 0.07, blue: 0.08, alpha: 1.0)

        guard let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: passDescriptor)
        else {
            releaseSemaphore()
            return
        }
        encoder.label = "dub.waveform.pass"
        encoder.setRenderPipelineState(waveformPipeline)
        if hasContent {
            encoder.setVertexBuffer(chunksBuffer, offset: 0, index: 1)
            encoder.setVertexBuffer(bandChunksBuffer, offset: 0, index: 2)

            if drawnAbove > 0 {
                encoder.setVertexBuffer(uniformBuffer, offset: 0, index: 0)
                encoder.drawPrimitives(
                    type: .triangleStrip, vertexStart: 0,
                    vertexCount: 2 * drawnAbove)
            }
            if drawnBelow > 0 {
                encoder.setVertexBuffer(
                    uniformBuffer, offset: uniformStride, index: 0)
                encoder.drawPrimitives(
                    type: .triangleStrip, vertexStart: 0,
                    vertexCount: 2 * drawnBelow)
            }
        }

        if beatGridEnabled,
           framesSinceSourceSwap > 20,
           cachedBeatsConfidence > 0,
           !cachedBeats.isEmpty,
           peakChunkDurationSecs > 0,
           pos.hasTrack
        {
            drawBeatGrid(
                encoder: encoder,
                drawableSize: drawableSize,
                playheadSecs: pos.playheadSecsUnclamped,
                snappedChunkF: Double(playheadChunkSigned),
                drawnAbove: drawnAbove,
                drawnBelow: drawnBelow,
                pastSubChunkOffsetNDC: pastSubChunkOffsetNDC,
                futureSubChunkOffsetNDC: futureSubChunkOffsetNDC)
        }

        encoder.endEncoding()

        commandBuffer.present(drawable)
        commandBuffer.commit()

        uniformIndex = (uniformIndex + 1) % WaveformRenderer.maxFramesInFlight
    }

    // MARK: Ingestion

    /// Copy `[startIdx, startIdx + count)` broadband peak chunks
    /// from the engine into the GPU ring at `index % chunkCapacity`.
    private func ingestBroadbandRange(startIdx: UInt64, count: UInt64) {
        guard count > 0 else { return }
        let data = engine.peaksExtend(deckIdx: deckIdx, startIdx: startIdx)
        if data.isEmpty { return }

        let chunkStride = MemoryLayout<PeakChunkLayout>.stride
        let newChunkCount = data.count / chunkStride
        guard newChunkCount > 0, data.count % chunkStride == 0 else { return }

        let ringBytes = WaveformRenderer.chunkCapacity * chunkStride
        let dstBase = chunksBuffer.contents()

        data.withUnsafeBytes { (rawSrc: UnsafeRawBufferPointer) in
            guard let srcBase = rawSrc.baseAddress else { return }
            let firstSlot = Int(startIdx % UInt64(WaveformRenderer.chunkCapacity))
            let bytesToWrite = newChunkCount * chunkStride
            let firstByteOffset = firstSlot * chunkStride

            if firstByteOffset + bytesToWrite <= ringBytes {
                memcpy(dstBase.advanced(by: firstByteOffset), srcBase, bytesToWrite)
            } else {
                let bytesBeforeWrap = ringBytes - firstByteOffset
                memcpy(dstBase.advanced(by: firstByteOffset), srcBase, bytesBeforeWrap)
                memcpy(
                    dstBase,
                    srcBase.advanced(by: bytesBeforeWrap),
                    bytesToWrite - bytesBeforeWrap)
            }
        }
    }

    /// Pull a playhead-centred window of broadband peaks into the
    /// ring. Bounded to [`maxChunksIngestPerFrame`] so load / seek /
    /// scrub stays responsive while the column scrolls.
    private func ingestNewChunks(playheadSecs: Double) {
        let currentLen = engine.peaksLen(deckIdx: deckIdx)
        if currentLen == 0 {
            lastSeenPeaksLen = 0
            return
        }
        if lastSeenPeaksLen == 0 {
            captureChunkCadences()
        }

        let budget = UInt64(WaveformRenderer.maxChunksIngestPerFrame)
        let phChunk: UInt64 = {
            guard peakChunkDurationSecs > 0 else { return 0 }
            let raw = (playheadSecs / peakChunkDurationSecs).rounded(.down)
            let clamped = max(0.0, min(Double(currentLen > 0 ? currentLen - 1 : 0), raw))
            return UInt64(clamped)
        }()
        let half = budget / 2
        let start = phChunk > half ? phChunk - half : 0
        let end = min(currentLen, start + budget)
        if phChunk == lastBroadbandIngestPhChunk,
           lastSeenPeaksLen == currentLen,
           NSEvent.pressedMouseButtons == 0
        {
            return
        }
        lastBroadbandIngestPhChunk = phChunk
        ingestBroadbandRange(startIdx: start, count: end &- start)
        lastSeenPeaksLen = currentLen
    }

    /// Copy `[startIdx, startIdx + count)` band peak chunks into
    /// the GPU ring.
    private func ingestBandRange(startIdx: UInt64, count: UInt64) {
        guard count > 0 else { return }
        let data = engine.bandPeaksExtend(deckIdx: deckIdx, startIdx: startIdx)
        if data.isEmpty { return }

        let chunkStride = MemoryLayout<BandPeakChunkLayout>.stride
        let newChunkCount = data.count / chunkStride
        guard newChunkCount > 0, data.count % chunkStride == 0 else { return }

        let ringBytes = WaveformRenderer.bandChunkCapacity * chunkStride
        let dstBase = bandChunksBuffer.contents()

        data.withUnsafeBytes { (rawSrc: UnsafeRawBufferPointer) in
            guard let srcBase = rawSrc.baseAddress else { return }
            let firstSlot = Int(startIdx % UInt64(WaveformRenderer.bandChunkCapacity))
            let bytesToWrite = newChunkCount * chunkStride
            let firstByteOffset = firstSlot * chunkStride

            if firstByteOffset + bytesToWrite <= ringBytes {
                memcpy(dstBase.advanced(by: firstByteOffset), srcBase, bytesToWrite)
            } else {
                let bytesBeforeWrap = ringBytes - firstByteOffset
                memcpy(dstBase.advanced(by: firstByteOffset), srcBase, bytesBeforeWrap)
                memcpy(
                    dstBase,
                    srcBase.advanced(by: bytesBeforeWrap),
                    bytesToWrite - bytesBeforeWrap)
            }
        }
    }

    /// Playhead-centred band peak ingest; mirrors [`ingestNewChunks`].
    private func ingestNewBandChunks(playheadSecs: Double) {
        let currentLen = engine.bandPeaksLen(deckIdx: deckIdx)
        if currentLen == 0 {
            lastSeenBandPeaksLen = 0
            return
        }

        let budget = UInt64(WaveformRenderer.maxChunksIngestPerFrame)
        let bandDur = engine.bandPeaksChunkDurationSecs(deckIdx: deckIdx)
        let phChunk: UInt64 = {
            guard bandDur > 0 else { return 0 }
            let raw = (playheadSecs / bandDur).rounded(.down)
            let clamped = max(0.0, min(Double(currentLen > 0 ? currentLen - 1 : 0), raw))
            return UInt64(clamped)
        }()
        let half = budget / 2
        let start = phChunk > half ? phChunk - half : 0
        let end = min(currentLen, start + budget)
        if phChunk == lastBandIngestPhChunk,
           lastSeenBandPeaksLen == currentLen,
           NSEvent.pressedMouseButtons == 0
        {
            return
        }
        lastBandIngestPhChunk = phChunk
        ingestBandRange(startIdx: start, count: end &- start)
        lastSeenBandPeaksLen = currentLen
    }

    /// Snapshot the engine's reported broadband + band chunk cadences
    /// on the first non-empty poll. Cached so subsequent draws skip
    /// the FFI cost. Falls back to the M9 / M9.5b defaults
    /// (64 / 512) if the engine returns 0 for either accessor.
    private func captureChunkCadences() {
        let sr = engine.sampleRate()
        if sr == 0 {
            return
        }
        let srD = Double(sr)
        let peakDur = engine.peaksChunkDurationSecs(deckIdx: deckIdx)
        let bandDur = engine.bandPeaksChunkDurationSecs(deckIdx: deckIdx)
        if peakDur > 0 {
            peakChunkDurationSecs = peakDur
            let samples = Int((peakDur * srD).rounded())
            if samples > 0 {
                samplesPerPeakChunk = UInt32(samples)
            }
        }
        if bandDur > 0 {
            let samples = Int((bandDur * srD).rounded())
            if samples > 0 {
                samplesPerBandChunk = UInt32(samples)
            }
        }
    }

    // MARK: - Beat grid (B-24 Metal pass)

    private func refreshBeatGridIfNeeded(
        peaksGeneration: UInt64,
        beatGridGeneration: UInt64,
        hasTrack: Bool,
        mirrorIntoSnapshot snapshot: WaveformRenderSnapshot?
    ) {
        let needsRefresh =
            cachedBeatsGeneration != peaksGeneration
            || cachedBeatGridGeneration != beatGridGeneration
            || (cachedBeatsConfidence == 0 && hasTrack)
        guard needsRefresh else { return }
        if cachedBeatsGeneration != peaksGeneration
            || cachedBeatGridGeneration != beatGridGeneration
        {
            beatGridFetchCooldown = 0
        } else if beatGridFetchCooldown > 0 {
            beatGridFetchCooldown -= 1
            return
        }
        beatGridFetchCooldown = 15
        let grid = engine.beatGrid(deckIdx: deckIdx)
        cachedBeats = grid.beats
        cachedBpm = grid.bpm
        cachedBeatsPerBar = max(1, Int(grid.beatsPerBar))
        cachedBarPhase = min(
            max(0, Int(grid.barPhase)),
            max(0, cachedBeatsPerBar - 1))
        cachedBeatsConfidence = grid.confidence
        cachedBeatsGeneration = peaksGeneration
        cachedBeatGridGeneration = beatGridGeneration
        logBeatGridSummary(
            gridBpm: grid.bpm,
            beats: grid.beats,
            beatsPerBar: Int(grid.beatsPerBar),
            confidence: grid.confidence,
            peaksGeneration: peaksGeneration,
            beatGridGeneration: beatGridGeneration)
        logWholeTrackBeatPeakAlignment(
            gridBpm: grid.bpm,
            beats: grid.beats,
            confidence: grid.confidence,
            peaksGeneration: peaksGeneration,
            beatGridGeneration: beatGridGeneration)
        if let snapshot {
            snapshot.beats = cachedBeats
            snapshot.beatsPerBar = cachedBeatsPerBar
            snapshot.barPhase = cachedBarPhase
            snapshot.beatsConfidence = cachedBeatsConfidence
            snapshot.beatsGeneration = peaksGeneration
        }
    }

    /// Map a beat's track-time to the same NDC the waveform vertex
    /// shader uses. Shared by the Metal tick pass; mirrors the
    /// attempt-3 formulas in `WaveformView.drawBeatGrid`.
    private static func beatTimeNDC(
        beatSecs: Double,
        peakDur: Double,
        snappedChunkF: Double,
        drawnAbove: Int,
        drawnBelow: Int,
        pastSubChunkOffsetNDC: Float,
        futureSubChunkOffsetNDC: Float
    ) -> Double {
        let chunksPerColumn = Double(WaveformRenderer.chunksPerColumn)
        let beatChunkF = beatSecs / peakDur
        let chunksFromSnapped = beatChunkF - snappedChunkF
        let inFuture = chunksFromSnapped > 1.0
        if inFuture {
            let chunkInWindow = chunksFromSnapped / chunksPerColumn
            let denom = max(Double(drawnBelow - 1), 1.0)
            let frac = chunkInWindow / denom
            return Double(0.5 - 1.5 * frac) + Double(futureSubChunkOffsetNDC)
        }
        let chunkInWindow =
            Double(drawnAbove - 1) + chunksFromSnapped / chunksPerColumn
        let denom = max(Double(drawnAbove - 1), 1.0)
        let frac = chunkInWindow / denom
        return Double(1.0 - 0.5 * frac) + Double(pastSubChunkOffsetNDC)
    }

    private static func deckTintRGBA(side: DeckSide, alpha: Float) -> SIMD4<Float> {
        switch side {
        case .a:
            return SIMD4(196.0 / 255.0, 145.0 / 255.0, 87.0 / 255.0, alpha)
        case .b:
            return SIMD4(90.0 / 255.0, 128.0 / 255.0, 136.0 / 255.0, alpha)
        }
    }

    /// Off-white used for beat ticks. Picked just below pure white
    /// (≈ `surface ramp top + 4 %`) so the ticks read as bright
    /// guides without competing with the playhead chevron's pure-
    /// white callout. Crucially, the hue is luminance-distinct
    /// from BOTH deck tints (orange and teal), so a tick remains
    /// visible against a saturated waveform in either deck.
    private static func beatTickRGBA(alpha: Float) -> SIMD4<Float> {
        SIMD4(232.0 / 255.0, 232.0 / 255.0, 232.0 / 255.0, alpha)
    }

    /// First beat index with `beats[i] >= time`. Beats are sorted
    /// ascending so we can skip the prefix outside the visible
    /// window in O(log n) instead of scanning from track start
    /// every frame (which was O(n) and caused occasional pause /
    /// click lag on longer tracks).
    private func firstBeatIndex(atOrAfter time: Double) -> Int {
        var lo = 0
        var hi = cachedBeats.count
        while lo < hi {
            let mid = (lo + hi) / 2
            if cachedBeats[mid] < time {
                lo = mid + 1
            } else {
                hi = mid
            }
        }
        return lo
    }

    private func logBeatGridSummary(
        gridBpm: Double,
        beats: [Double],
        beatsPerBar: Int,
        confidence: Float,
        peaksGeneration: UInt64,
        beatGridGeneration: UInt64
    ) {
        let now = ProcessInfo.processInfo.systemUptime
        guard confidence > 0 || now - debugLastGridSummaryLogUptime >= 1.0 else { return }
        debugLastGridSummaryLogUptime = now

        let intervals = zip(beats.dropFirst(), beats).map { next, prev in next - prev }
        func avg(_ values: ArraySlice<Double>) -> Double {
            guard !values.isEmpty else { return 0 }
            return values.reduce(0, +) / Double(values.count)
        }
        let firstIntervals = intervals.prefix(16)
        let lastIntervals = intervals.suffix(16)
        let firstAvg = avg(firstIntervals)
        let lastAvg = avg(lastIntervals)
        let bpmFirst = firstAvg > 0 ? 60.0 / firstAvg : 0
        let bpmLast = lastAvg > 0 ? 60.0 / lastAvg : 0

        // #region agent log
        agentDebugLog(
            hypothesisId: "H1,H3",
            location: "WaveformRenderer.swift:refreshBeatGridIfNeeded",
            message: "beatgrid estimator summary",
            data: [
                "deckIdx": Int(deckIdx),
                "gridBpm": gridBpm,
                "confidence": Double(confidence),
                "beatsPerBar": beatsPerBar,
                "beatCount": beats.count,
                "firstBeats": Array(beats.prefix(8)),
                "lastBeats": Array(beats.suffix(8)),
                "firstIntervalAvgSecs": firstAvg,
                "lastIntervalAvgSecs": lastAvg,
                "firstIntervalBpm": bpmFirst,
                "lastIntervalBpm": bpmLast,
                "intervalBpmDelta": bpmLast - bpmFirst,
                "peaksGeneration": Int(peaksGeneration),
                "beatGridGeneration": Int(beatGridGeneration),
                "peakDurSecs": peakChunkDurationSecs,
                "peaksLen": Int(lastSeenPeaksLen)
            ])
        // #endregion
    }

    private func logWholeTrackBeatPeakAlignment(
        gridBpm: Double,
        beats: [Double],
        confidence: Float,
        peaksGeneration: UInt64,
        beatGridGeneration: UInt64
    ) {
        guard confidence > 0,
              gridBpm > 0,
              peakChunkDurationSecs > 0,
              !beats.isEmpty
        else { return }

        let peakData = engine.peaksExtend(deckIdx: deckIdx, startIdx: 0)
        let stride = MemoryLayout<PeakChunkLayout>.stride
        let chunkCount = peakData.count / stride
        guard chunkCount > 0, peakData.count % stride == 0 else { return }

        let period = 60.0 / gridBpm
        let windowSecs = min(0.18, max(0.04, period * 0.25))
        let radius = max(4, Int((windowSecs / peakChunkDurationSecs).rounded()))
        var offsets: [Double] = []
        var weighted: [(idx: Int, offset: Double, amp: Double)] = []
        var boundaryHits = 0
        var sampleRows: [[String: Any]] = []

        peakData.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            guard let base = raw.baseAddress?.assumingMemoryBound(to: PeakChunkLayout.self)
            else { return }

            for (idx, beatSecs) in beats.enumerated() {
                let centre = Int((beatSecs / peakChunkDurationSecs).rounded())
                guard centre >= 0, centre < chunkCount else { continue }
                let lo = max(0, centre - radius)
                let hi = min(chunkCount - 1, centre + radius)
                var bestAmp = -1.0
                var bestChunk = centre
                for c in lo...hi {
                    let p = base[c]
                    let amp = Double(max(abs(p.minSample), abs(p.maxSample)))
                    if amp > bestAmp {
                        bestAmp = amp
                        bestChunk = c
                    }
                }
                let offsetChunks = bestChunk - centre
                if abs(offsetChunks) >= radius - 1 {
                    boundaryHits += 1
                }
                let offsetSecs = Double(offsetChunks) * peakChunkDurationSecs
                offsets.append(offsetSecs)
                if bestAmp >= 0.05 && abs(offsetChunks) < radius - 1 {
                    weighted.append((idx: idx, offset: offsetSecs, amp: bestAmp))
                }
                let includeSample =
                    idx < 8
                    || abs(idx - beats.count / 2) <= 4
                    || idx >= max(0, beats.count - 8)
                if includeSample {
                    sampleRows.append([
                        "idx": idx,
                        "beatSecs": beatSecs,
                        "offsetSecs": offsetSecs,
                        "offsetChunks": offsetChunks,
                        "bestAmp": bestAmp,
                        "boundaryHit": abs(offsetChunks) >= radius - 1
                    ])
                }
            }
        }

        func average(_ values: ArraySlice<Double>) -> Double {
            guard !values.isEmpty else { return 0 }
            return values.reduce(0, +) / Double(values.count)
        }

        let firstAvg = average(offsets.prefix(16))
        let midStart = max(0, offsets.count / 2 - 8)
        let midEnd = min(offsets.count, midStart + 16)
        let midAvg = midStart < midEnd ? average(offsets[midStart..<midEnd]) : 0
        let lastAvg = average(offsets.suffix(16))
        let blockSize = 64
        var blockSummaries: [[String: Any]] = []
        if !weighted.isEmpty {
            let blockCount = (beats.count + blockSize - 1) / blockSize
            for block in 0..<blockCount {
                let lo = block * blockSize
                let hi = min(beats.count, lo + blockSize)
                let rows = weighted.filter { $0.idx >= lo && $0.idx < hi }
                guard !rows.isEmpty else { continue }
                let sumW = rows.reduce(0.0) { $0 + $1.amp }
                guard sumW > 0 else { continue }
                let avgOffset = rows.reduce(0.0) { $0 + $1.offset * $1.amp } / sumW
                let avgAmp = sumW / Double(rows.count)
                blockSummaries.append([
                    "startBeatIdx": lo,
                    "endBeatIdx": hi - 1,
                    "startSecs": beats[lo],
                    "endSecs": beats[hi - 1],
                    "avgOffsetSecs": avgOffset,
                    "avgAmp": avgAmp,
                    "usableCount": rows.count
                ])
            }
        }

        let slopeSecsPerBeat: Double = {
            guard weighted.count >= 2 else { return 0 }
            let sumW = weighted.reduce(0.0) { $0 + $1.amp }
            guard sumW > 0 else { return 0 }
            let meanX = weighted.reduce(0.0) { $0 + Double($1.idx) * $1.amp } / sumW
            let meanY = weighted.reduce(0.0) { $0 + $1.offset * $1.amp } / sumW
            let denom = weighted.reduce(0.0) {
                let dx = Double($1.idx) - meanX
                return $0 + $1.amp * dx * dx
            }
            guard denom > 0 else { return 0 }
            return weighted.reduce(0.0) {
                let dx = Double($1.idx) - meanX
                return $0 + $1.amp * dx * ($1.offset - meanY)
            } / denom
        }()

        // #region agent log
        agentDebugLog(
            hypothesisId: "H1,H2,H3",
            location: "WaveformRenderer.swift:logWholeTrackBeatPeakAlignment",
            message: "whole-track beat-to-peak alignment",
            data: [
                "deckIdx": Int(deckIdx),
                "gridBpm": gridBpm,
                "confidence": Double(confidence),
                "beatCount": beats.count,
                "chunkCount": chunkCount,
                "peakDurSecs": peakChunkDurationSecs,
                "windowSecs": windowSecs,
                "radiusChunks": radius,
                "boundaryHitFraction": offsets.isEmpty ? 0 : Double(boundaryHits) / Double(offsets.count),
                "usableFitCount": weighted.count,
                "first16AvgOffsetSecs": firstAvg,
                "middle16AvgOffsetSecs": midAvg,
                "last16AvgOffsetSecs": lastAvg,
                "firstToLastAvgDeltaSecs": lastAvg - firstAvg,
                "slopeSecsPerBeat": slopeSecsPerBeat,
                "projectedDriftOverTrackSecs": slopeSecsPerBeat * Double(max(0, beats.count - 1)),
                "blockSummaries": blockSummaries,
                "sampleOffsets": sampleRows,
                "peaksGeneration": Int(peaksGeneration),
                "beatGridGeneration": Int(beatGridGeneration)
            ])
        // #endregion
    }

    private func peakWindowAround(globalChunk: Int64, radius: Int) -> [String: Any]? {
        guard globalChunk >= 0,
              globalChunk < Int64(lastSeenPeaksLen)
        else { return nil }

        let chunks = chunksBuffer.contents().assumingMemoryBound(to: PeakChunkLayout.self)
        var centreAmp: Double = 0
        var maxAmp: Double = -1
        var maxOffset = 0
        for offset in -radius...radius {
            let g = globalChunk + Int64(offset)
            guard g >= 0, g < Int64(lastSeenPeaksLen) else { continue }
            let c = chunks[Int(g) & (WaveformRenderer.chunkCapacity - 1)]
            let amp = Double(max(abs(c.minSample), abs(c.maxSample)))
            if offset == 0 { centreAmp = amp }
            if amp > maxAmp {
                maxAmp = amp
                maxOffset = offset
            }
        }

        return [
            "centreChunk": Int(globalChunk),
            "centreAmp": centreAmp,
            "maxAmp": maxAmp,
            "maxOffsetChunks": maxOffset,
            "maxOffsetSecs": Double(maxOffset) * peakChunkDurationSecs
        ]
    }

    private func logVisibleBeatAlignment(
        playheadSecs: Double,
        snappedChunkF: Double,
        drawnAbove: Int,
        drawnBelow: Int,
        pastSubChunkOffsetNDC: Float,
        futureSubChunkOffsetNDC: Float,
        drawableSize: CGSize
    ) {
        let now = ProcessInfo.processInfo.systemUptime
        guard now - debugLastVisibleBeatLogUptime >= 1.0 else { return }
        debugLastVisibleBeatLogUptime = now
        guard cachedBeatsConfidence > 0, !cachedBeats.isEmpty, peakChunkDurationSecs > 0 else {
            return
        }

        let nextIdx = firstBeatIndex(atOrAfter: playheadSecs)
        let candidates = [nextIdx - 1, nextIdx].filter {
            $0 >= 0 && $0 < cachedBeats.count
        }
        guard let closestIdx = candidates.min(by: {
            abs(cachedBeats[$0] - playheadSecs) < abs(cachedBeats[$1] - playheadSecs)
        }) else { return }

        let beatSecs = cachedBeats[closestIdx]
        let timeNDC = Self.beatTimeNDC(
            beatSecs: beatSecs,
            peakDur: peakChunkDurationSecs,
            snappedChunkF: snappedChunkF,
            drawnAbove: drawnAbove,
            drawnBelow: drawnBelow,
            pastSubChunkOffsetNDC: pastSubChunkOffsetNDC,
            futureSubChunkOffsetNDC: futureSubChunkOffsetNDC)
        let timeAxisPixels: Double
        switch orientation {
        case .vertical:
            timeAxisPixels = max(1, Double(drawableSize.height))
        case .horizontal:
            timeAxisPixels = max(1, Double(drawableSize.width))
        }
        let drawablePixel = (1.0 - timeNDC) * timeAxisPixels * 0.5
        let beatPeriod = cachedBpm > 0 ? 60.0 / cachedBpm : 0
        let beatChunk = Int64((beatSecs / peakChunkDurationSecs).rounded(.down))
        var peakProbe = peakWindowAround(globalChunk: beatChunk, radius: 32) ?? [:]
        peakProbe["radiusChunks"] = 32

        // #region agent log
        agentDebugLog(
            hypothesisId: "H1,H2,H4",
            location: "WaveformRenderer.swift:drawBeatGrid",
            message: "nearest beat render alignment",
            data: [
                "deckIdx": Int(deckIdx),
                "orientation": orientation == .vertical ? "vertical" : "horizontal",
                "playheadSecs": playheadSecs,
                "closestBeatIdx": closestIdx,
                "closestBeatSecs": beatSecs,
                "deltaBeatToPlayheadSecs": beatSecs - playheadSecs,
                "deltaBeatPeriods": beatPeriod > 0 ? (beatSecs - playheadSecs) / beatPeriod : 0,
                "timeNDC": timeNDC,
                "drawablePixelFromLeadingEdge": drawablePixel,
                "timeAxisPixels": timeAxisPixels,
                "snappedChunkF": snappedChunkF,
                "peakDurSecs": peakChunkDurationSecs,
                "beatChunk": Int(beatChunk),
                "localPeakProbe": peakProbe
            ])
        // #endregion
    }

    private func logBeatGridPixelStability(
        playheadSecs: Double,
        snappedChunkF: Double,
        drawnAbove: Int,
        drawnBelow: Int,
        pastSubChunkOffsetNDC: Float,
        futureSubChunkOffsetNDC: Float,
        drawableSize: CGSize
    ) {
        guard cachedBeatsConfidence > 0, !cachedBeats.isEmpty, peakChunkDurationSecs > 0 else {
            return
        }

        let nextIdx = firstBeatIndex(atOrAfter: playheadSecs)
        let candidates = [nextIdx - 1, nextIdx].filter {
            $0 >= 0 && $0 < cachedBeats.count
        }
        guard let closestIdx = candidates.min(by: {
            abs(cachedBeats[$0] - playheadSecs) < abs(cachedBeats[$1] - playheadSecs)
        }) else { return }

        let beatSecs = cachedBeats[closestIdx]
        let timeNDC = Self.beatTimeNDC(
            beatSecs: beatSecs,
            peakDur: peakChunkDurationSecs,
            snappedChunkF: snappedChunkF,
            drawnAbove: drawnAbove,
            drawnBelow: drawnBelow,
            pastSubChunkOffsetNDC: pastSubChunkOffsetNDC,
            futureSubChunkOffsetNDC: futureSubChunkOffsetNDC)
        let timeAxisPixels: Double
        switch orientation {
        case .vertical:
            timeAxisPixels = max(1, Double(drawableSize.height))
        case .horizontal:
            timeAxisPixels = max(1, Double(drawableSize.width))
        }
        let pixel = (1.0 - timeNDC) * timeAxisPixels * 0.5

        if debugStableBeatIdx == closestIdx {
            let secondsPerPixel = peakChunkDurationSecs
                * Double(WaveformRenderer.chunksPerColumn)
                / Self.effectivePixelsPerDrawnColumn(timeAxisZoom: timeAxisZoom)
            if secondsPerPixel > 0 {
                let expectedPixel = debugStableBeatPixel
                    - (playheadSecs - debugStableBeatPlayheadSecs) / secondsPerPixel
                let error = pixel - expectedPixel
                debugBeatPixelLastError = error
                debugBeatPixelMaxError = max(debugBeatPixelMaxError, abs(error))
                debugBeatPixelErrorSamples += 1
            }
        } else {
            debugStableBeatIdx = closestIdx
            debugBeatPixelMaxError = 0
            debugBeatPixelLastError = 0
            debugBeatPixelErrorSamples = 0
        }
        debugStableBeatPixel = pixel
        debugStableBeatPlayheadSecs = playheadSecs

        let now = ProcessInfo.processInfo.systemUptime
        guard now - debugLastGridStabilityLogUptime >= 1.0 else { return }
        debugLastGridStabilityLogUptime = now

        // #region agent log
        agentDebugLog(
            hypothesisId: "H4",
            location: "WaveformRenderer.swift:logBeatGridPixelStability",
            message: "beatgrid pixel stability",
            data: [
                "deckIdx": Int(deckIdx),
                "orientation": orientation == .vertical ? "vertical" : "horizontal",
                "trackedBeatIdx": closestIdx,
                "trackedBeatSecs": beatSecs,
                "playheadSecs": playheadSecs,
                "pixelFromLeadingEdge": pixel,
                "lastFrameErrorPixels": debugBeatPixelLastError,
                "maxAbsErrorPixels": debugBeatPixelMaxError,
                "samples": debugBeatPixelErrorSamples,
                "timeAxisPixels": timeAxisPixels,
                "snappedChunkF": snappedChunkF,
                "peakDurSecs": peakChunkDurationSecs
            ])
        // #endregion
    }

    private func agentDebugLog(
        hypothesisId: String,
        location: String,
        message: String,
        data: [String: Any]
    ) {
        // #region agent log
        let payload: [String: Any] = [
            "sessionId": "c73978",
            "runId": "beatgrid-pre-fix",
            "hypothesisId": hypothesisId,
            "location": location,
            "message": message,
            "data": data,
            "timestamp": Int(Date().timeIntervalSince1970 * 1000)
        ]
        guard JSONSerialization.isValidJSONObject(payload),
              let json = try? JSONSerialization.data(withJSONObject: payload),
              let line = String(data: json, encoding: .utf8),
              let bytes = (line + "\n").data(using: .utf8)
        else { return }

        let url = URL(fileURLWithPath: "/Users/klos/Development/dub/.cursor/debug-c73978.log")
        if let handle = try? FileHandle(forWritingTo: url) {
            defer { try? handle.close() }
            try? handle.seekToEnd()
            try? handle.write(contentsOf: bytes)
        } else {
            try? bytes.write(to: url, options: .atomic)
        }
        // #endregion
    }

    private func drawBeatGrid(
        encoder: MTLRenderCommandEncoder,
        drawableSize: CGSize,
        playheadSecs: Double,
        snappedChunkF: Double,
        drawnAbove: Int,
        drawnBelow: Int,
        pastSubChunkOffsetNDC: Float,
        futureSubChunkOffsetNDC: Float
    ) {
        let peakDur = peakChunkDurationSecs
        guard peakDur > 0, drawnAbove >= 2, drawnBelow >= 2 else { return }

        let chunksPerColumn = Int(WaveformRenderer.chunksPerColumn)
        let pastFirstChunkSigned =
            Int64(snappedChunkF) + 1 - Int64(drawnAbove * chunksPerColumn)
        let futureLastChunkSigned =
            Int64(snappedChunkF) + Int64(drawnBelow * chunksPerColumn)
        let visibleStart =
            Double(pastFirstChunkSigned - Int64(chunksPerColumn)) * peakDur
        let visibleEnd =
            Double(futureLastChunkSigned + Int64(chunksPerColumn)) * peakDur

        let timeAxisPixels: Float
        switch orientation {
        case .vertical:
            timeAxisPixels = max(1, Float(drawableSize.height))
        case .horizontal:
            timeAxisPixels = max(1, Float(drawableSize.width))
        }
        // PRD-BEATS §5.1 Serato-style rendering, round 4. The
        // pre-round-3 design used deck-tinted ticks against a
        // deck-tinted waveform — same hue, so the ticks vanished
        // in busy passages. Round-3 widened them and bumped alpha;
        // still invisible because LUMINANCE matched too.
        //
        // Round-4 fixes the actual problem: contrast. Beats are
        // now bright WHITE (luminance-distinct from BOTH deck
        // tints) and mirrored across the cross axis as TWO short
        // strips — top headroom `[+beatTickInnerNDC, +1.0]` AND
        // bottom headroom `[-1.0, -beatTickInnerNDC]`. A symmetric
        // peak can no longer occlude both; the user always sees at
        // least one tick edge. Downbeats stay deck-tinted at full
        // height so deck identity remains glanceable.
        let beatHalfNDC = 1.5 / timeAxisPixels
        let barHalfNDC = 3.5 / timeAxisPixels

        beatGridScratchVertices.removeAll(keepingCapacity: true)
        let startIdx = firstBeatIndex(atOrAfter: visibleStart)

        for idx in startIdx..<cachedBeats.count {
            let beat = cachedBeats[idx]
            if beat > visibleEnd { break }
            let timeNDC = Self.beatTimeNDC(
                beatSecs: beat,
                peakDur: peakDur,
                snappedChunkF: snappedChunkF,
                drawnAbove: drawnAbove,
                drawnBelow: drawnBelow,
                pastSubChunkOffsetNDC: pastSubChunkOffsetNDC,
                futureSubChunkOffsetNDC: futureSubChunkOffsetNDC)
            let isDownbeat =
                cachedBeatsPerBar > 0
                && (idx % cachedBeatsPerBar == cachedBarPhase)
            if isDownbeat {
                appendBeatLineQuad(
                    vertices: &beatGridScratchVertices,
                    timeNDC: Float(timeNDC),
                    halfThickness: barHalfNDC,
                    color: Self.deckTintRGBA(side: side, alpha: 1.0),
                    isDownbeat: true)
            } else {
                appendMirroredBeatTick(
                    vertices: &beatGridScratchVertices,
                    timeNDC: Float(timeNDC),
                    halfThickness: beatHalfNDC,
                    color: Self.beatTickRGBA(alpha: 0.88))
            }
        }

        logVisibleBeatAlignment(
            playheadSecs: playheadSecs,
            snappedChunkF: snappedChunkF,
            drawnAbove: drawnAbove,
            drawnBelow: drawnBelow,
            pastSubChunkOffsetNDC: pastSubChunkOffsetNDC,
            futureSubChunkOffsetNDC: futureSubChunkOffsetNDC,
            drawableSize: drawableSize)
        logBeatGridPixelStability(
            playheadSecs: playheadSecs,
            snappedChunkF: snappedChunkF,
            drawnAbove: drawnAbove,
            drawnBelow: drawnBelow,
            pastSubChunkOffsetNDC: pastSubChunkOffsetNDC,
            futureSubChunkOffsetNDC: futureSubChunkOffsetNDC,
            drawableSize: drawableSize)

        guard !beatGridScratchVertices.isEmpty else { return }

        let byteCount =
            beatGridScratchVertices.count * MemoryLayout<BeatGridVertexLayout>.stride
        if beatGridVertexBuffer == nil
            || beatGridVertexCapacity < beatGridScratchVertices.count
        {
            let newCap = max(beatGridScratchVertices.count, 256)
            beatGridVertexBuffer = device.makeBuffer(
                length: newCap * MemoryLayout<BeatGridVertexLayout>.stride,
                options: .storageModeShared)
            beatGridVertexCapacity = newCap
        }
        guard let buffer = beatGridVertexBuffer else { return }
        beatGridScratchVertices.withUnsafeBytes { raw in
            guard let src = raw.baseAddress else { return }
            memcpy(buffer.contents(), src, byteCount)
        }

        encoder.setRenderPipelineState(beatGridPipeline)
        encoder.setVertexBuffer(buffer, offset: 0, index: 0)
        encoder.drawPrimitives(
            type: .triangle,
            vertexStart: 0,
            vertexCount: beatGridScratchVertices.count)
    }

    /// PRD-BEATS §5.1 Serato-style cross-axis ranges. A beat tick
    /// is a pair of short marks in the "headroom" at the top and
    /// bottom edges of the cross axis (mirrored — see
    /// `appendMirroredBeatTick`); a downbeat is a full-height
    /// line that cuts through the waveform. The waveform shader
    /// uses cross-axis ∈ [-1, +1] for amplitude, so each tick
    /// strip lives in the outer ~19 % of the axis (|y/x| ∈
    /// [0.62, 1.0]). Mirroring is what guarantees visibility
    /// against busy waveforms: a hot peak can occlude one side,
    /// never both. Downbeats keep the full [-1, +1] span so the
    /// bar boundary is unmistakable at a glance.
    private static let beatTickInnerNDC: Float = 0.62
    private static let beatTickOuterNDC: Float = 1.0

    /// Emit a beat tick as TWO short strips: one in the top
    /// headroom `[+beatTickInnerNDC, +beatTickOuterNDC]` and one
    /// in the bottom headroom mirrored to negative cross-axis.
    /// Even an asymmetric loud peak that fills one half of the
    /// waveform still leaves the opposite tick fully visible.
    private func appendMirroredBeatTick(
        vertices: inout [BeatGridVertexLayout],
        timeNDC: Float,
        halfThickness: Float,
        color: SIMD4<Float>
    ) {
        appendCrossAxisStrip(
            vertices: &vertices,
            timeNDC: timeNDC,
            halfThickness: halfThickness,
            color: color,
            crossLow: Self.beatTickInnerNDC,
            crossHigh: Self.beatTickOuterNDC)
        appendCrossAxisStrip(
            vertices: &vertices,
            timeNDC: timeNDC,
            halfThickness: halfThickness,
            color: color,
            crossLow: -Self.beatTickOuterNDC,
            crossHigh: -Self.beatTickInnerNDC)
    }

    /// Append one axis-aligned tick quad (two triangles, six
    /// vertices) at `timeNDC`. Downbeats span the full amplitude
    /// axis; beat ticks span only the headroom strip
    /// (`beatTickInnerNDC ... beatTickOuterNDC`), keeping the
    /// waveform readable underneath.
    private func appendBeatLineQuad(
        vertices: inout [BeatGridVertexLayout],
        timeNDC: Float,
        halfThickness: Float,
        color: SIMD4<Float>,
        isDownbeat: Bool
    ) {
        let crossLow: Float = isDownbeat ? -1.0 : Self.beatTickInnerNDC
        let crossHigh: Float = isDownbeat ? 1.0 : Self.beatTickOuterNDC
        appendCrossAxisStrip(
            vertices: &vertices,
            timeNDC: timeNDC,
            halfThickness: halfThickness,
            color: color,
            crossLow: crossLow,
            crossHigh: crossHigh)
    }

    /// Emit a single axis-aligned strip quad spanning `[crossLow,
    /// crossHigh]` on the cross axis and ±`halfThickness` around
    /// `timeNDC` on the time axis. Vertex windings match the
    /// existing beat-grid shader (no culling required).
    private func appendCrossAxisStrip(
        vertices: inout [BeatGridVertexLayout],
        timeNDC: Float,
        halfThickness: Float,
        color: SIMD4<Float>,
        crossLow: Float,
        crossHigh: Float
    ) {
        switch orientation {
        case .vertical:
            let y0 = timeNDC - halfThickness
            let y1 = timeNDC + halfThickness
            let x0 = crossLow
            let x1 = crossHigh
            vertices.append(.init(position: SIMD2(x0, y0), color: color))
            vertices.append(.init(position: SIMD2(x1, y0), color: color))
            vertices.append(.init(position: SIMD2(x0, y1), color: color))
            vertices.append(.init(position: SIMD2(x1, y0), color: color))
            vertices.append(.init(position: SIMD2(x1, y1), color: color))
            vertices.append(.init(position: SIMD2(x0, y1), color: color))
        case .horizontal:
            let x0 = -timeNDC - halfThickness
            let x1 = -timeNDC + halfThickness
            let y0 = crossLow
            let y1 = crossHigh
            vertices.append(.init(position: SIMD2(x0, y0), color: color))
            vertices.append(.init(position: SIMD2(x1, y0), color: color))
            vertices.append(.init(position: SIMD2(x0, y1), color: color))
            vertices.append(.init(position: SIMD2(x1, y0), color: color))
            vertices.append(.init(position: SIMD2(x1, y1), color: color))
            vertices.append(.init(position: SIMD2(x0, y1), color: color))
        }
    }

    /// Peek the AppKit event queue without dequeuing. Returns
    /// `true` when a mouse / scroll event is waiting — used to
    /// bail out of `draw(in:)` so clicks reach gesture handlers
    /// before the next Metal encode.
    private static func hasPendingPointerInput() -> Bool {
        // Always yield for press events so click-to-scrub reaches
        // SwiftUI's `DragGesture` before Metal encodes.
        let pressMask: NSEvent.EventTypeMask = [.leftMouseDown, .rightMouseDown]
        let deadline = Date(timeIntervalSinceNow: 0)
        for mode in [RunLoop.Mode.eventTracking, .default] {
            if NSApp.nextEvent(matching: pressMask, until: deadline, inMode: mode, dequeue: false)
                != nil
            {
                return true
            }
        }
        // During playback we ignore `.leftMouseDragged` — micro-
        // jitter was skipping steady-state frames and stuttering the
        // scroll. While the button is held (active scrub) we yield
        // again so `scratchPointerOffset` runs before `draw(in:)`.
        guard NSEvent.pressedMouseButtons != 0 else { return false }
        let dragMask: NSEvent.EventTypeMask = [.leftMouseDragged, .leftMouseUp]
        for mode in [RunLoop.Mode.eventTracking, .default] {
            if NSApp.nextEvent(matching: dragMask, until: deadline, inMode: mode, dequeue: false)
                != nil
            {
                return true
            }
        }
        return false
    }
}
