//
//  Shaders.metal
//  Dub
//
//  Serato-faithful main-waveform shader.
//
//  This is a deliberate, audited reset from M10's HDR / bloom /
//  onset-confidence / chroma-desat / kick-prominence / RMS-body /
//  djLandmarks stack down to the reference algorithm Mixxx ships
//  (commit 358662d, Oct 2025) which itself mirrors what Serato has
//  done since Scratch Live. The look it produces is:
//
//      height(x) = max broadband peak over the pixel column
//      hue(x)    = R = lowPeak  · low.r + midPeak · mid.r + highPeak · high.r
//                  G = lowPeak  · low.g + midPeak · mid.g + highPeak · high.g
//                  B = lowPeak  · low.b + midPeak · mid.b + highPeak · high.b
//                  → normalise so max(R, G, B) == 1
//
//  Bands map onto Serato-style colour anchors — *not* the pure RGB
//  primaries Mixxx's stock skin uses. The reason is human-readable
//  band-mix discrimination:
//
//      Pure RGB primaries: bass + mid → yellow (R + G), bass + high
//      → magenta, mid + high → cyan. Because almost every musical
//      sample has measurable energy in *both* bass and mid, the
//      pixel column lands on yellow-orange the overwhelming
//      majority of the time. Sections with very different spectral
//      content end up rendering as visually-similar warm hues —
//      the eye can't tell a kick-only break apart from a kick +
//      snare drop.
//
//      Serato's anchors are chosen so the *pairwise mixes* stay
//      readable without letting bass + mid collapse into the old
//      yellow wash. Low stays warm red/orange; mid/presence is
//      green (guitar/string/vocal body); high is blue/cyan.
//
//          low  = (1.00, 0.30, 0.05)  warm orange-red  (kick / bass)
//          mid  = (0.10, 0.95, 0.28)  green            (guitar / string / vocal body)
//          high = (0.18, 0.62, 1.00)  cyan-blue         (hats / cymbals / air)
//
//      Equal-energy broadband peaks still trend toward white because
//      all three anchors contribute different channels; quiet columns
//      are separately desaturated by broadband peak amplitude below.
//
//  **Critical:** `dub-spectral` log-compresses every FFT bin via
//  `ln(1 + λ · |X|)` with `λ = 1000` *before* the per-band RMS, so
//  the band values arriving here are perceptual loudness values,
//  not linear amplitudes. Because the decimator stores
//  `sqrt(mean(log_mag²))`, the transform is not invertible with a
//  plain `exp()` in the shader:
//
//      exp(sqrt(mean(log(x)²))) != mean(x)
//
//  That bad inverse was the "all white" bug: it expanded loud
//  perceptual values back into huge pseudo-linear amplitudes and
//  every colour channel clipped. The fragment stage below instead
//  uses the compressed values directly, with separate handling for
//  brightness (how loud the column is) and chroma (which bands rise
//  above the column's shared broadband floor).
//
//  Bands come from `dub-spectral`'s 8 log-spaced RMS bands aggregated
//  into 3 (max-of-band-in-group, matching Mixxx's per-pixel max):
//      bass = max(b0, b1)
//      mid  = max(b2, b3, b4, b5)
//      high = max(b6, b7)
//
//  Height comes from the broadband `PeakChunk` min/max — the envelope
//  is the geometry, the colour is the fragment.
//
//  Geometry (per-pixel-column max aggregation, the Mixxx primitive):
//  each region (past / future) is one triangle strip of `2 ×
//  chunksVisible` vertices where `chunksVisible` is the **drawn
//  column count** (≈ region pixels along the time axis). For every
//  drawn column the vertex stage loops over `chunksPerColumn`
//  consecutive raw chunks and emits the max envelope + max
//  per-band-group. This is the same operation Mixxx's
//  `WaveformRendererRGB::draw` performs CPU-side; doing it in the
//  vertex shader keeps the data path zero-copy. With 1 drawn column
//  per drawable pixel the trapezoidal slices are ≥ 1 px tall, so the
//  amplitude variance between adjacent raw chunks no longer creates
//  the "pin-stripe / venetian-blind" comb pattern the un-aggregated
//  version produced at high zoom.
//
//  Vertical NDC layout (PRD §9.1 — vertical performance default):
//      Past region:   y ∈ [+0.5, +1.0]   (top 25 %)
//      Playhead line: y =  +0.5          (drawn as a SwiftUI overlay)
//      Future region: y ∈ [-1.0, +0.5)   (bottom 75 %)
//
//  Horizontal layout (Prep mode, M10.8) flips time onto the x-axis;
//  the renderer flags this via `Uniforms.orientation` and the vertex
//  swap is the only difference.
//

#include <metal_stdlib>
using namespace metal;

/// Per-frame uniforms. Field order + types must match
/// `WaveformRenderer.WaveformUniforms` exactly — `memcpy`'d each
/// frame from the host with no padding adjustment.
struct Uniforms {
    /// Ring offset of the oldest visible broadband (and filtered)
    /// chunk for this region.
    uint chunkOffset;
    /// Number of broadband chunks in this region's draw.
    uint chunksVisible;
    /// > 0 ⇒ this is the past-region draw; == 0 ⇒ future region.
    uint chunksAbovePlayhead;
    /// Amplitude scale in NDC. 0.95 leaves a small gutter so peaks
    /// don't kiss the deck-column edge.
    float yScale;
    /// Audio samples per broadband chunk (for the band-ring lookup).
    uint samplesPerPeakChunk;
    /// Ring offset of the oldest visible band chunk for this region.
    uint bandChunkOffset;
    /// Audio samples per band chunk.
    uint samplesPerBandChunk;
    /// Power-of-two band-ring capacity. The vertex shader does
    /// `(idx & (capacity - 1))` to wrap.
    uint bandCapacity;
    /// 0 = vertical (time→y), 1 = horizontal (time→x).
    uint orientation;
    /// Raw broadband chunks aggregated into one drawn column. ≥ 1.
    /// The vertex shader reads this many consecutive chunks starting
    /// at `chunkOffset + chunkInWindow * chunksPerColumn` and emits
    /// the max envelope + max per-band-group for that range.
    uint chunksPerColumn;
    /// Band-slot phase offset, in samples, of the visible region's
    /// first peak chunk inside its containing band chunk. The shader
    /// uses this to compute the correct band slot for every column,
    /// not just the first one. See the doc comment on the host-side
    /// `WaveformUniforms.bandStartPhaseSamples` for the off-by-one
    /// bug it closes.
    uint bandStartPhaseSamples;
    /// Sub-chunk geometry shift in NDC for this region. Added to
    /// every vertex's `timeNDC` so the displayed waveform position
    /// advances by the *continuous* playhead delta per frame even
    /// though the chunk indexing snaps to integer-pair boundaries.
    /// See the host-side `WaveformUniforms.subChunkOffsetNDC` doc
    /// for the smoothness rationale.
    float subChunkOffsetNDC;
    /// Fraction of this region's NDC span that holds content (`1.0`
    /// normally; < 1.0 only near a track edge on a track too long for
    /// the empty-groove zero-pad). The filled columns render at native
    /// density anchored at the playhead; the rest stays background.
    /// See the host-side `WaveformUniforms.regionFillFrac` doc.
    float regionFillFrac;
};

/// One element of the broadband-peak ring. CPU-side
/// `PeakChunkLayout` mirror — 12 bytes, no padding.
struct PeakChunk {
    float minSample;
    float maxSample;
    float rms;
};

/// One element of the band-peak ring. CPU-side `BandPeakChunkLayout`
/// mirror — 32 bytes (8 × f32 band RMS).
struct BandPeakChunk {
    float b0; float b1; float b2; float b3;
    float b4; float b5; float b6; float b7;
};

struct VertexOut {
    float4 position [[position]];
    /// Aggregated low / mid / high band peaks for this chunk.
    /// Linear, raw — no smoothing, no normalisation. The fragment
    /// stage mixes them into RGB.
    float3 bands;
    /// Aggregated sub-bass band (`b0`, ≈ 43-86 Hz at 44.1 kHz).
    /// Used only for Serato-style quiet greying: quiet columns
    /// should recede when they are sub-bass/rumble, not when they
    /// contain audible midrange instruments.
    float subBass;
    /// Aggregated broadband peak amplitude for this drawn column.
    /// This is the same peak envelope that drives geometry height,
    /// surfaced to the fragment stage so quiet columns can be
    /// desaturated without confusing "quiet" with "low frequency."
    float peak;
};

/// Vertex shader. Two vertices per chunk at the chunk's time-centre:
/// even `vid` = `-min` edge, odd `vid` = `+max` edge. Triangle strip
/// topology stitches them into a continuous envelope.
vertex VertexOut waveformVertex(uint vid                       [[vertex_id]],
                                constant Uniforms& u           [[buffer(0)]],
                                constant PeakChunk* chunks     [[buffer(1)]],
                                constant BandPeakChunk* bands  [[buffer(2)]]) {
    VertexOut out;

    const uint chunkInWindow = vid >> 1u;
    const bool isMaxEdge     = (vid & 1u) == 1u;

    // Visibility guard. The renderer caps the draw count to
    // `2 × chunksVisible`, but a one-off layout race could oversend
    // vertices; collapse them onto the clear colour so they don't
    // streak.
    if (u.chunksVisible == 0u || chunkInWindow >= u.chunksVisible) {
        out.position = float4(0, 0, 0, 0);
        out.bands    = float3(0);
        out.subBass  = 0.0;
        out.peak     = 0.0;
        return out;
    }

    // Per-drawn-column max aggregation. We span `chunksPerColumn`
    // consecutive raw chunks (the broadband ring and the band ring
    // are both addressed at this cadence) and take the per-band
    // max + the broadband min/max envelope across the run. This is
    // the operation Mixxx's CPU renderer performs once per pixel
    // column and is the difference between a smooth filled
    // envelope and a sub-pixel comb pattern when the trapezoidal
    // strip's row height drops below 1 px.
    const uint colStart = u.chunkOffset + chunkInWindow * u.chunksPerColumn;
    const uint nAgg     = max(u.chunksPerColumn, 1u);

    // **3-tap horizontal envelope smoothing (M11d.5 follow-up).**
    //
    // The raw `max(chunk.maxSample / |chunk.minSample|)` aggregate
    // jumps wildly between adjacent drawn columns because each
    // column samples only ~ 2.9 ms of audio at 44.1 kHz × 64-
    // sample peak chunks × 2 chunks/column. Music's true envelope
    // is a continuous curve, so the eye reads our raw envelope as
    // "tiny vertical sticks of different heights" instead of one
    // smooth wave shape — the Mixxx-style look that fails the
    // Serato side-by-side test.
    //
    // Fix: emit each column's amplitude as a weighted average of
    // (column N-1, column N, column N+1) with weights (¼, ½, ¼).
    // This is the cheapest possible spline approximation that
    // still preserves transient peaks (because the centre weight
    // dominates and the neighbours, being smaller, only soften
    // the silhouette into the adjacent column instead of erasing
    // the peak entirely). Boundary columns clamp to the centre's
    // own value via `chunkInWindow > 0` / `< chunksVisible - 1`
    // tests so we don't leak past the visible region.
    //
    // The kernel runs on the post-aggregation envelope so each
    // tap is the same `nAgg`-chunk max we already trust. Cost is
    // 2× extra `nAgg`-chunk loops per drawn vertex — negligible
    // on Apple Silicon (the vertex shader is currently bandwidth-
    // bound, not ALU-bound).
    float maxPos  = 0.0;
    float maxNeg  = 0.0;
    {
        float centrePos = 0.0;
        float centreNeg = 0.0;
        for (uint i = 0u; i < nAgg; ++i) {
            const PeakChunk pc = chunks[(colStart + i) & (1048576u - 1u)];
            centrePos = max(centrePos, pc.maxSample);
            centreNeg = max(centreNeg, fabs(pc.minSample));
        }

        float leftPos  = centrePos;
        float leftNeg  = centreNeg;
        if (chunkInWindow > 0u) {
            const uint leftStart = colStart - u.chunksPerColumn;
            leftPos = 0.0;
            leftNeg = 0.0;
            for (uint i = 0u; i < nAgg; ++i) {
                const PeakChunk pc = chunks[(leftStart + i) & (1048576u - 1u)];
                leftPos = max(leftPos, pc.maxSample);
                leftNeg = max(leftNeg, fabs(pc.minSample));
            }
        }

        float rightPos = centrePos;
        float rightNeg = centreNeg;
        if (chunkInWindow + 1u < u.chunksVisible) {
            const uint rightStart = colStart + u.chunksPerColumn;
            rightPos = 0.0;
            rightNeg = 0.0;
            for (uint i = 0u; i < nAgg; ++i) {
                const PeakChunk pc = chunks[(rightStart + i) & (1048576u - 1u)];
                rightPos = max(rightPos, pc.maxSample);
                rightNeg = max(rightNeg, fabs(pc.minSample));
            }
        }

        maxPos = 0.25 * leftPos + 0.5 * centrePos + 0.25 * rightPos;
        maxNeg = 0.25 * leftNeg + 0.5 * centreNeg + 0.25 * rightNeg;
    }

    // Per-column band lookup with phase-correct slot indexing.
    //
    // **The bug this closes.** The band ring is cadenced at one
    // slot per `samplesPerBandChunk` (512 at 44.1/48 kHz), an
    // order of magnitude coarser than the broadband peak ring
    // (64 samples). The host's `bandChunkOffset` points at the
    // band slot that *contains* the visible region's first peak
    // chunk, but that first peak chunk is generally **not** band-
    // chunk-aligned — it sits some `bandStartPhaseSamples` into
    // its band slot. The previous formula
    //
    //   bandIdx = bandChunkOffset + localSampleIdx
    //                                / samplesPerBandChunk
    //
    // ignored that phase, so for any column whose local sample
    // offset pushed past the next band-chunk boundary inside the
    // region (which is `samplesPerBandChunk − bandStartPhase
    // Samples` samples away, not `samplesPerBandChunk`), the
    // shader read the band slot *one earlier* than the slot that
    // actually contained that column's audio.
    //
    // As the playhead advanced, `bandStartPhaseSamples` cycled
    // frame to frame, which moved the boundary column back and
    // forth across every visible transient — the same audio
    // position alternated between reading its correct band slot
    // and the slot before it. Those two slots have different
    // spectral content on any sharp transient, so the column's
    // colour flipped at frame rate (most visibly purple ↔
    // light-blue on hi-hats and cymbals, where the `b5` presence
    // band feeds both `mid` and `high`).
    //
    // **The fix.** Add the phase offset before the integer
    // division so each column's band slot lookup is computed
    // against the same band-chunk grid the host used to place
    // the region:
    //
    //   bandIdx = bandChunkOffset
    //           + (bandStartPhaseSamples + localSampleIdx)
    //             / samplesPerBandChunk
    //
    // With this, a specific audio chunk always lands in the same
    // band slot regardless of where the playhead sits relative to
    // band-chunk boundaries, and the flicker goes away without
    // any smoothing kernel disturbing the colour palette. The
    // amplitude path above is unaffected.
    const uint localBaseChunk = chunkInWindow * u.chunksPerColumn;
    const uint localSampleIdx = localBaseChunk * u.samplesPerPeakChunk;
    const uint bandIdx = u.bandChunkOffset
        + (u.bandStartPhaseSamples + localSampleIdx) / u.samplesPerBandChunk;
    const BandPeakChunk bc = bands[bandIdx & (u.bandCapacity - 1u)];
    const float maxSubBass = bc.b0;
    const float maxBass    = max(bc.b0, bc.b1);
    const float maxMid     = max(max(max(bc.b2, bc.b3), bc.b4), bc.b5 * 0.92);
    const float maxHigh    = max(max(bc.b5 * 0.35, bc.b6), bc.b7);

    out.bands = float3(maxBass, maxMid, maxHigh);
    out.subBass = maxSubBass;
    out.peak = max(maxPos, maxNeg);

    // Amplitude edge. `pc.minSample` is signed and ≤ 0 for normal
    // music; the envelope's lower edge is `pc.minSample`, the
    // upper edge `pc.maxSample`. The visual envelope is symmetric
    // around 0 (M10.5b convention), so we flip `minSample`'s sign
    // here and let the strip topology fill the trapezoid.
    const float amp = isMaxEdge ? maxPos : -maxNeg;
    // Soft-knee amplitude companding (M12e dogfood). A brick-wall-
    // limited master has |sample| ≈ 1.0 almost everywhere, so the old
    // linear `amp * yScale` pinned the envelope flat against the frame
    // and the whole strip read as a solid colour block. Instead: leave
    // magnitudes below `knee` linear (quiet material keeps full vertical
    // detail), and above it compress asymptotically toward `ceiling`
    // (= yScale, the existing gutter) so loud peaks bunch *softly* just
    // under the frame with a visible gradient rather than a hard clip.
    // The map is C¹-continuous at the knee (unit slope both sides), so
    // there is no kink where it engages. Tunable: raise `knee` for a
    // punchier look, lower it for a flatter/compressed one.
    const float knee = 0.55;
    const float ceiling = u.yScale;
    const float mag = fabs(amp);
    float companded = mag;
    if (mag > knee) {
        const float range = ceiling - knee;
        const float over = mag - knee;
        companded = knee + range * (over / (over + range));
    }
    const float ampNDC = clamp(sign(amp) * companded, -1.0, 1.0);

    // Time-axis NDC. Past region maps `chunkInWindow ∈ [0, K)` to
    // `y ∈ [+1.0, +0.5]` (oldest at top, newest at the playhead).
    // Future region maps `chunkInWindow ∈ [0, K)` to `y ∈ [+0.5, -1.0]`
    // (oldest just under the playhead, newest at the bottom).
    //
    // `regionFillFrac` (= 1.0 normally) anchors the filled columns at
    // the playhead (y = +0.5) and scales the span so content keeps its
    // native density when the region is starved near a track edge —
    // the unfilled remainder falls outside [−1, 1] / the region and
    // shows as background, instead of the few available chunks being
    // stretched across the whole region (the long-track lead-in warp).
    const float frac = (u.chunksVisible > 1u)
        ? float(chunkInWindow) / float(u.chunksVisible - 1u)
        : 0.0;
    float timeNDC;
    if (u.chunksAbovePlayhead > 0u) {
        // Past: playhead at +0.5, older columns step up toward +1.0.
        timeNDC = 0.5 + 0.5 * u.regionFillFrac * (1.0 - frac);
    } else {
        // Future: playhead at +0.5, later columns step down toward -1.0.
        timeNDC = 0.5 - 1.5 * u.regionFillFrac * frac;
    }

    // Add the sub-chunk geometry shift so the displayed waveform
    // moves continuously between integer-chunk snap positions. This
    // is the "make 60 fps actually look like 60 fps" fix. See the
    // host-side `WaveformUniforms.subChunkOffsetNDC` doc for the
    // full derivation. Sign-wise: past content needs to move toward
    // +y (older direction) as the playhead advances continuously,
    // and so does future content (toward +y, toward the playhead),
    // so both regions get the same additive shift. The horizontal
    // path below mirrors `timeNDC` into `xNDC = -timeNDC`, which
    // turns `+y` into `-x` (leftward) — the correct scroll direction
    // for the Prep-mode horizontal layout.
    timeNDC += u.subChunkOffsetNDC;

    // Vertical (default) puts time on y, amplitude on x.
    // Horizontal swaps the two — the playhead lives at x = -0.5
    // (= NDC 25 % from the left) for the same chunkInWindow-=-0
    // start. The amplitude axis flips sign in horizontal so the
    // `+max` edge sits *above* the time axis (positive y) and
    // `-min` *below*, matching the eye's "up is louder" intuition.
    if (u.orientation == 0u) {
        out.position = float4(ampNDC, timeNDC, 0.0, 1.0);
    } else {
        // Horizontal: rotate 90° clockwise. Time runs left → right,
        // playhead at x = -0.5 (mirrors the vertical "top = past"
        // semantic: left = past, right = future).
        const float xNDC = -timeNDC;
        out.position = float4(xNDC, ampNDC, 0.0, 1.0);
    }
    return out;
}

/// Fragment shader. Uses the perceptual band loudness values as
/// perceptual values. Brightness comes from the column's loudest
/// band; hue comes from a **calibrated** low/mid/high comparison.
///
/// The calibration is not cosmetic. `dub-spectral`'s RMS-over-log-
/// magnitude bands are structurally bass-heavy: on "Potential
/// Victims" at 1:24-1:28, the raw grouped max classifier produced
/// low/mid/high winners of 338/7/0. Serato shows obvious mid/high
/// colour in the same region, so the renderer must whiten the
/// groups before deciding hue. The bias/gain below is a compressed-
/// domain equal-loudness correction for this analysis path.
fragment float4 waveformFragment(VertexOut in [[stage_in]]) {
    const float bass = max(in.bands.x, 0.0);
    const float mid  = max(in.bands.y, 0.0);
    const float high = max(in.bands.z, 0.0);

    const float3 raw = float3(bass, mid, high);
    const float rawMax = max(max(raw.x, raw.y), raw.z);

    // Compressed-loudness scale: silence sits near 0; strong music
    // in this analysis path lands around 8-12. Keep gate and brightness in
    // that same domain instead of pretending the values are linear.
    const float gate = smoothstep(0.02, 0.12, rawMax);
    const float brightness = smoothstep(0.4, 11.5, rawMax) * gate;

    // Serato-faithful anchors. After moving the 1.5-3.3 kHz
    // presence band (`b5`) into mid, guitars/strings and many vocal
    // consonants should read green, while true top-end (`b6-b7`)
    // stays cyan/blue.
    const float3 lowColor  = float3(1.00, 0.12, 0.24);
    const float3 midColor  = float3(0.08, 0.94, 0.22);
    const float3 highColor = float3(0.58, 0.36, 1.00);

    // Equal-loudness calibration in the compressed domain. These
    // numbers are deliberately offsets, not multipliers alone: a
    // constant broadband hip-hop loop raises the low band by about
    // two log-loudness units over mid and four over high before
    // there is any useful colour information. Subtract that fixed
    // bias first, then give high a modest gain so hats/brilliance
    // can win instead of just tinting red.
    const float3 bandBias = float3(9.45, 7.75, 5.75);
    const float3 bandGain = float3(1.00, 0.82, 1.00);
    float3 calibrated = max((raw - bandBias) * bandGain, float3(0.0));

    // Loud, short columns with real low-band content are usually
    // kicks. Give those a modest low-band push so kick transients
    // stay pink/red instead of being pulled green by the broad
    // mid/presence bucket. Quiet strings/guitars do not trigger
    // this because their broadband peak is low and their low band
    // is weak.
    const float kickPush =
        smoothstep(0.18, 0.42, in.peak) * smoothstep(0.25, 1.10, calibrated.x);
    calibrated.x *= mix(1.0, 1.35, kickPush);
    calibrated.y *= mix(1.0, 0.78, kickPush);
    const float chromaMax = max(max(calibrated.x, calibrated.y), calibrated.z);
    float3 hue;
    if (chromaMax > 0.03) {
        const float3 weights = pow(saturate(calibrated / chromaMax), float3(1.45));
        float3 mixRgb = weights.x * lowColor
                      + weights.y * midColor
                      + weights.z * highColor;
        const float mixMax = max(max(mixRgb.r, mixRgb.g), mixRgb.b);
        hue = mixRgb / max(mixMax, 1e-6);
    } else {
        hue = float3(1.0);
    }

    // Serato's greyed details are quiet sub-bass/rumble columns,
    // not quiet midrange instruments. The broadband peak says
    // "quiet"; `subBass / rawMax` says "mostly below ≈ 80 Hz".
    // Requiring both keeps green guitar/string notes alive while
    // letting barely-audible low bed material recede.
    const float quiet = 1.0 - smoothstep(0.08, 0.20, in.peak);
    const float subFocus = smoothstep(0.62, 0.90, in.subBass / max(rawMax, 1e-6));
    const float audibleMidTop = smoothstep(0.20, 0.85, max(calibrated.y, calibrated.z));
    const float quietGrey = quiet * subFocus * (1.0 - 0.70 * audibleMidTop);
    hue = mix(hue, float3(0.56), quietGrey);
    const float finalBrightness = brightness * mix(0.30, 1.0, 1.0 - quietGrey);

    const float3 rgb = saturate(hue * finalBrightness);

    return float4(rgb, 1.0);
}

// MARK: - Beat grid (B-24 Metal pass)

/// Per-vertex data for beat-grid tick quads. Positions are already
/// in clip-space NDC; the vertex shader is a passthrough.
///
/// **M11d.6 round 15 — analytic anti-aliasing.** The original
/// shader returned a flat colour and relied on the quad's
/// geometric edges + MSAA 4× resolve to soften the line. That
/// breaks down when the line is moving faster than its own
/// width on a 60 Hz panel: each frame's "lit region" lands at a
/// different *integer-rounded* pixel set, with a visible gap
/// between consecutive frame positions that the eye reads as
/// "1 px jumps left/right" (sub-logical-pixel lines) or "the
/// grid line ghost-flashes" (super-logical-pixel lines).
///
/// The fix is to decouple the *visible* line width from the
/// *rasterised quad* width. The quad is widened on the host
/// side by 1 px in NDC so the shader has a fade region to
/// work in. The host writes a signed time-axis offset to
/// `signedTimeAxisDistNDC` on each vertex; that varies linearly
/// across the quad along the time axis (constant in the cross
/// axis). The fragment shader uses screen-space derivatives
/// (`fwidth`) to convert NDC distance to *pixels*, and applies
/// a `smoothstep` falloff that produces sub-pixel-perfect
/// coverage at every fractional line position. Combined with
/// MSAA 4×, the line's visible centre advances continuously
/// regardless of integer pixel boundaries.
struct BeatGridVertex {
    float2 position;
    float4 color;
    /// Signed distance from the line's time-axis centre, in
    /// NDC units. Varies linearly across the quad along the
    /// time axis (constant in the cross axis). Reaches
    /// ±`quadHalfNDC` at the quad's edges.
    float signedTimeAxisDistNDC;
    /// Half of the *visible* (post-AA) line width, in NDC. The
    /// alpha falloff starts where `|dist| > visibleHalfNDC`.
    /// Always ≤ `quadHalfNDC`.
    float visibleHalfNDC;
};

struct BeatGridVertexOut {
    float4 position [[position]];
    float4 color;
    float signedTimeAxisDistNDC;
    float visibleHalfNDC;
};

vertex BeatGridVertexOut beatGridVertex(uint vid [[vertex_id]],
                                        constant BeatGridVertex* verts [[buffer(0)]]) {
    BeatGridVertexOut out;
    out.position = float4(verts[vid].position, 0.0, 1.0);
    out.color = verts[vid].color;
    out.signedTimeAxisDistNDC = verts[vid].signedTimeAxisDistNDC;
    out.visibleHalfNDC = verts[vid].visibleHalfNDC;
    return out;
}

fragment float4 beatGridFragment(BeatGridVertexOut in [[stage_in]]) {
    // Analytical line-edge AA. `dist` is the absolute
    // NDC-space distance from this fragment to the line's
    // centre on the time axis. `fwidth` is the change in
    // `signedTimeAxisDistNDC` between adjacent fragments —
    // i.e. one pixel's worth of NDC distance — so the
    // smoothstep transition spans exactly 1 pixel regardless
    // of viewport size, orientation, or per-deck zoom level.
    // The MSAA pass on top of this gives a further factor of
    // 4 sub-pixel positions for the resolve, which lands the
    // line centre at the precise fractional position requested
    // by the host every frame.
    float dist = abs(in.signedTimeAxisDistNDC);
    float aa = fwidth(in.signedTimeAxisDistNDC);
    float alpha = 1.0 - smoothstep(in.visibleHalfNDC - 0.5 * aa,
                                    in.visibleHalfNDC + 0.5 * aa,
                                    dist);
    return float4(in.color.rgb, in.color.a * alpha);
}
