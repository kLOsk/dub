# Dub — Beat-Grid, Tempo and Waveform Spec

> **Sub-spec of [PRD.md](PRD.md).** Source of truth for everything BPM,
> beat grid, downbeat, tap-to-grid, and the visual contract between
> the grid and the waveform. Implementation lives in
> `crates/dub-bpm/`, `crates/dub-ffi/`, and `apple/Dub/Performance/`.
> Visual companion: [`docs/html/beats.html`](html/beats.html).

**Version:** 1.3 (Round 10 — integer-snap slack revert + deck-header Reset → fresh reanalyze)
**Date:** 2026-05-26
**Status:** Replaces PRD §8.3.1. PRD §8.3, §8.3.3, §9 (waveform) refer
back here for definitions and behaviour.

---

## 1. Why this spec exists

The previous tap-to-grid implementation conflated three distinct
concepts. That conflation produced a string of "doesn't make sense"
bugs: the 1-tap downbeat reset deleted grid lines from the start of
the track, "set the 1" pulled the yellow marker onto silence,
re-analysis trimmed the grid against pre-roll. All of those bugs
are symptoms of the same root cause: the data model fused two
fields that DJs treat as orthogonal.

This document fixes that. It establishes precise terms, the data
model, the user-action surface, the waveform-rendering contract,
the detection pipeline, the reliability gates, and how Dub
positions itself against Serato / rekordbox / Traktor on this same
problem.

Anything that contradicts this spec is a bug.

---

## 2. Glossary

The vocabulary is binding. Every term used by Rust code, FFI
contracts, Swift UI, log lines, error messages, calibration logs,
and bug reports refers back to a definition here. Synonyms are
listed only to disambiguate prior usage; new code must use the
canonical term.

| Term | Definition |
|---|---|
| **BPM** | Tempo in beats per minute. Scalar, positive. |
| **period** | Seconds per beat = 60 / BPM. Derived; never stored. |
| **beat** | One tick on the grid. A single point in time. |
| **bar** (also "measure") | A group of N consecutive beats. v1 hard-codes N = 4 (4/4 time). |
| **bar position** | Integer 1..4 identifying a beat's place inside its bar. |
| **downbeat** | A beat whose bar position is 1. DJ vernacular: "the 1". |
| **"the 1"** | Synonym for downbeat. Used in user-facing copy. |
| **beat anchor** | One known beat timestamp (seconds from sample 0) from which every other beat is derived: `beat[i] = anchor + i * period` for all integer `i` such that the result lies in `[0, duration]`. Historical: this was sometimes called "first beat", which is misleading — see §3. |
| **bar phase** (also "downbeat offset") | Integer 0..3 saying *which* beat in every group of 4 is the downbeat. Independent of `beat_anchor`. See §3. |
| **grid** | The full, uniform set of beat timestamps for one track. Always continuous. Always covers `[0, duration]`. |
| **drift** | Measured deviation between grid beats and audible transients, accumulating over time. |
| **grid lock** | User-blessed frozen grid. Absolute: no analyze, re-analyze, tap-tempo, or set-the-1 mutates a locked grid. The user must explicitly toggle Lock grid off before any edit. |
| **library row** | The active `track_beatgrids` row for a track. Single source of truth for BPM in both the library list cell and the deck header. See §4.5. |
| **waveform sidecar** | Pre-rendered peaks file at `~/Library/Caches/Dub/waveforms/{fingerprint}.wf`. Written by analyze passes, read on deck load so the waveform paints before the first frame. See §4.5. |
| **BPM confidence** | Scalar `[0, 1]` from the auto estimator; 0 = silence / no tempo. |
| **bar-phase confidence** | Scalar `[0, 1]` — how much the kick-band emphasis on one bar position out of 4 distinguishes it from the others. |
| **ODF** (onset-detection function) | Per-band spectral-flux signal that peaks at musical transients. The estimator works on the ODF, not the raw audio. |
| **noise floor** (snap) | The minimum ODF magnitude a transient peak must clear to count as "real" inside the snap window. Adaptive: a fraction of the track-wide p95 ODF. |
| **transient snap** | Refining a user-supplied time (tap or downbeat) onto the nearest real transient inside a bounded window. |
| **GridQuality** | Residual statistics (RMS, p95, max-abs, kept-fraction, drift slope) between the uniform grid and the ODF peaks. Drives the auto-lock heuristic and the ⚠ drift indicator. |

---

## 3. The data model

A beat grid is exactly three fields:

```text
struct BeatGrid {
    bpm:              f64,    // tempo, > 0
    beat_anchor_secs: f64,    // one known beat time
    bar_phase:        u8,     // 0..3 (which beat is the 1)
}
```

Everything else is derived. The grid is defined as

```text
beats = { beat_anchor_secs + i * (60 / bpm) for all integer i, clipped to [0, duration] }
downbeats = { beat | beat = beat_anchor_secs + i * (60 / bpm)
                     and i % 4 == bar_phase }
```

Two consequences flow from this and matter for every other section:

1. **The grid is continuous across the entire track.** Pre-roll
   silence, post-tail silence, breakdowns, drops — every section of
   the track gets beat ticks. The grid is what *would be* true if
   the music were playing; it does not depend on whether sound is
   actually audible at a given second.
2. **`bar_phase` is orthogonal to `beat_anchor`.** Setting the
   downbeat does not move beats. Moving beats does not change which
   ones are downbeats. Two independent user gestures, two independent
   fields. Conflating them was the root cause of the M11d.7 bugs.

### 3.1 What "the 1" actually means

"The 1" is *bar position 1*. In a 4/4 pattern, beats cycle through
positions 1, 2, 3, 4, 1, 2, 3, 4, ... A 32-bar track has 32 ones.
The DJ talks about "setting the 1" because they care about *which
of every four beats* lights up as the bar boundary — typically
chosen to align with the song's kick pattern or with the bass
phrasing of the genre being mixed.

"Setting the 1" never starts the grid from the tap. It rotates
which 1-in-4 beats are flagged as downbeats. The beats themselves
stay exactly where they were.

This is the same model Serato uses (the BAR function), rekordbox
uses (downbeat-only adjustment after BPM lock), and Traktor uses
(grid offset by ±beat).

### 3.2 What the grid is NOT

- Not a list of "audible beats." The grid is a uniform mathematical
  construct over the full track. Silence still has beats.
- Not a tempo curve. v1 has exactly one BPM per track. No rubato,
  no swing modeling, no half-bars, no time-signature changes. Per
  PRD §8.3.3, warp markers are parked for v2 (see §9).
- Not a downbeat-relative origin. `beat_anchor_secs` is one known
  beat's time. It happens to be representable as "the bar 1 closest
  to time T" but the model never relies on bar boundaries to derive
  beat positions.
- Not constrained to start at t = 0. The anchor can be anywhere
  inside the track; the grid walks back to t = 0 and forward to
  duration from there.

---

## 4. User actions

Exhaustive list. Anything not on this table is not a user action;
new requests must extend the table explicitly.

**Lock guard:** every grid-mutating action below (Auto analyze,
Re-analyze, Tap tempo, Set the 1, Nudge anchor, Nudge BPM) is a
**no-op when `grid_locked = true`**. The only action available on
a locked grid is Toggle lock. See §4.4 for the full lock-and-menu
contract.

| Action | Trigger | Effect on `bpm` | Effect on `beat_anchor` | Effect on `bar_phase` | Notes |
|---|---|---|---|---|---|
| **Auto analyze** | first deck-load of an un-analyzed track, or right-click → Analyze (when the track has never been analyzed) | Set from estimator | Set from estimator | Set from kick-band emphasis | Writes `auto` row in `track_beatgrids`; `is_active=1` unless a higher-priority row exists. |
| **Re-analyze** | right-click → Re-analyze (when the track has been analyzed at least once, and `grid_locked = false`) | Recomputed | Recomputed | Recomputed | Writes new `auto` row; demotes any active `user_tap` row so the new auto grid becomes active. |
| **Tap tempo** (3+ taps in one session) | deck-header BPM column (when `grid_locked = false` **and deck transport is playing**) | **Constrained re-analysis** in the tap-median ±15% neighborhood; result is the strongest real ODF peak in that range, integer-snapped if safe (§6.1) | Snap first tap to nearest transient with noise floor (§6.2) | Among the 4 candidate phases, pick the one whose downbeats minimize residual against the tap times | Writes new `user_tap` row; demotes any prior `user_tap` row for this track to `is_active = 0`. Each tap session is independent of all prior tap sessions (§4.6). Session window is dynamic (§4.2). |
| **Set the 1** (1–2 taps in one session) | deck-header BPM column (when `grid_locked = false`) — works in any transport state (playing, paused, cued) | Unchanged | Unchanged | Rotated: find existing beat nearest the tap; its `i % 4` becomes the new `bar_phase` | Writes new `user_tap` row (BPM and anchor copied from previous active grid); demotes any prior `user_tap` row to `is_active = 0`. |
| **Nudge anchor** | future v1.x (PRD §15 backlog) | Unchanged | `± ε` (typically 5 ms) | Unchanged | UI not in v1. |
| **Nudge BPM** | future v1.x | `± ε` (typically 0.1 BPM); anchor pinned to playhead so the visible beat doesn't jump | Adjusted to pin playhead beat | Unchanged | UI not in v1. |
| **Toggle lock** | right-click → Lock grid | Unchanged | Unchanged | Unchanged | Flips `tracks.grid_locked`. Only action available on a locked grid. |

### 4.1 Set the 1 — formal definition

Precondition: `grid_locked = false` (otherwise the gesture is a
no-op; the deck-header BPM column does not accept taps on a
locked grid).

Given a tap at time `t_tap`:

```text
period          = 60 / bpm
i_nearest       = round((t_tap - beat_anchor_secs) / period)
new_bar_phase   = i_nearest mod 4
```

Then `bpm`, `beat_anchor_secs`, and all beat positions are unchanged;
only `bar_phase` is updated. The beat at index `i_nearest` is the
new downbeat. The yellow tick in the renderer rotates by 0..3 beats.

Boundary cases:

- If the tap lands between two beats, ties break by choosing the
  earlier beat (deterministic; matches the "tap *on* the 1" reading).
- If the user taps wildly far from any grid beat (e.g. while the
  grid is badly wrong), this gesture is still semantically valid —
  it just sets the bar phase to whatever the nearest beat happens
  to be. The user can then tap-tempo to fix BPM or hit Re-analyze.

### 4.2 Tap tempo — formal definition

**Preconditions:** `grid_locked = false` AND deck transport is
playing. The grid-lock check is the standard guard (gate 4 in §7).
The transport check is new in this round: tap tempo against
silence has no audible reference for the user to keep their taps
consistent, no transient under the first tap to snap the anchor
to (§6.2), and no way for the user to verify the committed grid
without immediately starting playback. Set the 1 (1–2 taps,
§4.1) does not need this precondition because it's pure
bar-phase rotation that doesn't move beats and doesn't run
constrained re-analysis.

**Session window — dynamic.** A tap session opens on the first
tap of any 2 s burst. Subsequent taps belong to the same session
as long as they arrive within `max(1.5 s, 1.5 × median tap
interval so far)` of the previous tap. This is a *dynamic*
ceiling that follows the music:

| Tempo | Period | Window after each tap | 4-tap session total |
|---|---|---|---|
| 174 BPM (dnb) | 345 ms | ≥ 1.5 s | ≈ 1.0 s |
| 130 BPM (house) | 462 ms | ≥ 1.5 s | ≈ 1.4 s |
| 90 BPM (boom bap) | 667 ms | 1.5 s | ≈ 2.0 s |
| 75 BPM (slow reggae) | 800 ms | 1.5 s | ≈ 2.4 s — was timing out |
| 60 BPM (dubwise) | 1000 ms | 1.5 s | ≈ 3.0 s — was timing out |

The previous fixed 2 s window from M11c.3b cut off slow-music
users (any ≤ 90 BPM with a 4-tap minimum exceeded the ceiling),
which silently fired Set the 1 instead of tap tempo on
reggae / dub / slow hip-hop tracks. The dynamic rule fixes this
without making fast-music sessions less responsive.

**Tap-count dispatch.** A session closes when the window expires
without a new tap. At that point:

- 1–2 taps in the closed session → fire **Set the 1** (§4.1).
- 3+ taps in the closed session → fire **constrained
  re-analysis** (this section, below).

**Constrained re-analysis** is the heart of this section: tap
tempo is not tap-derived BPM substitution. The user's taps tell
the algorithm where to look; the algorithm tells the user the
precise answer. A 3–8 tap session will never beat a full-track
spectral-flux estimator on tempo precision (human reaction-time
jitter contaminates every tap edge by ~25 ms 1σ), so the taps
are a *hint*, never the answer.

Given tap times `t_1, t_2, ..., t_n` with `n ≥ 3`:

```text
tap_median        = weighted_median(60 / interval(t_1..t_n))   // ±0.5 BPM typical

# 1. BPM neighborhood. ±15% is comfortably wider than tap noise
# (~5 BPM at 100 BPM) and tight enough to exclude half/double
# (at ±50%/+100%) so an octave-error correction lands in the
# right octave.
search_range      = BpmRange::new(tap_median * 0.85, tap_median * 1.15)

# 2. Run the full estimator with the constrained range. The
# strongest real autocorrelation peak inside `search_range` is
# the answer. Integer-BPM snap fires when residuals don't get
# worse (§6.1).
(new_bpm, odf, kick_odf) = analyze_bpm_with_range(samples, search_range)
new_bpm           = snap_to_integer_if_safe(new_bpm, residuals)

# 3. Snap the first tap to the nearest significant kick within
# ±min(period/4, 70 ms). Falls back to the raw tap when the
# window is silent (§6.2).
new_anchor        = snap_to_nearest_transient(t_1, kick_odf, odf,
                                              noise_floor = 12% of p95)

# 4. Bar phase: among the 4 candidates, pick the one whose
# downbeats best fit the user's tap times. With n taps and an
# anchored grid, this is a min-residual search across 4 integers.
new_bar_phase     = argmin over phase in [0,1,2,3] of
                      Σ_i |t_i − nearest_downbeat(t_i, phase)|²
```

**What the user perceives:**

- Auto says 87 BPM (octave error). User taps near 174. Search
  range becomes 148–200. Algorithm picks the peak at 174.3 (or
  wherever the real peak is in that range). User's tap was a
  hint that excluded the wrong octave; auto-precision is preserved.
- Auto says 109.3 BPM. User taps 108 (small error). Search
  range becomes 91.8–124.2. The strongest peak in that range is
  still 109.3. Result: 109.3 unchanged. User's small misread
  doesn't disturb the good auto BPM.
- DnB tune detected at 174 BPM, user wants half-time reference
  (87). User taps near 87. Search range 73.9–100.0. Picks the
  peak at 87.0. Now both decks can mix against the half-time
  reference even though the auto picked the other octave.
- Breakbeat with bar 1 mis-placed by the auto kick-emphasis
  scorer. User taps the actual downbeats. BPM unchanged
  (constrained search confirms the auto BPM), but `bar_phase`
  rotates because the user's tap times fit a different phase
  candidate.

### 4.3 What "Set the 1" does NOT do

Stated explicitly because the previous implementation got this
wrong in four different places:

- It does NOT change `bpm`.
- It does NOT change `beat_anchor_secs`.
- It does NOT trim beats before the tap from the grid.
- It does NOT make the tap time the "first beat" of the track.
- It does NOT influence what the renderer treats as `beats[0]` —
  the renderer must not infer downbeats from beat-list position.
- It does NOT operate on locked grids — the tap is rejected.

### 4.4 Lock semantics and the right-click menu

Lock is absolute. When `grid_locked = true`:

- Auto analyze, Re-analyze, Tap tempo, and Set the 1 are all
  no-ops. The Rust engine rejects them; the Swift UI rejects them
  before the FFI call as a defense-in-depth measure (taps don't
  even queue, the menu items are disabled).
- The right-click menu's Analyze/Re-analyze item is visible but
  disabled, with a tooltip: *"Unlock the grid first."* This tells
  the user the option exists and points them at the gesture
  needed to enable it.
- Toggle lock is the only available grid action. The user
  explicitly unlocks, then runs the desired mutation, then
  (optionally) re-locks. Two clicks by design — combining
  "unlock and re-analyze" would hide the lock state change and
  break the audit trail in `track_beatgrids`.

The right-click menu uses one Analyze/Re-analyze entry whose
label is state-dependent:

| Track state | Label | Enabled? |
|---|---|---|
| Never analyzed (no row in `track_beatgrids`) | **Analyze** | Yes |
| Has analysis, unlocked | **Re-analyze** | Yes |
| Has analysis, locked | **Re-analyze** | No (tooltip: "Unlock the grid first") |

The previous parallel "Analyze" + "Re-analyze" entries are
removed; one item, one purpose, label switches on state.

### 4.5 Instant display contract — library row and deck header parity

The BPM rendered in the library row and the BPM rendered in the
deck header are **always the same value**, derived from the same
source of truth (the active row in `track_beatgrids` for the
loaded track). No code path may display one without the other,
display them out of sync, or show a stale value while the new
value is "loading."

Concrete consequences:

1. **Library has a BPM → deck header shows it instantly on load.**
   No nil-and-wait. No "displays after engine polls." The
   `preloadedGrid` snapshot returned by `Library.loadTrack(...)`
   is the deck header's initial value; the engine's grid is a
   live re-fetch but never produces a different number for the
   same `track_beatgrids` row.
2. **Auto analyze completes → library row updates → deck header
   refreshes.** Both writes are observable; the deck header's
   binding is the library row, not a separate engine snapshot.
   A track loaded into the deck *while* analysis is running
   gets the BPM as soon as the row is updated, with no extra
   user gesture.
3. **Tap-tempo commits → both library row and deck header
   update in the same frame.** No "deck header updates, library
   row stays stale until next selection."
4. **Re-analyze commits → both update.** Same rule.
5. **Lock toggle does not change BPM, so neither display changes.**
6. **During an active tap-tempo session, the deck-header BPM
   column shows a rolling preview** (Traktor pattern). From tap
   3 onward the displayed value is `60 / median(tap_intervals_
   so_far)`, updated after each new tap. The preview is visually
   distinguished (italic + accent-orange + a small "TAPPING ·
   N" badge) so the user knows this number is not the committed
   BPM. The library row remains at the previous committed BPM
   throughout the session; only the deck header shows the
   preview. On session commit (constrained re-analysis completes
   in §6.1), the deck-header preview is replaced by the final
   precise value AND the library row updates to match — both in
   the same publisher tick, per gate 14 in §7. If the session
   closes with only 1–2 taps, the preview disappears (no BPM
   change) and Set the 1 fires.

The waveform sidecar at
`~/Library/Caches/Dub/waveforms/{fingerprint}.wf` (PRD §8.4,
M10.5j) is the parallel guarantee for the visual: Analyze /
Re-analyze writes the sidecar; deck load reads the sidecar
synchronously and renders before the first frame. A track with
an existing sidecar must never show "waveform building…" on
load — that's a regression against this spec.

### 4.6 Idempotence — fixing a wrong tap by tapping again

The user's recovery path for any tap mistake is "tap again."
Each tap session is fully independent of every prior tap
session. Restated as a contract:

1. **The new session's tap median is the only BPM hint.** The
   previous `user_tap` row's BPM is not consulted, not blended
   in, not weighted, not used to bias the search range. The
   constrained re-analysis in §6.1 sees only the new tap times
   and the full-track ODF.
2. **The new session's first tap is the only anchor seed.** The
   previous `user_tap` row's `beat_anchor_secs` is not used as
   a fallback or a tie-breaker.
3. **The new session writes a new `user_tap` row.** The previous
   `user_tap` row for this track is demoted to `is_active = 0`
   in the same transaction. Only one `user_tap` row per track is
   ever active.
4. **Demoted rows stay in the table.** This is the audit trail.
   A future v1.x "Revert to previous grid" affordance can read
   prior rows back; v1 ships without that UI but the data is
   ready.

Worked example:

```text
t=0:    auto analyze → user_tap=∅, auto row active, BPM = 130.0
t=10s:  user taps 4× near 140 BPM (misheard the kick on hi-hat)
            → search 119–161, peak at 130 → user_tap row #1 written
               with BPM = 130.0, auto row demoted
            (notice: in this case the user's wrong tap landed on
            the same answer the auto already had, because 130 was
            in the search window. So nothing visibly changed for
            the user. This is fine.)
t=12s:  user taps 4× near 87 BPM (correcting an octave error)
            → search 73.9–100, peak at 87.0 → user_tap row #2
               written with BPM = 87.0, user_tap row #1 demoted
               (NOT consulted as a hint)
            (the auto's 130 reading is irrelevant; the new
            session's tap median defines the new search range)
```

The fact that this works correctly with zero state-management
glue in the algorithm is by design: `analyze_beat_grid_from_taps`
takes only `(samples, sample_rate, channels, tap_times,
profile)`. It does not take "previous BPM" or "previous tap
session" — there's no way for prior state to leak in. The only
plumbing the implementation needs is the database write that
demotes the prior row, which lives one level up in
`install_beat_grid_from_taps`.

---

## 5. Waveform integration

The waveform is the audio. The grid is an overlay drawn on top of
the waveform. They are visually coupled but independently sourced:
the waveform comes from `PeakBuffer` (per PRD §9, M10-B), the grid
comes from this spec.

### 5.1 Rendering contract

Serato-style rendering. The downbeat is the primary visual
reference for the DJ; beat ticks are a quieter rhythm aid. The
waveform itself is clipped vertically so neither marker is ever
hidden behind a tall transient.

| Element | Visual treatment | Source |
|---|---|---|
| Waveform vertical extent | Clipped to the inner ≈ 70 % of the canvas. ≈ 15 % top headroom is reserved for beat ticks and the downbeat extension; ≈ 15 % bottom headroom is reserved for the playhead chrome and drag handles. The waveform never touches the top or bottom edge — guarantees that grid markers are visible on every track regardless of peak loudness. | `PeakBuffer` |
| Beat tick | **Above the waveform only.** Short vertical tick in the top reserved band (≈ 1 px wide × ≈ 10 px tall, mid-luminance per `DubColor.gridBeat`). Does *not* cross the waveform. | All beats in `[0, duration]` derived from `(bpm, beat_anchor_secs)` |
| Downbeat marker | **Full-height vertical line through the waveform** from top reserved band through waveform body (≈ 2.5–3 px wide, accent-yellow per `DubColor.gridDownbeat`, with a soft outer glow at ≈ 18 % opacity). Higher visibility than beat ticks by design — the downbeat is the primary alignment landmark when mixing. | Subset of beats where `i mod 4 == bar_phase` |
| Beat-grid generation tag | Used by Metal renderer to invalidate cached tick geometry on grid edits | `beat_grid_generation` atomic per deck (FFI) |

The renderer **never** treats `beats[0]` as the downbeat. Downbeat
identity comes from `bar_phase` and the beat's index modulo 4.

**Why the asymmetry between beat tick and downbeat marker.** A
busy hip-hop track has a snare every half-bar, a hi-hat every
beat, percussion fills constantly. If every beat were a
full-height line through the waveform (the previous rendering),
the waveform becomes a forest of lines and the downbeat — the
*musically meaningful* alignment landmark — gets lost in the
clutter. The Serato pattern (beat ticks above, downbeat line
through) keeps the bar boundary clearly visible even on busy
audio. Trade-off: the downbeat line partially overlaps the
waveform peak at the kick, which is fine because the line is
*marking* the kick and the visual coincidence is correct.

### 5.2 Coverage rules

- Beat ticks render across the entire viewport, including silence.
- A track with 8 s of leading silence and a 0.5 s period has 16
  beat ticks visible in that pre-roll, one of which is yellow
  (the downbeat that the kick happens to fall on after the
  silence ends).
- A track that fades to silence at the end keeps beat ticks
  through the tail.
- The grid does **not** render beats with negative timestamps. If
  the anchor is at 8 s and the period 0.5 s, the renderer emits
  the 16 beats at `0.0, 0.5, ..., 7.5` (derived by walking back
  from the anchor) and does not emit beats at `-0.5, -1.0, ...`.

### 5.3 Drift indication

Per PRD §8.3.3, a track whose audible transients deviate > 5 %
from the grid over its length is non-uniform. When `GridQuality`
metrics cross the drift threshold (`drift_slope_ms_per_min ≥ 3`,
or `p95_ms ≥ 25`), the deck header shows the ⚠ "May drift"
indicator. The grid itself is still drawn uniformly — it's the
indicator's job to warn the DJ that the visual alignment will
degrade across the track, and the mitigation (pitch slider) is
on the turntable hardware.

### 5.4 Two-deck mode

Both decks render with identical horizontal seconds-per-pixel
scale. Beat ticks at the same time on each deck visually line up
when the playheads are in matching phase. The DJ uses the Phase-
Drift Trail (PRD §9, "single most opinionated visual design
decision") for fine-grain alignment; the grid ticks themselves
give the coarse-grain "are we on the bar boundary together?"
read.

---

## 6. Detection pipeline

End-to-end, what happens when auto-analyze runs on a fresh track:

1. **Decode** the track to mono `f32` samples (`dub-io`).
2. **Compute the spectral-flux ODFs** — broadband and kick-band —
   via the shared STFT pipeline (`dub-spectral`,
   `BpmEstimator::analyze_*_with_range_profile_and_odfs`).
3. **Estimate BPM** by harmonic-summed autocorrelation over the
   broadband ODF, with the perceptual tempo prior (PRD §5.2.3),
   skank rejection (M11c.3c), and genre-aware octave profile if
   the importer attached an ID3 genre.
4. **Find the beat anchor** by `score_grid` sweep over phases in
   `[0, period)`: the phase that maximises the sum of ODF samples
   at grid positions wins. Parabolic refinement gives sub-frame
   accuracy.
5. **LSQ refit** of `(bpm, anchor)` against the actual transient
   peaks in the first ~32 bars; produces `GridQuality` residuals.
6. **Integer-BPM snap** (M11d.7a): if the LSQ-refined BPM is
   within ±0.10 of an integer and the snapped-BPM residuals are
   no worse than ±3 ms RMS slack vs raw, snap to the integer.
   Common dance tempos (133, 140, 174) thus avoid 0.02 BPM grid
   drift over a 5-minute track.
7. **Determine bar phase** by computing kick-band energy at each
   of the four candidate downbeat positions; the position with
   the highest kick emphasis wins. Confidence = ratio of best to
   second-best.

The output of auto-analyze is exactly `(bpm, beat_anchor_secs,
bar_phase)` plus a `GridQuality` for the drift indicator and a
`downbeat_confidence` for the auto-lock heuristic.

### 6.1 Tap as hint for constrained re-analysis

Tap tempo runs the full auto pipeline with the search range
restricted to the tap-median neighborhood. The taps never decide
the BPM; they decide *which* peak the algorithm considers.

```text
TAP_SEARCH_RADIUS_FRACTION = 0.15   // ±15 % around the tap median

tap_median   = weighted_median(60 / interval(taps))   // outlier-filtered
search_range = BpmRange::new(
                  (tap_median * (1 - 0.15)).max(MIN_BPM),
                  (tap_median * (1 + 0.15)).min(MAX_BPM))

(bpm_raw, broadband_odf, kick_odf) =
    analyze_bpm_with_range_profile_and_odfs(
        samples, sample_rate, channels, search_range, profile)

bpm_final = snap_to_integer_if_safe(bpm_raw, residuals(samples, bpm_raw))
```

**Why ±15 %.** At 100 BPM, the window is 85–115. Human tap noise
is ~5 BPM 1σ in our tap-window length, so the user's *intent* is
solidly inside the window. The half-time peak (50 BPM) and
double-time peak (200 BPM) are far outside — an octave-error
correction cannot accidentally re-pick the wrong octave.

**Why no half/double special case.** The constrained search
handles octave errors structurally: if the user taps near the
*intended* octave, the wrong octave is excluded from the search
by construction. The previous `reconcile_tap_bpm_with_hint`
function (M11d.7 round 2) is removed entirely — it added a ±5 %
"keep the hint" band that pretended taps were a comparable
precision input, which contradicts §4.2.

**What this does for the four scenarios from §4.2:**

| Scenario | Previous behaviour (round 2) | New behaviour (this spec) |
|---|---|---|
| Auto = 109.3, user taps 108 (1.2 % off) | `|seed - hint| / hint = 0.012 ≤ 0.05` → keep hint 109.3 | Search 91.8–124.2, find peak at 109.3 → 109.3 |
| Auto = 87 (octave error), user taps 174 | `|seed - hint*2| ≤ 0.05` → integer-snap seed → 174 | Search 148–200, find peak at 174.3 → 174.0 after snap |
| Auto = 174 (DnB), user taps 87 (half-time) | `|seed - hint/2| ≤ 0.05` → integer-snap seed → 87 | Search 73.9–100, find peak at 87.0 → 87 |
| No prior auto, user taps 120 | `hint is None` → integer-snap seed → 120 | Search 102–138, find true peak (might be 121.5) → 121 after snap |

The new model is more principled (one rule instead of four
branches) and gives auto-quality precision in every scenario
because the algorithm always runs the full estimator.

### 6.2 Transient snap

Used in **tap tempo** to refine the first tap into a precise beat
anchor. Not used in **set the 1** (which is pure bar-phase
rotation, beats unchanged).

The snap window is bounded by `min(period/4, 70 ms)`. Inside that
window, the highest ODF peak above the noise floor wins;
parabolic refinement gives sub-frame accuracy. The noise floor is
12 % of the track-wide p95 ODF.

If no peak inside the window clears the floor, the snap returns
`None` and the caller uses the raw tap. This is the M11d.7 round 3
fix for "the 1 lands on a non-transient" — previously the snap
would land on whatever sub-noise grain was loudest.

### 6.3 Bar-phase selection — auto vs tap-driven

**Auto** (no tap input): for each of the 4 candidate downbeat
positions in the bar, compute kick-band energy at the candidate
downbeats vs the off-beats. The candidate with the highest
emphasis ratio wins; confidence = ratio of best to second-best.

**Tap-driven** (3+ taps): the user has just told us where their
downbeats are. For each phase candidate `p ∈ {0, 1, 2, 3}`,
compute the residual:

```text
residual(p) = Σ_i  |t_i − nearest_downbeat(t_i, anchor, period, p)|²
```

Pick `argmin_p residual(p)`. Kick emphasis is ignored — when the
user has explicitly tapped the bar, their intent is the final
word on which beat is the 1, not the kick-track's energy
distribution. (If the user tapped consistently *off* the kick,
that's a deliberate musical choice we honour. No "weak-beat
warning" surfaces; the spec trusts the tap.)

For **set the 1** (1 tap), the residual reduces to `|t_1 −
nearest_downbeat(t_1, ...)|`, which is exactly the "rotate to the
beat nearest the tap" rule in §4.1.

**Downbeats land on transients automatically.** No separate
transient-snap is needed for downbeats beyond the anchor snap in
§6.2. The reasoning: the anchor is snapped to a transient (the
nearest significant ODF peak to the first tap); the grid is
uniform, so beats sit at `anchor + i × period` for integer `i`;
downbeats are the subset where `i mod 4 == bar_phase`. When the
BPM is correct and the music is uniform, *every* beat coincides
with a transient (that's what "BPM is correct" means audibly),
so downbeats coincide with transients for free. If a downbeat
falls in pre-roll silence, that's the §5.2 coverage rule (grid
covers the whole track); it's correct, not a placement bug.

---

## 7. Reliability gates

Restated from PRD §1, §2.2.6, §8.3.3 and binding for the beat-grid
code paths specifically.

1. **Uniform 4/4 only in v1.** No warp markers, no tempo curves,
   no half-bars, no rubato. One BPM per track.
2. **The grid is the whole track.** No code path may trim beats,
   advance the anchor past pre-roll silence, or render fewer beats
   than `[0, duration]` would imply.
3. **`bar_phase` is independent.** No code path may infer the
   downbeat from `beats[0]`, from the "first audible" time, from
   the first tap (except in tap-tempo where the spec says so), or
   from anything other than the explicit `bar_phase` field.
4. **`grid_locked = true` is absolute.** No code path — analyze,
   re-analyze, tap-tempo, set-the-1, anchor nudge, BPM nudge — may
   mutate `bpm`, `beat_anchor_secs`, or `bar_phase` while
   `grid_locked = true`. The only legal grid action on a locked
   track is Toggle lock. The previous `force` re-analyze parameter
   is removed.
5. **Unlocking is an explicit gesture.** The user right-clicks →
   Lock grid (to unlock) before any other grid edit is accepted.
   Combined "unlock-and-do-X" affordances are not added; the lock
   toggle is its own observable event in the `track_beatgrids`
   audit trail.
6. **Re-analyze on an unlocked track demotes the active
   `user_tap` row** so the new `auto` grid claims `is_active = 1`.
   The user's previous tap-derived grid stays in the table for
   audit (it's a row, not a delete), it just stops being the
   active read.
7. **Each tap-tempo session demotes any prior `user_tap` row for
   the same track** and is fully independent of all prior tap
   sessions. The new session's tap median is the only BPM hint;
   the previous `user_tap` BPM, anchor, and bar-phase are not
   consulted, blended, or used as fallbacks. Only one `user_tap`
   row per track is active at a time. See §4.6.
8. **Tap input is rejected on locked grids before it reaches the
   FFI boundary.** Swift gates tap acceptance on
   `deckState.gridLocked`; the Rust engine independently rejects
   the call (defense in depth). The deck-header BPM column
   visually signals "tap disabled" while locked (cursor change,
   no tap-count overlay).
9. **Tap tempo (3+ taps) requires deck transport playing.**
   Tapping against silence has no audible reference for the user
   and no transient under the first tap to snap to. The
   deck-header BPM column accepts the first 1–2 taps in any
   transport state (they fall through to Set the 1, §4.1), but a
   third tap on a paused / cued / stopped deck is rejected
   silently (or with a brief "Press play to tap tempo" hint).
   Set the 1 (1–2 taps) is unaffected by transport state.
10. **Tap session window is dynamic.** A session stays open until
    no tap arrives for `max(1.5 s, 1.5 × median tap interval so
    far)`. Replaces the fixed 2 s window from M11c.3b that cut
    off slow-music users. See §4.2.
11. **Deck-header BPM column shows a rolling preview during an
    active tap session** from tap 3 onward (Traktor pattern).
    Library row stays at the committed BPM until commit. The
    preview is visually distinguished (italic + accent +
    "TAPPING · N" badge) so the user never confuses it with a
    final value. See §4.5 bullet 6.
12. **Lock toggle is intentionally a mouse-only / prep-mode
    gesture.** No keyboard shortcut. Lock is an "I'm done
    editing this grid" gesture; it belongs in prep, not in live
    performance, where it would only be reached during the kind
    of grid-editing crisis the spec is designed to make rare.
    The right-click → Lock grid menu item on the library row
    and on the loaded deck is the only entry point.
13. **Right-click menu uses one Analyze/Re-analyze entry** whose
    label and enabled state follow §4.4. The previous parallel
    entries are removed.
14. **Library row BPM and deck-header BPM are always in sync** for
    the loaded track (outside an active tap-tempo session — see
    gate 11). Single source of truth: the active
    `track_beatgrids` row. The deck-header binding is the library
    row publisher; the engine snapshot is a parallel live read,
    not an alternate source of BPM truth. See §4.5.
15. **Analyze pre-renders the waveform sidecar.** Auto analyze
    and Re-analyze both write
    `~/Library/Caches/Dub/waveforms/{fingerprint}.wf`. Deck load
    reads the sidecar synchronously and renders the waveform
    before the first frame paints. "Waveform building…" on a
    track with a sidecar is a regression.
16. **Serato-style rendering** (§5.1): waveform clipped vertically,
    beat ticks above the waveform only, downbeats as full-height
    yellow lines through the waveform. The renderer must respect
    the headroom so grid markers are never hidden behind tall
    peaks.
17. **Drift > 5 % gets the ⚠ indicator.** Pitch slider is the
    mitigation. Per PRD §8.3.3.
18. **Audio thread never touches grid edits.** All grid mutation
    happens off the RT path; the audio thread reads an immutable
    snapshot via the engine's atomic generation counter.

---

## 8. Industry comparison

Each app has shipped its own version of the "uniform 4/4 + tap"
contract. The differences are mostly about how much the GUI lets
the user push beyond uniform.

| App | Tempo model | Set-the-1 gesture | Warp markers | What Dub takes |
|---|---|---|---|---|
| **Serato** | Single BPM per track | Drag the BPM box; BAR key rotates downbeat | Yes (BAR markers post-2018) | Bar-phase model. Tap tempo + Set-the-1 split. |
| **rekordbox** | Single BPM + downbeat anchor | Auto-grid then memory-cue based correction | Yes (memory cue + BPM box per region) | First-beat anchor concept, but Dub uses any-beat anchor. |
| **Traktor** | Single BPM per track | Manual nudge keys for grid offset, ± beat | Limited (Auto-Grid Lock) | Hint-reconcile model (Traktor's "Set Beatgrid" preserves auto BPM when consistent). |
| **Engine DJ** | Single BPM + first downbeat | Drag in beatgrid editor | No | Reference for the read-only DJ-set-time UX (Engine forbids edits during performance). |
| **Dub (v1)** | Single BPM + arbitrary-position anchor + explicit bar_phase | 1 tap = pure rotation; 3+ taps = retempo + reset bar to first tap | **No (v2)** | Bias for keyboard-tap workflows; smallest user surface compatible with the urban/scratch DJ; integer-BPM snap on by default. |

What Dub does differently and on purpose:

- **No warp markers in v1.** The ⚠ indicator + pitch-slider
  mitigation is the contract. We don't pretend to fix non-uniform
  recordings; we tell the DJ they exist.
- **Set the 1 is pure phase rotation, never moves beats.** Serato's
  BAR key behaves the same way; rekordbox's "set first beat" can
  also move the anchor. We pick the Serato semantics because it's
  the one our target user (urban/scratch DJs) reaches for first.
- **Tap tempo is constrained re-analysis, not tap-derived BPM.**
  The taps tell the algorithm where to look (±15 % around the
  tap median); the algorithm tells the user the precise answer.
  Closest in spirit to Traktor's "Set Beatgrid" but Dub goes
  further: the auto estimator *always* runs (with a hint), so
  the tap path produces the same BPM precision as auto-analyze.
  Dub never treats the tap median as the BPM — the tap-window
  is too short (≤ 2 s, ~25 ms human reaction-time jitter) to
  beat a full-track spectral-flux estimator.
- **Integer-BPM snap with anchor refit.** Most commercial dance
  music sits on integer BPMs; a 133.02 BPM detection is the
  estimator's noise floor, not the song's truth. Snapping
  eliminates ~45 ms of visible grid drift over 5 minutes on
  affected tracks.
- **Lock is absolute.** A locked grid is a frozen contract.
  Re-analysis, tap-tempo, and Set the 1 are all rejected. The
  user must explicitly unlock first. We don't have a "force"
  bypass — silently working around a user-set lock is the same
  kind of trust violation as silently re-pitching a track.
- **Library / deck-header BPM parity.** One source of truth, two
  observers. No code path may show different BPMs in the library
  row and the deck header for the same loaded track. Most apps
  cheat on this (rekordbox shows the original BPM in the library
  while the deck shows the corrected one); Dub does not.

---

## 9. v2 considerations (not v1 scope)

- **Warp / flex markers.** Multi-anchor non-uniform grids for live
  recordings, vintage soul, drummer-played breakbeats. Per PRD
  §8.3.3, gated by user demand.
- **Time-signature support.** 3/4, 6/8, 12/8 for live reggae /
  certain hip-hop. The `bar_phase` model already allows any N
  beats per bar in principle; v1 hard-codes N = 4.
- **Tap tempo with octave preview.** Show the user candidate
  octave-corrected BPMs in the UI before committing.
- **Per-section BPMs.** Songs that legitimately change tempo (rare
  in our genres but possible in funk/Afrobeat/long live sets).

None of these are M14 dependencies. They're v2 conversations.

---

## 10. Implementation map

| Spec section | Owns | Lives in |
|---|---|---|
| §3 Data model | `BeatGrid` struct (Rust core), `BeatGrid` FFI struct, `DeckState.bpm/bar_phase` (Swift) | `crates/dub-bpm/src/beats.rs`, `crates/dub-ffi/src/lib.rs`, `apple/Dub/Performance/DeckState.swift` |
| §4 User actions | `analyze_beat_grid_*`, `latch_beat_grid_at_downbeat`, `analyze_beat_grid_from_taps`, FFI wrappers, `MainView.commitTapGrid` | `crates/dub-bpm/src/beats.rs`, `crates/dub-ffi/src/lib.rs`, `apple/Dub/MainView.swift` |
| §5 Waveform contract | `WaveformRenderer`, beat tick + downbeat coloring, `beat_grid_generation` invalidation | `apple/Dub/Performance/Waveform/*.swift`, `apple/Dub/Performance/Waveform/Shaders.metal` |
| §6 Detection pipeline | `BpmEstimator`, `analyze_bpm_with_range_profile_and_odfs`, `lsq_refit_grid`, `snap_bpm_to_integer_if_safe`, `snap_to_nearest_transient`, `bar_phase_from_taps` (new), `bar_phase_from_kick_emphasis` | `crates/dub-bpm/` |
| §7 Reliability gates | `analyze_track` (lock check, no `force`), `deactivate_user_tap_beatgrid`, `track_beatgrids` schema, library-row publisher, waveform sidecar reader on `loadTrack` | `crates/dub-library/src/analysis.rs`, `crates/dub-library/src/schema.rs`, `apple/Dub/Library/LibraryStore.swift`, `apple/Dub/Performance/Waveform/*.swift` |

---

## 11. Required code changes (driven by this spec)

Tracked as M11d.7 round 3. Each item is a bug against §3, §4, §5, or §7.

### 11.1 Data model and core (Rust)

1. **`BeatGrid` data model adds `bar_phase: u8`** as a first-class
   field in `crates/dub-bpm/src/beats.rs` and the FFI struct in
   `crates/dub-ffi/src/lib.rs`. Persisted in `track_beatgrids` as a
   new column (migration).
2. **`analyze_uniform_grid_from_odf`** stops advancing the anchor
   past pre-roll silence. The audible-start logic instead chooses
   `bar_phase` so the first kick after pre-roll is a downbeat;
   beats still cover `[0, duration]`.
3. **`latch_beat_grid_at_downbeat` is replaced** by
   `set_bar_phase_from_tap(grid, tap_secs)` which rotates
   `bar_phase` only. Existing `bpm` and `beat_anchor_secs` carry
   over unchanged. The FFI wrapper `relatch_beat_grid_at_downbeat`
   becomes `set_bar_phase`.
4. **`analyze_beat_grid_from_taps` becomes constrained
   re-analysis** (§6.1). Signature:
   `analyze_beat_grid_from_taps(samples, sr, ch, taps, profile)
   → BeatGridCore`. The previous `bpm_hint` parameter and
   `reconcile_tap_bpm_with_hint` helper are removed; the function
   builds a `BpmRange` from the tap median and re-runs
   `analyze_bpm_with_range_profile_and_odfs` end-to-end. Bar
   phase is chosen from `bar_phase_from_taps` (new helper
   minimising tap residual across 4 candidates). The function
   takes no "previous tap session" or "previous BPM" input by
   design — see §4.6, idempotence is enforced by the function
   signature.
4a. **`install_beat_grid_from_taps` (FFI / library) wraps the
    above with the row-management transaction**: write a new
    `user_tap` row with `is_active = 1`; demote any prior
    `user_tap` row for the same track to `is_active = 0` in
    the same transaction. Per gate 7 in §7, this is how "tap
    again to fix" works at the persistence layer.
5. **`synthesise_beat_grid` (FFI)** emits beats in `[0, duration]`,
   walking back from the anchor as well as forward. No trim.

### 11.2 Lock-is-absolute (Rust + Swift)

6. **`analyze_track(force: bool)` parameter is removed** from
   `crates/dub-library/src/analysis.rs`. New behaviour:
   `analyze_track` returns `Err(GridLocked)` if
   `tracks.grid_locked = true`. The caller (Swift) must ensure the
   track is unlocked before invoking.
7. **`install_beat_grid_from_taps` returns `Err(GridLocked)`** if
   the active grid is locked. Same rule for `relatch_beat_grid_at_
   downbeat` / `set_bar_phase`.
8. **Swift gates all grid-mutating calls on `deckState.gridLocked
   == false`** before crossing the FFI boundary. Tap input on a
   locked deck: no-op with a transient visual signal (cursor /
   tap-count overlay disabled).

### 11.3 Right-click menu (Swift)

9. **`LibraryView` context menu collapses Analyze and Re-analyze
   into a single item** whose `Text` is "Analyze" or "Re-analyze"
   based on `track.hasActiveBeatGrid`, and whose `.disabled(...)`
   modifier reflects `track.gridLocked`. Tooltip on disabled:
   "Unlock the grid first."
9a. **Tap controller becomes session-aware** with a dynamic
    window. The current 2 s constant in `TapToGridController`
    becomes `max(1.5 s, 1.5 × median_so_far)` evaluated after
    each tap. Sessions of 1–2 taps still fire Set the 1; 3+ fire
    constrained re-analysis.
9b. **Tap controller rejects the 3rd tap on a paused deck.** First
    1–2 taps are accepted in any transport state (Set the 1
    path); when a 3rd tap arrives, the controller checks
    `deckState.transportState == .playing` and either continues
    accumulating (playing) or drops the tap + emits a brief
    "Press play to tap tempo" hint (paused). The session window
    keeps running so the user can press play and continue.
9c. **Deck-header BPM column shows the rolling tap preview** from
    tap 3 onward, per §4.5 bullet 6. Visually distinct: italic,
    accent-orange, with a "TAPPING · N" badge in the corner of
    the BPM column. The Swift binding is `deckState.bpmDisplay`
    which is computed: returns `(60 / median).rounded()` while a
    session is active with 3+ taps, otherwise returns the
    committed BPM from the library row.

### 11.4 Library / deck-header BPM parity (Swift)

10. **`Library.loadTrack(...)` returns a `preloadedGrid` that the
    `DeckState` consumes as its initial BPM unconditionally.** The
    current branch (`supplied.source != "auto" || supplied.gridLocked`
    → drop and wait for engine snapshot) is removed. If the library
    has a BPM, the deck shows it.
11. **The library row publisher is the single source of BPM truth
    for both the library list cell and the deck header.** Both
    bindings subscribe to the same `LibraryTrackRow`; engine
    snapshots are validated against the library row, not displayed
    independently.
12. **Analyze / Re-analyze emits a library-row update event**
    inside the same write that updates `track_beatgrids`, so both
    bindings refresh in the same frame.

### 11.5 Waveform sidecar parity (Swift + Rust)

13. **Auto analyze and Re-analyze write
    `~/Library/Caches/Dub/waveforms/{fingerprint}.wf`** as part of
    the analysis pass, before returning. Re-analyze overwrites the
    existing sidecar.
14. **`DeckState.loadTrack` reads the sidecar synchronously**
    when present. The deck waveform must paint with sidecar data
    before the first frame; live `PeakBuffer` then takes over.
    Sidecar absent → fall back to the live build (current
    behaviour, only used for never-analyzed tracks).

### 11.6 Serato-style rendering (Swift + Metal)

15. **`WaveformRenderer` clips the waveform vertically** to the
    inner ≈ 70 % of the canvas (≈ 15 % top + 15 % bottom
    reserved). The amplitude-to-pixel map should respect this
    clip — drawing waveform peaks into the reserved bands is the
    bug §5.1 explicitly forbids.
16. **Beat ticks render in the top reserved band only**
    (`Shaders.metal` beat-tick path): ≈ 1 px wide × ≈ 10 px
    tall, mid-luminance, above the waveform body. The previous
    full-height beat ticks are removed.
17. **Downbeat markers become Serato-style full-height yellow
    lines** through the waveform body and into the top reserved
    band: ≈ 2.5–3 px wide, accent-yellow per
    `DubColor.gridDownbeat`, with an existing soft outer glow at
    ≈ 18 % opacity. Distinct from the beat tick by an order of
    magnitude of visual prominence — the downbeat is the primary
    alignment landmark, beat ticks are a quieter rhythm aid.

### 11.7 Tests

18. **Tests** for each of the above: continuous grid coverage,
    set-the-1 phase rotation, tap-tempo bar-phase via residual
    minimisation, drift indicator unchanged, lock semantics
    absolute (analyze/re-analyze/tap all rejected), library/deck
    parity (same value in both bindings after every action),
    sidecar written by analyze pass and read by `loadTrack`,
    dynamic tap window keeps sessions open at slow tempos,
    rolling tap preview replaces deck-header BPM but not library
    row, tap-tempo rejected on paused deck while set-the-1
    still works.

The M11d.7 round 2 transient snap with noise floor and the
M11d.7a integer-BPM snap already shipped stay valid; the
`reconcile_tap_bpm_with_hint` helper and the `force` parameter
are removed by this round.

---

## 12. Open questions

None blocking implementation as of v0.1. Captured for future
spec revisions:

- Should `set_bar_phase` allow the user to explicitly pick a bar
  position other than 1 (e.g. mark a beat as "the 2")? Useful for
  reggae one-drop alignment but ambiguous UX. Deferred.
- Should the renderer differentiate bar position 3 (the "&") from
  positions 2 and 4? Mid-luminance vs faint. v1 keeps all non-
  downbeats identical.
- Should there be an explicit "Reset bar phase" gesture (set
  `bar_phase = 0`)? Auto-analyze covers it; manual gesture not
  needed in v1.

---

## 13. v1.x follow-ups (out of scope for the round-3 implementation)

Captured from the May 2026 UX audit so we don't lose them. None
of these block v1 / round 3; each is a small follow-up.

| ID | Item | Rationale |
|---|---|---|
| F1 | **Bar-phase visual pip in the deck header.** A small visual indicator (no numbers) that advances 1 of 4 positions as the playhead crosses each beat. Lets the DJ see at a glance which beat-in-the-bar is currently playing, so two locked-and-matched decks at "130 BPM" with different `bar_phase` settings reveal the mismatch in the header rather than only on the waveform. Pip only; intentionally no numeric label. | Resolves the "two decks at 130 but bars don't line up" mystery (Tier-2 UX item #6). |
| F2 | **Define exact SLA for library row ↔ deck header sync.** Currently the spec says "always in sync." Implementation should commit both writes inside the same SwiftUI publisher tick AND the engine atomic snapshot should update within one audio-render-loop iteration of the DB commit. Spec gate 14 covers the user-visible contract; this item formalises the timing budget. | Removes ambiguity if a future change ever risks introducing a measurable lag. |

### Explicitly deferred / not adopted

- **"Revert to auto" right-click menu item.** Not needed:
  Re-analyze on an unlocked track already demotes any active
  `user_tap` row (gate 6) and replaces it with a fresh `auto`
  row. "Re-analyze" *is* the revert affordance.
- **Error toast for unsupported / corrupted files.** Not needed
  in v1.
- **LRU eviction for orphan waveform sidecars.** Not needed in
  v1. The cache is bounded by the size of the user's library and
  the user is expected to have GB-scale free disk space; sidecar
  bloat is a v1.x consideration if it ever becomes measurable.

### Shipped in round 4

The May 2026 round-4 implementation pass closed both structural
items that round 3 had deferred. Documented here so the changelog
in `SHIPPED.md` doesn't lose the cross-reference.

| ID | Item | Outcome |
|---|---|---|
| C1 | **Waveform sidecar.** `analysis_cache.waveform_sidecar_path` now has a writer. `dub-library::analyze_track` computes `OfflinePeaks` once after the BPM pass and writes a `.wf` sidecar via `dub_peaks::write_sidecar` (a `#[forbid(unsafe_code)]`-compatible field-by-field serializer). The path round-trips through `ActiveBeatgrid.waveform_sidecar_path` and `LibraryBeatGrid.waveform_sidecar_path` into `background_analyze_and_install`, which calls `dub_peaks::read_sidecar` on the load path and skips `compute_offline_peaks` on a cache hit. Magic-byte + endianness check on read; cache misses fall back to the on-load compute path. Test isolation lives on `Library::with_waveforms_cache_dir`; `open_in_memory` allocates a `tempfile::TempDir` per database so unit tests never touch the user cache. | Closes spec gate 13 (instant waveform display). |
| C2 | **`bar_phase` structural refactor (PRD-BEATS §11.1 items 1–3).** `BeatGrid` now carries `bar_phase: u8` as a first-class field; the SQLite `track_beatgrids` table has a `bar_phase INTEGER NOT NULL DEFAULT 0` column behind schema v5. `relatch_beat_grid_at_downbeat` has been renamed to `set_bar_phase` and rewritten as pure phase rotation (no anchor or beat-position changes). The new `dub_bpm::bar_phase_from_tap` helper picks the nearest existing beat and returns `i mod beats_per_bar`. Both renderer paths (Metal Beat-grid pass and SwiftUI Canvas overlay) now read `(idx mod beats_per_bar == bar_phase)` instead of `(idx mod beats_per_bar == 0)`. `upsertUserTapBeatgrid` takes `barPhase: UInt32` and `synthesise_beat_grid` round-trips the phase from library into the engine `BeatGrid`. | Closes §11.1 items 1–3 in full. |

### Round 4 follow-up — Oppidan regression triad

The user's first dogfooding session after C1 + C2 ship surfaced
three concurrent issues on the "Oppidan" track screenshot: the
first yellow downbeat was placed on bar 2's first kick instead
of bar 1's, no grid markers were drawn before that downbeat (the
pre-roll silence was completely empty), and every grid line sat
on the rising edge of the kick attack instead of inside the
visible amplitude lobe. The same May 2026 cycle delivers three
focused fixes in `crates/dub-bpm/src/beats.rs`:

> **Superseded by the universal-downbeat-fix (round 5).** O1's
> `first_kick_peak_secs`-as-primary rule was withdrawn after
> Baddadan exposed the failure mode: an ODF startup artifact at
> frame 0 reads as "the first kick" and anchors bar 1 inside the
> silent intro, dragging the entire grid one full beat (or four)
> off the body. Whole-track scoring decides phase now; the
> first-kick rule is retained as a low-confidence tiebreaker
> only. O2 (backward grid extension) and O3 (amplitude-peak
> snap) are unchanged and still ship. See "Round 5 — universal
> downbeat fix" below.

| ID | Issue | Fix |
|---|---|---|
| O1 | **First downbeat lands on bar 2.** `analyze_uniform_grid_from_odf` used to "walk the LSQ anchor forward by full bars until it crossed `audible_start`", which overshoots by exactly one bar whenever bar 1's kick sits less than a full bar past the pre-roll silence. | New `first_kick_peak_secs(kick_odf, …)` finds the parabolically refined peak of the first kick-band ODF lobe above the adaptive noise floor; that time becomes the chosen downbeat. The previous walk-forward loop is gone. Falls back to `find_downbeat_offset` only when the kick band is silent for the entire track. |
| O2 | **No grid before bar 1.** The same function used `beats.retain(\|t\| t >= anchor)` to drop every pre-roll beat, so the renderer had nothing to draw in the silence. | Filter removed. `uniform_beats(bpm, chosen_downbeat, duration)` already emits beats spanning the full track via its wrap-back-into-`[0, period)` rule; the C2 `bar_phase` field carries the downbeat identity explicitly, so pre-roll beats render as regular grey ticks while the yellow marker still lands on the first kick. The same change applies to `latch_beat_grid_at_downbeat` and `analyze_beat_grid_from_taps` for consistency. |
| O3 | **Grid line at attack edge, not amplitude peak.** Spectral-flux ODFs track the *derivative* of band magnitudes, so the peak fires during the rising edge of the kick attack — typically 5–25 ms before the visible amplitude peak that DJs use as their visual beat-match reference. Serato / Rekordbox / Traktor all render at the amplitude peak. | New `amplitude_peak_offset_secs` measures the median offset between each beat time and the strongest `|sample|` in `[t, t + 30 ms]` across the loudest 50 % of beats. `shift_grid_to_amplitude_peak` adds that single per-track delta to the anchor and recomputes `bar_phase` so the yellow marker tracks the shifted downbeat. Applied at the end of all three top-level analyzers; the LSQ `quality` and `bpm` fields are left untouched because the shift is uniform and the algorithm's internal residual measurement still references the ODF-peak grid. |

Regression tests live in `crates/dub-bpm/src/beats.rs` under the
"Oppidan regression triad" comment block:
`auto_grid_downbeat_lands_on_first_kick_not_second_bar`,
`auto_grid_extends_backward_into_pre_roll_silence`, and
`amplitude_peak_offset_pulls_grid_toward_visible_peak`. Two
extra unit tests pin the new helper:
`first_kick_peak_secs_finds_first_kick` and
`first_kick_peak_secs_returns_none_below_noise_floor`.

### Round 4 follow-up — "Set the 1" UX (Roni Size regression)

The next dogfooding session (Roni Size, "Brown Paper Bag", 174 BPM
DnB) surfaced four concurrent issues with the BPM-column
single-tap "set the 1" path. Two short-lived "fixes" landed first
(immediate-commit via `forceCommit`; an 80 ms reaction-time bias)
and were both withdrawn the same cycle — the immediate-commit
path leaked stale buffered taps when the user paused mid-window,
and the constant reaction-time bias is a category error because a
trained DJ taps accurately and the bias silently moves the marker
away from the click the user actually made. The shipping
solution combines three correct fixes:

| ID | Issue | Root cause | Fix |
|---|---|---|---|
| S1 | **Paused: press 1, yellow downbeat doesn't move until the user hits Play.** | The MTKView is paused on a paused deck (`continuouslyRendering = false`); `WaveformRenderer.refreshBeatGridIfNeeded` only runs when a frame is drawn. `persistTapGrid` bumped no Swift-side generation counters, so the on-demand `MTKView.setNeedsDisplay` trigger inside `WaveformMetalView.updateNSView` (which watches `seekGeneration` and `peaksGeneration`) never fired. The Rust engine had correctly rotated `bar_phase`; the renderer was still showing the *previous* bar's downbeat from the cached snapshot. Hitting Play resumed `CVDisplayLink`, drew a frame, picked up the new `beat_grid_generation_seq`, and the marker "magically moved" to the right place. | `persistTapGrid` now bumps `deck.seekGeneration &+= 1` synchronously after the engine call, matching the M11d.6 defense already in `persistNudgedGrid`. The next SwiftUI body re-evaluation invokes `updateNSView`, which calls `setNeedsDisplay`; the next vsync redraws the MTKView, refetches the grid via `engine.beatGrid(deckIdx:)`, and the Canvas overlay's `TimelineView` reads the updated `cachedBarPhase` out of the shared `renderSnapshot`. |
| S2 | **Paused: 1.5 s lag between pressing 1 and seeing any visual change. Also: paused taps stopped working after the first attempted fix.** | First attempt routed paused taps through `TapToGridController.forceCommit()` to bypass the 1.5 s idle window. But `forceCommit` flushes the *buffered* session — so a paused tap that arrived inside the still-fresh window of a prior playing-deck tap committed the stale playing-tap playhead time instead of the fresh paused tap. The user observed "doing the 1 on a paused song doesn't work anymore". | New `TapToGridController.commitSingleTap(playheadSecs:)` cancels any open session, clears the buffer, then dispatches exactly the caller's playhead through `onCommit`. `handleTapForGrid` calls it for every paused tap. The 1.5 s window stays in place for playing decks (where the 3+ tap upgrade to constrained re-analysis is legitimate). |
| S3 | **Playing: tap lands on a grid beat in a flat region instead of the obvious kick a few ms away.** | `bar_phase_from_tap` snapped purely to the nearest *grid beat* with no audio awareness. When the closest grid intersection is offset from the closest audible kick by more than the user's tap jitter, the marker lands in dead air. The screenshot from the report showed exactly this: a clear kick visible a few ms past the click, ignored in favour of an empty grid line. | `DubEngine::set_bar_phase` now snaps `tap_secs` to the strongest LF-filtered amplitude peak inside ±half-period before the nearest-grid-beat rotation. The LF (kick-band, time-domain) peaks are already cached on `FilePeaks.filtered` from the offline-peaks pass — no new analysis, no caching infra, no audio-thread allocations. The snap window is the hard cap; the marker can never cross into a neighbouring beat's kick. Genuine silence (no chunk above the −26 dBFS noise floor) falls back to the raw tap so a deliberate silent-region click is preserved. |
| S4 | **(Withdrawn)** 80 ms reaction-time + output-latency bias on playing-deck taps. | Posited that human reaction time exceeds half-period at fast tempos. | Removed in the same cycle. DJs tap accurately to the kick they hear; a constant bias is unprincipled and the actual fix for the original "marker one beat late" complaint is the S3 transient snap, which absorbs small perceptual offsets purely from the audio itself. |

Test coverage for S3 lives in `crates/dub-ffi/src/lib.rs` under
the kick-snap helper comment block:
`snap_tap_to_kick_finds_strongest_peak_in_window`,
`snap_tap_to_kick_returns_raw_tap_in_silence`,
`snap_tap_to_kick_cannot_cross_search_window`,
`snap_tap_to_kick_clamps_to_chunk_array_bounds`,
`snap_tap_to_kick_returns_raw_tap_on_empty_chunks`,
`snap_tap_to_kick_returns_raw_tap_on_zero_sample_rate`,
`snap_tap_to_kick_returns_raw_tap_on_nonfinite_inputs`,
`snap_tap_to_kick_respects_noise_floor_just_below`,
`snap_tap_to_kick_respects_noise_floor_just_above`.

### Round 5 — universal downbeat fix (Baddadan and the rest of the corpus)

The next dogfooding session surfaced "Baddadan" (Chase & Status,
DnB, 175 BPM): the auto grid placed bar 1 at 0.0275 s, in the
silent pre-roll. The diagnose tool (`dub diagnose`) traced the
failure to a structural property of the round-4 rule: the
spectral-flux ODF has a startup artifact at frame 0 (no
previous frame to differ against), which `first_kick_peak_secs`
picked as "the first audible kick" above the adaptive noise
floor. That single phantom dragged bar 1 onto silence even
though hundreds of body kicks said phase 0 (or whichever) was
correct. O1 was treating a global property (which of the four
beats-per-bar is bar 1) as a function of a single early ODF
sample.

The fix demotes `first_kick_peak_secs` from primary to
tiebreaker, routes phase decisions through whole-track scoring,
and hardens that scoring against quiet intros / outros and
silent breakdowns. The sub-beat `anchor_secs` from
`refine_full_pipeline` (LSQ over hundreds of onsets) is
untouched: only the bar-phase rotates. O2 (backward grid
extension) and O3 (amplitude-peak snap) still ship as round-4
delivered them.

| ID | Issue | Fix |
|---|---|---|
| U1 | **Whole-track phase score discarded.** The round-4 O1 rule used `first_kick_peak_secs(kick_odf, …)` as the chosen downbeat and only borrowed `find_downbeat_offset`'s `confidence` value. Two failure modes followed: (a) ODF startup spike at `t=0` reads as the first kick and the entire grid drifts; (b) a one-bar pickup beat in the pre-roll dominates over 100+ body bars. | `analyze_uniform_grid_from_odf` always calls `find_downbeat_offset` now, computes `chosen_downbeat = anchor_secs + offset * period`, and ignores `first_kick_peak_secs` outright in the production path. The structural property "which beat is bar 1" is decided by the body of the track, not by a single ODF sample. The function and its tests stay for use inside the tiebreaker (U3). |
| U2 | **Intro / outro contamination of the phase score.** `score_grid` summed `kick_odf[phase + n * bar_period]` over the whole track, so a phantom ODF spike at `t = 0` contributed to every one of the four phase candidates almost equally, and a long silent intro / breakdown leaked low-energy votes that diluted the body's signal. | New `score_grid_weighted(kick_odf, broadband_odf, phase, bar_period, body_start, body_end)`: skips beats outside `[body_start, body_end]` (default ±8 s, capped at 25 % of duration on each side so short clips still get scored over their middle half), and weights each kick value by the peak broadband ODF inside a tenth-of-a-bar window around the candidate downbeat. Silent bars contribute ~zero regardless of phase; loud body bars dominate naturally. The unweighted `score_grid` is retained for period refinement (`refine_full_pipeline`) and parabolic peak interpolation, which both want the simple shape. |
| U3 | **Genuinely ambiguous body scores need a tiebreaker.** Click tracks, dnb fills, and other "every beat is identical" patterns produce all four phase scores within a few percent of each other. The existing `kick_only_intro_tiebreaker` only fires when `non_kick = broadband - kick` is meaningfully non-zero (it abstains on featureless click tracks). | New first-kick proximity tiebreaker inside `find_downbeat_offset`: when `confidence < FIRST_KICK_TIEBREAK_CONFIDENCE` (1.05, i.e. top-2 within 5 %) AND `kick_only_intro_tiebreaker` returned `None`, call `first_kick_peak_secs` and rotate to the phase whose first beat sits nearest the detected first kick (computed modulo bar period, so it works regardless of where in the track the kick lands). Preserves the user's mental model — "the first audible kick is the 1" — without letting it override 100+ bars of body evidence. |

After the fix, Baddadan's auto downbeat moves from 0.0275 s to
1.2482 s — inside one beat of the first audible kick, and
within 6.5 ms of the anchor the user had previously stamped on
the track via "set the 1". Bredren Evacuation, the other track
the user complained about, gets downbeat 0.0729 s vs the user's
locked anchor 0.0664 s (6.5 ms again — same alignment quality).

The round-4 O1 helper rationale ("first audible kick is bar 1
by convention") still drives the user's mental model and stays
in the spec as a tiebreaker, but is no longer the structural
load-bearer of phase decisions.

Regression tests live in `crates/dub-bpm/src/beats.rs` under the
"universal-downbeat-fix regression suite" comment block:
`quiet_intro_then_loud_body_picks_body_phase` (Baddadan
synthetic),
`one_bar_pickup_picks_body_downbeat` (pickup-beat regression),
`score_grid_weighted_excludes_intro_outro_bars` (unit test on the
new weighted scorer),
`first_kick_tiebreaker_rotates_phase_when_body_scores_tie`
(direct unit test on the new U3 tiebreaker). Two existing
"Oppidan triad" tests
(`auto_grid_first_beat_lands_in_audible_content_not_pre_roll`,
`auto_grid_extends_backward_into_pre_roll_silence`) were
re-pointed at the underlying user-facing property they were
guarding (downbeat on the click lattice; grid extends through
pre-roll silence) since the previous "first kick = bar 1"
assertion no longer matches the design.

The `dub diagnose` subcommand (`crates/dub-cli/src/diagnose.rs`)
is the go-to debugger for this class of issue: takes a path or
track ID, prints the fresh BPM + bar_phase + fit quality, an
independently computed broadband + low-band envelope with
amplitude landmarks, a verdict line ("downbeat sits within
±half a beat of the first audible transient"), and every
`track_beatgrids` row from the live SQLite library so you can
compare the algorithm's current choice against the user's
historical tap corrections.

### Round 6 — set-the-1 phase contract, tap-as-hint, octave self-verification

The dogfooding session after Round 5 surfaced two new failure
modes on different tracks:

1. **Excel Apocalypse (Drum & Bass, 177.7 BPM)**: auto-analyse
   gives a tight 177.7 BPM grid, but the visible 1 sits off
   the kick. The user opens the deck, taps once on the first
   kick to set the 1, the marker lands on the kick — and the
   displayed BPM JUMPS from 177.72 to 178.70, dragging the
   rest of the grid forward so every beat past the 1 sits ~1
   ms further off the kick than it did before. By the end of
   the track the grid has drifted ~2.2 s out of phase. The
   user reported this as "setting the 1 makes the rest of the
   grid worse".
2. **Bangin' Westside Connection (Hip-Hop, ~88 BPM)**: auto-
   analyse picks 175.7 BPM (the hi-hat / snare grid octave).
   The user starts tapping at the perceived 88 BPM kick
   tempo; the displayed BPM jumps to 86.232 BPM (the tap
   median, jittered), and the grid lands on the kick — but
   drifts visibly across the track because 86.232 isn't the
   real tempo of the recording. The user reported this as
   "tapping is an absolute mess; first analysed at twice the
   speed, then turned into something completely uncorrelated
   to the actual beat".

The `dub diagnose` tool exposed the three structural causes
that produced these two reports between them:

| ID | Issue | Fix |
|---|---|---|
| **6a** | **`latch_beat_grid_at_downbeat` refits BPM.** The function called `refine_period_at_anchor` (±1 % spectral score search) followed by `lsq_refit_grid` with `anchor_fixed = true` (joint OLS on slope = 1/period). Both moved BPM despite the documented "set-the-1 is a phase assertion" contract. On Apocalypse the LSQ found a marginally tighter local fit at 178.70 and committed to it; net result was a phase-perfect "1" sitting on top of a BPM-wrong grid. | Both calls removed. `latch_beat_grid_at_downbeat` now keeps the deck's BPM bit-identical to its input; only the anchor moves. `measure_grid_quality` (hold-both-fixed) still measures residuals at the new anchor so the drift indicator stays meaningful for the user-supplied grid. Regression: `latch_preserves_input_bpm_even_when_lsq_disagrees` asserts that passing a deliberately-wrong 120.7 BPM with a click track at 120.0 produces grid.bpm == 120.7 exactly, AND that the drift indicator surfaces the systematic mismatch. |
| **6b** | **Tap median taken verbatim.** The M11d.7a override of the original PRD-BEATS §6.1 contract had `analyze_beat_grid_from_taps` set `bpm = weighted_median_bpm_from_taps(taps)` and stop. Tap jitter (~5–15 ms / tap) baked directly into the persisted BPM as ~1–3 BPM noise; clean taps at the perceived 87 BPM tempo produced a noisy 86.232 BPM that drifted off the kicks. | Tap median is now a HINT for a narrow ±3 % LSQ search (`refine_bpm_around_tap_hint`). The search grid is quantised to 0.1 BPM so integers and half-integers are always sampled, and the winner is selected by `(kept_fraction >= 0.95 * max_kept, lowest rms_ms)` to defeat the `LSQ_MIN_PEAK_RATIO` artifact where slightly-wrong BPMs drop "hard" beats and report misleadingly tight RMS on the easy ones. Finally `snap_bpm_to_integer_if_safe` snaps to the nearest integer when it doesn't worsen the fit. Window (±3 %) is narrower than the ≥10 % gap to any neighbour metric level, so the search cannot drift to a different musical interpretation. Regressions: `tap_grid_refines_to_integer_within_search_window` (jittered taps at 133 BPM converge to exactly 133.0), `tap_grid_keeps_non_integer_bpm_when_integer_doesnt_fit` (drum pattern at 87.8 BPM stays at 87.8, not 88). |
| **6c** | **Octave decision is spectral-only.** Pass 2 in `tempo.rs` picks between octave candidates based on harmonic-summed autocorrelation energy. On busy-hat genres (hip-hop, R&B) the 16th-note hat / shaker layer wins on spectral energy even when an 88 BPM kick grid fits the actual ODF onsets much tighter. The genre profile system is the first line of defence but only helps tagged tracks. | Profile-independent octave self-verification (`octave_self_verify`) runs unconditionally after `snap_bpm_to_integer_if_safe`. For each of `bpm / 2` and `bpm * 2`: try both possible sub-phases at the half octave (anchor and anchor + main-period — the wrong sub-phase lands every beat on a snare instead of a kick, destroying the fit), AND inside each sub-phase run a narrow ±2 % LSQ search to absorb the gap between pass-2's sub-integer spectral pick and the real tempo's integer (Bangin': pass 2 picks 175.742, strict half is 87.871, the truth is 88.000 — 0.13 BPM off the strict half; without the BPM search the half RMS measures 73 ms where the true 88.0 fits at 10 ms). Swaps to the alternate octave when `rms_alt < 0.65 * rms_main` AND `kept_alt >= kept_main` AND `kept_alt >= 0.5`. Strict thresholds on purpose — only swap when "the algorithm picked the hat grid instead of the kick grid" is the only plausible explanation. Skipped when `fixed_anchor.is_some()` (caller has asserted intent). Integer-snapped after swap. Regressions: `octave_self_verify_swaps_when_alternate_fits_materially_tighter` (90 BPM click, lied main = 180 BPM, swaps back to 90), `octave_self_verify_keeps_main_when_quality_is_already_tight` (tight 120 BPM main is not swapped to 60), `octave_self_verify_respects_fixed_anchor_bypass` (tap-driven caller is left untouched). Real-world: Bangin' Westside Connection (Hip-Hop, untagged Default profile) swaps from 175.742 → 88.000 BPM with rms improving from 41 ms to 10 ms. |
| **6d** | **No `HipHop` octave profile.** `octave_profile_from_genre("Hip-Hop")` mapped to `Default`, so the profile system contributed nothing on hip-hop / rap / trap / R&B / boom-bap tags. Bangin' is tagged "Hip-Hop"; the spectral winner at 175.7 was kept because no genre rule fired. | New `OctaveProfile::HipHop` variant. Genre matcher covers `"Hip-Hop"`, `"Hip Hop"`, `"HipHop"`, `"Rap"`, `"Trap"`, `"R&B"`, `"R & B"`, `"RnB"`, `"Boom Bap"`, `"Boom-Bap"`, `"Boombap"`, and composite tags like `"Conscious Hip-Hop"`, `"West Coast Rap"`. Lower-octave bias via `profile_doubletime_rejected`: the upper octave (135–180 BPM) is rejected when paired with a credible 70–95 BPM peer (raw ≥ 70 % of upper). Less strict than the unconditional M11c.3e `hiphop_doubletime_rejected` (which requires ≥ 96 %) because tagged hip-hop has more evidence; the lower threshold catches the structural cases that the unconditional rule misses. Regressions: `genre_mapping_covers_hip_hop_family`, `hiphop_profile_rejects_bangin_shape`. |
| **6e** | **No `DrumAndBass` octave profile.** DnB / jungle tagged tracks could half-time-flip to ~85 BPM when the snare backbeat produced a strong autocorrelation peak at the 2-beat period (the same K-S-backbeat half-time problem the rolling-DnB regression test documents). Nothing in the profile system prevented it for tagged DnB. | New `OctaveProfile::DrumAndBass` variant. Genre matcher covers `"Drum & Bass"`, `"Drum and Bass"`, `"Drum n Bass"`, `"Drum'n'Bass"`, `"drumandbass"`, `"DnB"`, `"D&B"`, `"Jungle"`, and composite tags like `"Liquid DnB"`. Inverse-of-hip-hop logic via new `profile_halftime_rejected`: the LOWER octave (70–95 BPM) is rejected when paired with a credible 135–180 BPM peer (raw ≥ 70 % of lower). DnB is matched BEFORE the hip-hop branch so composite tags like `"hip-hop jungle"` resolve to DnB tempo (the user's mix tempo for the fusion material). Regressions: `genre_mapping_covers_drum_and_bass_family`, `drumandbass_profile_rejects_inverted_halftime`. |

Rationale for the layering. The three fixes work at different
levels of authority:

- **6d / 6e (profile rules)**: genre-tagged signal "this is
  hip-hop, prefer the lower octave". Highest authority because
  the user labelled the file.
- **6c (self-verification)**: profile-independent measurement
  "at this anchor, BPM X fits the ODF measurably tighter than
  BPM 2X". Catches untagged tracks plus edge cases the profile
  rules don't know about.
- **6a / 6b (caller contracts)**: explicit user intent "this
  is the 1" (6a) / "this is the tempo I'm tapping at" (6b).
  Highest authority of all — the user is in front of the deck
  watching the grid; we trust them.

`octave_self_verify` deliberately skips when `fixed_anchor` is
supplied so 6c can never overrule 6a / 6b.

### Round 7 — amplitude-peak cheat: leading edge of the loud region

After Round 6 the user reported one more class of "grid is in the
wrong place" complaint, this time about the **visible** alignment
between the grid line and the kick rather than the BPM or the bar
phase:

> "Can you check Blaze Up Tha Dance please. I set the 1 and it
> put it behind the peak. Generally I feel the grid always sits
> a bit behind the peak of the transient. I think we need to
> adjust our cheat a bit to really sit at the peak of the
> transient, otherwise users are confused."

When asked to be precise about "the peak", the user clarified:

> "The right position would be where the transient is visually
> the largest. If the transient is long at same loudness, the
> grid would sit right at the beginning where its the loudest in
> the waveform."

That clarification is doing real work. It splits the problem into
two cases that the previous cheat collapsed into one:

1. **Single sharp peak** (most kicks, most snares). The visibly
   tallest point is a well-defined sample: `argmax |x|`. The
   grid should sit on it.
2. **Sustained loud region** (compressed kicks, sub-bass tails,
   dub one-drops, continuous high-energy material like loop-
   driven house). There is no single tallest point — a flat
   plateau of near-max samples. The grid should sit at the
   **leading edge** of that plateau, because that's where the
   user's eye registers the kick "starting".

The Round 4 `amplitude_peak_offset_secs` returned `argmax |x|`
across the search window. On compressed material `argmax` lands
at the very front of the attack lobe — the highest needle, not
the fattest body. Visually that reads as "the grid sits behind
the peak", matching the user's report. On continuous loop-driven
audio `argmax` lands at whichever local hot spot happens to
spike highest, with no guarantee that it represents the
musically meaningful beat anchor.

Two failures, two fixes (with the same helper underneath):

| ID | Issue | Fix |
|---|---|---|
| **7a** | **Amplitude-peak finder returns a single sample.** `amplitude_peak_offset_secs` recorded `(peak_amp, offset_secs)` per beat using `argmax |sample|` in `[beat, beat + 90 ms]`, then took the median offset across the top 50 % loudest beats. On a kick with a sharp leading transient followed by a sustained body, `argmax` lands in the first 1–3 ms of the attack — visually that's the leading edge of the kick, NOT the body the user perceives as "the peak". On continuous loop material it picks an arbitrary peak inside a near-max region, varying beat-to-beat in ways the median diffuses. | New `amplitude_peak_for_beat` helper. Computes a backward 1.5 ms `max |sample|` envelope (matches the waveform display's 64-sample chunking ≈ 1.45 ms at 44.1 kHz, so "the user's tallest chunk" perception lines up with the algorithm's notion of "loud"). Finds the env's global max position, then walks backward from there as long as the env stays within 5 % (`AMPLITUDE_PEAK_NEAR_MAX_FRACTION = 0.95`) of that max. Returns the earliest sample of the contiguous near-max region containing the max. For a sharp peak this is the peak itself (the walk stops immediately on the attack slope). For a sustained loud region this is its leading edge. For continuous material it anchors on the actual loudest moment in the window rather than the first sample where env crosses some threshold. `amplitude_peak_offset_secs` (auto-path median) is rewritten as a thin wrapper that calls `amplitude_peak_for_beat` per beat and keeps the existing top-50 %-by-amplitude median. Regressions: `amplitude_peak_for_beat_lands_on_sharp_impulse`, `amplitude_peak_for_beat_lands_on_slow_attack_body_peak`, `amplitude_peak_for_beat_lands_on_leading_edge_of_flat_loud_region`, `amplitude_peak_for_beat_returns_none_for_silence`. |
| **7b** | **"Set the 1" diffused the per-beat shift through the all-beat median.** `latch_beat_grid_at_downbeat` ended with `shift_grid_to_amplitude_peak`, which uses the all-beat median offset across the entire track. The user explicitly tapped on *this* visible peak; deferring to a track-wide median means the anchor sits at the typical attack-to-peak time, not at the visible peak of the specific kick the user pointed at. On Blaze Up Tha Dance the median undershot by ~5 ms — exactly the "grid behind the peak" report. The auto path's median is correct in spirit (no single beat is privileged in auto), but for set-the-1 the privileged beat IS the entire user intent. | `latch_beat_grid_at_downbeat` now calls `amplitude_peak_for_beat` for the **snapped tap alone** and anchors the grid at the result. No call to `shift_grid_to_amplitude_peak`. The visible downbeat lands at the leading edge of the near-max region containing the tap, every time, with no track-wide averaging in between. `measure_grid_quality` is still measured against the ODF-aligned `snapped_downbeat` (the spectral-flux ground truth for fit residuals — the visible shift moves only the rendered phase, not the algorithm's residual structure). Regression: `latch_anchor_lands_at_visible_peak_not_at_track_median` — a click track with a single slow-attack kick spliced in at bar 5; the rest of the track's beats have per-beat amp-peak offset ≈ 0 (clicks peak at the impulse), so the all-beat median is ≈ 0; the spliced kick's visible peak is at +5 ms past the click position; the test asserts the latched downbeat lands within ±2 ms of the +5 ms visible peak, which the old all-beat median could not satisfy. |

Rationale for `AMPLITUDE_PEAK_NEAR_MAX_FRACTION = 0.95`. 0.95
of max amplitude is within 0.45 dB of the peak — tight enough
that a single sharp peak's sole near-max sample IS the peak
itself, loose enough that a real sustained loud region with
mild internal variation registers as one contiguous block whose
leading edge we can latch. Lower values (e.g. 0.90 = within
0.92 dB) start stitching together adjacent peaks that the user
perceives as separate transients; higher values (e.g. 0.98 =
within 0.18 dB) shrink the near-max region down to noise
fluctuations on top of the actual peak.

Rationale for `AMPLITUDE_PEAK_SMOOTH_WINDOW_SECS = 0.0015`.
The waveform display aggregates 64 samples per chunk (≈ 1.45 ms
at 44.1 kHz, ≈ 1.33 ms at 48 kHz). The user's "tallest chunk"
perception is inherently a 1.5 ms-wide max. A backward envelope
of the same window length matches what the user sees while
keeping the leading-edge detection unbiased: backward-max env
reaches its peak value at the **exact** position of the actual
peak sample (it does not overshoot the way a forward or
centered window would).

Rationale for `walk-backward-from-max` (instead of "earliest
near-max in the whole window"). Continuous high-energy material
(Blaze Up Tha Dance is the dogfood track) has an env that's
already within 95 % of its eventual max at the search start —
"earliest in window" always returns offset 0 there, sitting the
grid at the leading edge of the search window regardless of
where the actual loud region is. Anchoring the walk at the
global-max position guarantees the result sits on a sample that
IS the loudest, and walking backward finds the leading edge of
THAT loud region. Sharp peaks: walk stops on the attack slope
within a sample or two. Sustained loud regions: walk continues
to the leading edge of the plateau and stops when env dips below
threshold. The contiguous-only walk also prevents two separate
loud regions from being stitched together — if there is a real
gap between regions (the smoothing window will not hide drops
longer than 1.5 ms) the walk stops at the leading edge of the
region containing the global max.

Layering with previous rounds. 7a / 7b only change WHERE the
grid renders (the rendered phase), not WHICH beats the algorithm
selected (the LSQ residuals) or WHICH BPM the analyzer picked
(Round 6's invariants). `measure_grid_quality` and
`shift_grid_to_amplitude_peak`'s caller-side `bar_phase`
recomputation are unchanged. The set-the-1 BPM-preservation
invariant (Round 6 §6a) is preserved bit-identically: the new
latch path still anchors a uniform `bpm`-period grid; only the
anchor position changes.

### Round 8 — "Set the 1" is literal: the tap IS the downbeat

After Round 7 the user re-tested Blaze Up Tha Dance and reported
the regression had not actually closed:

> "Blaze Up Tha Dance doing the same error again. At first
> analysis it's almost correct, just a small bit behind the
> peak. Then I want to set the 1 a bit better and it moves
> further back in a territory where there is no peak. If we
> can't get this done properly can we make it that setting the
> 1 does simply set the 1 exactly where the user presses? Like
> with not changing the beat, it should not change the position
> the user selected? This can be true for both paused tracks as
> well as running. It's more understandable for the user I
> believe."

The user is reporting two failures of the post-Round 7 algorithm
that they can no longer tolerate:

1. **The marker moves *away* from where they clicked.** The
   Round 5 ODF snap (`snap_to_nearest_transient`, ±70 ms window
   to the strongest local ODF peak) followed by the Round 7
   amp-peak shift (0–90 ms forward walk-back-from-max) is a
   two-stage diffusive chain. When both stages agree with the
   user the marker lands cleanly. When they disagree the
   marker can land tens of milliseconds away from the click,
   *in either direction*. On Blaze the snap pulled the tap
   backward to an earlier ODF peak and the amp-peak walk-back
   continued the move backward to a leading edge in quiet
   audio. From the user's POV the marker moved "further back
   in a territory where there is no peak".
2. **The behaviour is unpredictable.** Even when the algorithm
   gets the right answer most of the time, the user cannot
   reason about WHEN it will move the tap and WHEN it will
   trust it. A 5 ms move in the click can produce a 20+ ms
   move in the marker depending on the local ODF / amp
   structure. The basic UI contract "what I click is where
   it goes" is broken.

Both failures share a single root cause: **the algorithm was
trying to be smarter than the user, and the user owns the
click coordinate.** The waveform display already gives the user
pixel-accurate control (the playhead snaps to a 64-sample chunk,
≈ 1.45 ms at 44.1 kHz, ≈ 1.33 ms at 48 kHz — well below the
~10 ms perceptual phase tolerance of human onset perception);
adding heuristics on top of that just introduced failure modes
the user can see but cannot predict or correct.

Round 8 removes every algorithmic adjustment from
`latch_beat_grid_at_downbeat`. The contract is now:

> The rendered downbeat = `downbeat_secs`, bit-exact.

| ID | Issue | Fix |
|---|---|---|
| **8a** | **The latch snaps + shifts the tap.** `latch_beat_grid_at_downbeat` ran a two-stage chain: (1) Round 5 `snap_to_nearest_transient` pulled the raw tap to the strongest ODF peak in ±70 ms (or kept the raw tap if the window was silent), then (2) Round 7 `amplitude_peak_for_beat` walked forward 0–90 ms from there to the leading edge of the near-max region. On well-behaved tracks both stages agreed with the user and the marker landed at the visible peak. On Blaze Up Tha Dance the snap pulled the tap backward to an earlier ODF transient and the amp-peak walk-back continued the move further back into quiet audio. Iterative re-tapping made the marker drift instead of converging — each tap re-ran the same diffusive chain on a slightly different region. | Both stages removed. `latch_beat_grid_at_downbeat` now uses `downbeat_secs` verbatim as the grid anchor. No snap, no amp-peak shift, no BPM refit. `measure_grid_quality` still measures residuals against the raw tap so the drift indicator surfaces a systematic phase / BPM mismatch (e.g. user tapped on a snare while the ODF backbone is on the kick, or the tapped tempo no longer fits the rest of the track). Round 6 §6a's BPM-preservation invariant is preserved — only the anchor changed. Regressions: `latch_downbeat_uses_tap_exactly` (30 ms-late tap lands 30 ms late), `latch_downbeat_uses_silent_tap_exactly` (tap in pre-roll silence stays in silence), `latch_downbeat_lands_exactly_at_user_tap_regardless_of_local_audio` (a slow-attack kick whose visible peak is +5 ms past the tap site does NOT pull the anchor toward the visible peak — the algorithm cannot second-guess the click any more). |

Scope of the change. Round 8 applies ONLY to
`latch_beat_grid_at_downbeat` (1–2 tap "set the 1" path,
called by the FFI's `set_bar_phase`). Multi-tap BPM derivation
(`analyze_beat_grid_from_taps`, 3+ taps) is unchanged — there
the user is explicitly asking the engine to derive a tempo from
their tapping rhythm, so the snap (which cleans up inter-tap
jitter inside the LSQ search) still earns its keep. Auto-
analysis (`analyze_beat_grid`) is also unchanged: it has no
user-supplied anchor and Round 7's amp-peak median is still the
right behaviour there.

Layering with previous rounds. Round 8 supersedes Round 5's
ODF snap and Round 7 §7b's per-beat amp-peak shift INSIDE
`latch_beat_grid_at_downbeat`. It does NOT change `analyze_
beat_grid` (auto path) or `analyze_beat_grid_from_taps` (3+ tap
path). The Round 6 §6a invariant (BPM preserved bit-identical
across the latch) is preserved bit-identically. Round 6 §6c
(`octave_self_verify`) still skips when `fixed_anchor` is
supplied, so the user's tapped anchor remains untouchable by
the algorithm.

Why this is the right answer, not a give-up. The user's
mental model — "what I click is where it goes" — is the
correct UI contract for a single, intentional, pixel-accurate
gesture. Heuristics ("you meant the kick, here let me find it
for you") earn their keep when the user is providing low-
fidelity input (e.g. tapping along to a playing track at
human reaction-time accuracy). For a paused deck with the
playhead scrubbed to a specific waveform position, the user's
input IS the high-fidelity ground truth; any algorithmic
"help" is necessarily worse than what the user already
provided. The auto-analysis path remains free to use every
heuristic in the book because it has no user input to honour.

### Round 9 — Integer-snap slack accounts for geometric drift

After Round 8 the user reported a related but distinct case
that the integer-snap safety net had been silently rejecting
for months:

> "Chase & Status — Come Back is getting analyzed at 174.98,
> why? Didn't we say it should jump to 175 in such a case?"

The diagnostic output for the track is the smoking gun:

```text
dub-bpm: integer-snap REJECTED bpm_raw=174.9756 -> bpm=175.00
  (rms 12.16ms -> 18.43ms exceeds slack 3.0ms), keeping bpm_raw
```

The auto path measured 174.9756 BPM. That sits 0.0244 BPM
inside the ±0.10 snap tolerance, so the integer-snap safety
net (`snap_bpm_to_integer_if_safe`) attempted the snap to
175.00. It re-fit the anchor at the snapped tempo and
measured the residuals. The result: RMS went from 12.16 ms to
18.43 ms — a 6.27 ms increase, which exceeded the historical
`INTEGER_SNAP_RMS_SLACK_MS = 3.0` slack. So the helper kept
the raw 174.98 BPM and rejected the snap.

This is correct under the old rule, and wrong for the user.

The mistake was treating the slack as a *pure* "structural
fit" budget. Snapping `bpm_raw → bpm_snapped` mathematically
shifts every predicted beat time after the anchor refit:

```text
residual_i = (i - mean(i)) * (period_raw - period_snapped)
```

For `N` observations indexed `0..N-1`, the RMS over a
mean-centred OLS anchor refit is:

```text
rms_drift = |Δperiod| * sqrt((N² - 1) / 12) * 1000  [ms]
```

This is **purely geometric**: the snap *must* introduce this
much additional RMS, regardless of whether the snapped BPM is
musically correct, simply because we changed the slope of the
predicted-times line. For the Chase & Status numbers (Δbpm
0.0244, N ≈ 530 kept beats) the geometric drift alone is
≈ 7.5 ms — already larger than the 3 ms absolute slack
before any structural mismatch has been measured.

The historical 3 ms slack was implicitly asking "is the snap
free?". For tight click-track baselines (sub-millisecond RMS)
3 ms is loose enough to absorb both geometric drift and
measurement noise on small Δbpm. For long real-music tracks
where the LSQ baseline is itself 8–15 ms RMS, the geometric
drift dominates and the slack rejects every snap that isn't
already on an integer to the third decimal. The bound is
*scale-blind*: it ignores how many observations the fit covers
and how far the snap has to move.

| ID | Issue | Fix |
|---|---|---|
| **9a** | **`INTEGER_SNAP_RMS_SLACK_MS` ignored the geometric drift the snap mathematically introduces.** A 0.02 BPM snap over 500+ beats contributes ~7 ms RMS on its own; a 3 ms absolute slack rejects this every time even though the structural fit is unchanged. On Chase & Status — Come Back the raw LSQ resolved to 174.9756 BPM (12.16 ms RMS); snap to 175 gave 18.43 ms RMS. The 6.27 ms Δ was within the geometric drift (7.5 ms predicted), meaning the snap introduced no structural mismatch — but the absolute slack rejected it. Every DnB / techno / house production where the auto resolves within ~0.05 BPM of an integer over a 4+ minute track hit the same wall. | New `expected_bpm_shift_rms_ms` helper computes the geometric RMS contribution from `(bpm_raw, bpm_snapped, n_kept)`. `snap_bpm_to_integer_if_safe` now compares the observed ΔRMS against `INTEGER_SNAP_RMS_SLACK_MS + expected_drift`: the absolute floor still catches measurement noise on small snaps, and the drift term scales the slack with the snap's irreducible cost. Chase & Status: slack 10.1 ms (3 abs + 7.1 drift), observed Δ 6.27 ms, ACCEPT 175.00. Companion `INTEGER_SNAP_MIN_KEPT_RATIO = 0.85` guard rejects the snap when `kept_fraction` drops by > 15 % (the structural-disagreement safety net the old absolute slack was *trying* to provide; this version measures it directly via the observation set rather than indirectly via RMS). The diagnostic eprintln logs the slack breakdown (abs + drift, kept ratio) so users can verify a snap decision from the CLI. Regressions: `expected_bpm_shift_rms_matches_chase_status_geometry` (the formula matches the real numbers from the bug report), `expected_bpm_shift_rms_zero_for_noop_snap` (degenerate inputs return 0), `integer_snap_accepts_near_integer_dnb_via_model_slack` (helper end-to-end on 174.98 → 175), `integer_snap_rejects_when_kept_fraction_collapses` (structural-mismatch safety net works via the kept_fraction guard, not RMS). |

Rationale for the geometric model. The drift term isn't a
heuristic; it's the closed-form RMS contribution of changing
the slope of an OLS line with re-optimised intercept. For
observations `t_i = i * period_raw + anchor_raw` and a snapped
grid `predicted_i = i * period_snapped + anchor_snapped`:

```text
best anchor_snapped = mean(t_i) - period_snapped * mean(i)
                    = anchor_raw + mean(i) * (period_raw - period_snapped)
residual_i          = t_i - predicted_i
                    = (i - mean(i)) * (period_raw - period_snapped)
RMS(residual)       = |period_raw - period_snapped|
                      * sqrt(variance(i - mean(i)))
                    = |Δperiod| * sqrt((N² - 1) / 12)
```

This is exact for click-track-style input (no per-beat
measurement noise). For real audio the inherent ODF noise
adds in quadrature: `rms_snapped² ≈ rms_raw² + drift²`. Our
slack comparison uses *linear* subtraction (`ΔRMS = rms_snapped
- rms_raw`) rather than quadratic because:

1. Real ODF noise is correlated across beats (peak-pick bias,
   spectral-flux smoothing), so the quadratic-addition
   assumption is loose anyway.
2. Linear comparison is more permissive in exactly the right
   regime: when the raw fit is already loose (10–15 ms RMS,
   typical of real music), linear slack scales naturally with
   the inherent uncertainty; quadratic would be overly
   conservative and re-introduce the original rejection bug.

Rationale for `INTEGER_SNAP_MIN_KEPT_RATIO = 0.85`. When the
snapped grid lines up with materially fewer real ODF peaks
(`kept_fraction` drops), the residual comparison is no longer
apples-to-apples — we'd be comparing a tight fit on N points
against a tight fit on N/2 of them. 0.85 means the snap may
shed up to 15 % of its kept beats, well above the ±5 % noise
floor of `kept_fraction` for small BPM perturbations on
typical commercial music and well below the level where
"the structural fit broke" is hidden. This guard catches the
genuine "snap target is at a completely different metric
level" cases that geometric drift can't see.

Bounded worst-case for a true non-integer track. The change
makes the snap aggressive within ±0.10 BPM. In the rare case
the underlying track is *genuinely* at, say, 174.92 BPM
(vintage tape master, mid-2000s drum-machine drift, repitched
audio), snapping to 175 introduces a cumulative phase error
bounded by `0.08 / 175 * duration` ≈ 0.0457 % of the track
length: ≈ 8 ms / minute, ≈ 32 ms over a 4 min mix slot. A DJ
beatmatching by ear with a pitch fader corrects this
automatically; the cleaner integer BPM is more useful for
mental tempo book-keeping than the fractional truth. The
drift indicator (now scaled by the snap distance, see
`drift_slope_ms_per_min`) surfaces the residual systematic
drift so the user can verify the snap was within bounds.

Scope of the change. Round 9 modifies `snap_bpm_to_integer_
if_safe`, the helper called from the auto path (`refine_full_
pipeline`) and the tap path (`refine_bpm_around_tap_hint`)
to absorb sub-BPM jitter at the analyzer output. It does not
change `INTEGER_BPM_SNAP_TOLERANCE = 0.10` (the entry gate),
the `octave_self_verify` invariants (Round 6 §6c), or any
user-facing FFI surface. The tap path inherits the new slack
because it calls the same helper.

### Round 10 — Integer-snap slack revert (Round 9 was wrong)

The day after Round 9 shipped the user re-tested Chase &
Status — Come Back on real audio:

> "ok come back tune was actually legit at 174.9~ now at 175
> the drift is too big towards the end. shoudl we revert?"

The user is right and Round 9 was wrong. Round 10 reverts the
core slack change while keeping the diagnostic transparency
and the `kept_fraction` guard improvements.

What Round 9 got wrong (the framing error).

Round 9 added the expected geometric drift `expected_bpm_
shift_rms_ms` to the slack budget, on the reasoning that "the
snap mathematically *has* to introduce this much RMS, so we
shouldn't penalise the snap for it." That reasoning is
exactly backwards.

The geometric drift `|Δperiod| * sqrt((N²-1)/12) * 1000` is
the **signature of the wrong tempo**, not a measurement
artefact to be excluded. If you fit a 175.0 grid to audio
that's genuinely at 174.98, the residuals show exactly this
geometric drift because the grid and the audio diverge by a
constant per-beat phase error. Round 9's slack effectively
said: "any RMS increase explainable by 'we changed the BPM
slope' is free", which silently accepted snaps on every track
that the LSQ correctly identified as non-integer. The Round 9
bound `cumulative_drift ≈ 32 ms / 4 min` for a true 174.92
track was the size of the user's complaint — they DID notice
it. 32 ms of cumulative phase error breaks beatmatching on
the second half of a track.

What Round 10 does.

Reverts `snap_bpm_to_integer_if_safe` to the strict
`Δ <= INTEGER_SNAP_RMS_SLACK_MS` (3 ms absolute) comparison
that shipped through Round 8. The 3 ms strict bound is the
right framing for almost-free reasons:

```text
drift_rms ≈ Δbpm * duration * kept_frac / sqrt(12) / bpm * 1000
```

Solving for the snap distance at which the geometric drift
alone exhausts 3 ms of slack:

```text
Δbpm_critical ≈ 3 * sqrt(12) * bpm / (1000 * duration * kept_frac)
```

For a 5-min, 175 BPM track at `kept_frac = 0.55` (Chase &
Status's regime): `Δbpm_critical ≈ 0.020 BPM`. Snaps within
~0.005 BPM of the integer pass (the inherent LSQ noise
dominates the geometric drift at that scale); snaps further
than ~0.02 BPM are rejected (geometric drift alone breaches
the slack). This implicitly caps cumulative phase drift at
~15–25 ms over typical track lengths, which lines up with DJ
beatmatching tolerance.

| ID | Issue | Fix |
|---|---|---|
| **10a** | **Round 9's geometric-drift-aware slack silently accepted snaps on genuinely non-integer tracks.** Chase & Status — Come Back (true tempo ~174.98) was snapped to 175.00 with ~43 ms cumulative phase drift over the 5-min track, audible at the end. The Round 9 framing treated geometric drift as "irreducible cost" to be budgeted into the slack; in reality it's the wrong-tempo signature. | Reverted to the strict `delta_ms <= INTEGER_SNAP_RMS_SLACK_MS = 3.0` comparison from Round 8. Chase & Status: ΔRMS 6.27 ms > slack 3.0 ms → REJECT, keep 174.98 BPM. Brown Paper Bag (174.0057 → 174): ΔRMS 0.20 ms ≤ 3.0 ms → still ACCEPT. The 3 ms strict bound implicitly scales with track length (geometric drift dominates the comparison for distant snaps on long tracks) and is the right level for DJ beatmatching tolerance. |
| **10b** | **Round 9's `INTEGER_SNAP_MIN_KEPT_RATIO = 0.85` guard is still right and is kept.** A snap that sheds > 15 % of its kept observations breaks the apples-to-apples premise of the RMS comparison — this is an independent structural-mismatch safety net that operates on the observation set rather than residual magnitude. | Preserved verbatim. |
| **10c** | **`expected_bpm_shift_rms_ms` helper is kept** but used for diagnostic logging only. Showing the expected geometric drift alongside the observed ΔRMS in the `dub-bpm: integer-snap ...` eprintln lets the user immediately see *why* a snap was rejected: "observed Δ exceeds expected, true tempo is likely non-integer." | The helper's math is unchanged (validated by `expected_bpm_shift_rms_matches_chase_status_geometry`); only the use site changed. |

Regression test. `integer_snap_rejects_genuine_non_integer_
chase_status` synthesises a click train at 174.9756 BPM over
5 minutes and asserts the snap to 175.0 is rejected. This
nails down the Round 10 contract: the strict slack must catch
this geometry regardless of any future reorganisation of
helper internals.

Why not a fancier rule (quadratic comparison, cumulative
drift cap, etc.)? A quadratic comparison (compute observed
drift `= sqrt(rms_snapped² - rms_raw²)`, compare against
expected geometric drift with absolute tolerance) is also
principled and would also reject Chase & Status. A cumulative
phase drift cap (`|Δbpm| * duration / bpm <= 20 ms`) maps the
user's complaint directly to a single number. Both work. The
strict 3 ms linear comparison is the simplest rule that
matches user perception in the cases we have evidence for,
and it's the same code path that shipped through Round 8 with
no user complaints about over-eager snapping. Round 10 picks
strict revert over new design because the user signal is
"please undo this", and there's no evidence that any track
in the user's library was being rejected under the strict
slack that *should* have snapped.

Scope of the change. Round 10 modifies
`snap_bpm_to_integer_if_safe` only:

- Reverts the RMS comparison from `delta_ms <= abs_slack +
  drift_rms_ms` to `delta_ms <= INTEGER_SNAP_RMS_SLACK_MS`.
- Updates the eprintln diagnostic to log `expected geometric
  drift` separately from the slack so the user can read the
  reason for any reject from the CLI.
- Keeps `INTEGER_SNAP_MIN_KEPT_RATIO = 0.85` and the
  `kept_fraction` guard from Round 9.
- Keeps `expected_bpm_shift_rms_ms` for diagnostic use.

Round 10 does not modify `INTEGER_BPM_SNAP_TOLERANCE`, the
`octave_self_verify` invariants, `latch_beat_grid_at_
downbeat` (Round 8 bit-exact contract), the tap path
(`refine_bpm_around_tap_hint`), or any FFI / Swift surface.

### Round 10 follow-up — Deck-header "Reset to auto" = full reanalyze

Immediately after the Round 10 slack revert the user asked:

> "does bpm context reset do the same as reanalyze in browser
> library? it should"

It didn't. Two distinct code paths reached the same conceptual
operation ("revert this track's grid to the algorithm's
baseline") with materially different results:

| Surface | Path | What it actually did |
|---|---|---|
| Library row right-click → "Re-analyze" | `analyzeTracks` → `library.analyzeTrack` (Rust) | Demoted any active `user_tap` row, ran a *fresh* analysis through the current `dub-bpm` algorithm, wrote a new `auto` row, refreshed loaded decks. |
| Deck-header BPM right-click → "Reset to auto" | `resetLoadedDeckBeatGrid` → `library.resetActiveBeatGridToAuto` (Rust) | Demoted any active `user_tap` row, **re-activated the existing `auto` row verbatim** (whatever the DB had cached, possibly from an old algorithm version). |

The two diverge precisely when the algorithm has changed
since the track was last analyzed. After the Round 10 revert
Chase & Status — Come Back had a stale 175.0 `auto` row in
the DB from when the user re-ran analysis under Round 9.
Hitting "Reset to auto" on the deck would resurrect that
stale 175.0, directly contradicting the Round 10 fix the user
just verified with Re-analyze. Same intent on the user side
("give me the algorithm's current answer for this track"),
two different outcomes depending on which menu they hit.

| ID | Issue | Fix |
|---|---|---|
| **10d** | **Deck-header "Reset to auto" revived stale `auto` rows.** When the audio-analysis algorithm changes (Round 9 → Round 10 integer-snap revert, any future analyzer tuning) the DB-cached `auto` row no longer reflects the current algorithm. User-visible result: "Re-analyze" in the library gave the correct answer, "Reset" on the deck gave the old (wrong) answer for the same track. | `resetLoadedDeckBeatGrid` now delegates to `analyzeTracks([trackId])` instead of `library.resetActiveBeatGridToAuto`. Same Rust path the library context menu uses: demotes `user_tap`, runs fresh analysis, writes a new `auto` row, installs it on the loaded deck via `publishLibraryRowAnalysisUpdate(refreshLoadedDecks: true)`. Footer "Analyzing (n)" pill appears, library row refreshes, deck BPM updates without a reload. Front-end early-out on `deck.gridLocked` is kept so the operation feels instant (Rust would also refuse with `GridLocked`). Side effects: the "no auto exists yet" error path becomes unreachable (the new analyse path creates one); a track that was never analyzed now Just Works when the user hits Reset — useful UX improvement. The Rust `reset_active_beatgrid_to_auto` function is preserved with its tests for potential future "revert without re-analyzing" entry points; it is currently dead code from the Swift side. |

Rationale for choosing "re-analyze" over "fast row-flip". A
DB cache that diverges from the live algorithm produces
violently surprising behaviour for the user when the
algorithm is iterated (i.e. constantly, in this stage of the
project). Surfacing the algorithm's current answer is the
*meaning* of Reset; the row-flip was an optimisation that
silently broke the meaning. The cost is one analyzer pass
(~1–2 seconds for a typical track on modern hardware), which
is acceptable for a deliberate user action, and the
existing footer progress pill already communicates the wait.

Scope of the change. Round 10 §10d modifies
`MainView.resetLoadedDeckBeatGrid` only (Swift). No Rust
changes, no FFI surface changes, no PRD-BEATS contract
changes elsewhere. The Rust `reset_active_beatgrid_to_auto`
function and its tests are preserved.

---

## 14. Document conventions

- Times are in seconds unless noted; intervals in milliseconds
  for ergonomic numbers.
- BPM is the canonical tempo unit; `period` is derived.
- Beats are 0-indexed in the grid array; bar positions are
  1-indexed in user-facing copy (matching DJ vernacular: "the 1",
  "the 2", "the 3 and the 4").
- "M11d.7 round 1/2/3" refers to the work logged in
  [`SHIPPED.md`](SHIPPED.md). Round 3 is the structural correction
  this spec drives.

---

**Document history**

| Version | Date | Notes |
|---|---|---|
| 0.1 | 2026-05-24 | Initial spec, post-M11d.7 round 3 design conversation. Replaces PRD §8.3.1. |
| 0.2 | 2026-05-24 | Round 4: C1 (waveform sidecar) + C2 (`bar_phase` refactor) shipped. |
| 0.3 | 2026-05-25 | Round 4 follow-up: Oppidan regression triad fixed (first-kick downbeat, backward grid extension, amplitude-peak alignment). |
| 0.4 | 2026-05-26 | Round 4 follow-up: "Set the 1" UX triad fixed (paused-deck redraw via `seekGeneration` bump, paused-deck immediate commit, playing-deck reaction-time + output-latency bias). |
| 0.5 | 2026-05-26 | "Set the 1" UX rework: withdrew the 80 ms reaction-time bias (DJs tap accurately, marker should land where the user clicks). Replaced `forceCommit` with `commitSingleTap` for paused decks so a stale playing-tap session can't leak into the paused commit. Added kick-snap inside `set_bar_phase` using the cached `FilePeaks.filtered` LF amplitude peaks so the marker latches onto the closest audible transient instead of a flat region between grid beats. |
| 0.6 | 2026-05-26 | **Root-cause fix (Brown Paper Bag regression).** Round-4 C2 rewrote `set_bar_phase` as pure `bar_phase` rotation (nearest existing grid tick goes yellow). That cannot fix a misaligned auto grid: on Brown Paper Bag the auto anchor sits at 14 ms in silence while the first kick is at 112 ms; tapping the kick with pure rotation picks the grid line at 14 ms (97 ms left of the kick — exactly the screenshot). Restored `latch_beat_grid_at_downbeat` for 1–2 tap "set the 1" (ODF snap + full grid re-anchor at the kick, BPM preserved). Fixed `synthesise_beat_grid` library reload to use `uniform_beats` + downbeat reconstruction from `(beats[0], bar_phase)` so persisted user_tap rows round-trip the same beat positions as the in-memory relatch grid. Diagnostic: `crates/dub-bpm/tests/brown_paper_bag_set_the_one.rs`. |
| 0.7 | 2026-05-26 | **Round 5 — universal downbeat fix (Baddadan).** Demoted `first_kick_peak_secs` from primary phase decider to low-confidence tiebreaker inside `find_downbeat_offset` (U1). Routed `find_downbeat_offset` through a new `score_grid_weighted` that skips intro/outro bars and weights each bar by broadband ODF energy (U2). Added a first-kick proximity tiebreaker that fires when top-2 phase scores are within 5 % AND the existing `kick_only_intro_tiebreaker` abstains (U3). Result: Baddadan's auto downbeat moves from 0.0275 s (ODF startup artifact, in the silent intro) to 1.2482 s (within 6.5 ms of the user's prior tap correction). The `dub diagnose` subcommand (lives in `crates/dub-cli/src/diagnose.rs`) is the go-to debugger for grid / waveform / BPM issues. Two pre-existing "Oppidan triad" tests re-pointed at the underlying user-facing property they were guarding; four new synthetic regression tests added. |
| 1.3 | 2026-05-26 | **Round 10 follow-up — Deck-header "Reset to auto" = full reanalyze (§10d).** Deck-header BPM right-click → "Reset to auto" used to demote the active `user_tap` row and re-activate the existing DB-cached `auto` row verbatim, which silently resurrected stale grids whenever the analyzer changed between the original analysis and the Reset (e.g. Chase & Status — Come Back's stale 175.0 BPM `auto` row written under Round 9 surviving the Round 10 revert). User reported the mismatch immediately ("does bpm context reset do the same as reanalyze in browser library? it should"). `MainView.resetLoadedDeckBeatGrid` now delegates to `analyzeTracks([trackId])`, the same Rust path the library row's "Re-analyze" right-click uses: demotes `user_tap`, runs a fresh analysis through the current algorithm, writes a new `auto` row, refreshes the loaded deck without a reload. The Rust `reset_active_beatgrid_to_auto` function is preserved (tests still pass) but dead-from-Swift; kept for a potential future "revert without re-analyzing" entry point. |
| 1.2 | 2026-05-26 | **Round 10 — Integer-snap slack revert (§10a–10c).** Round 9's geometric-drift-aware slack accepted snaps on genuinely non-integer tracks because it treated geometric drift as "irreducible cost" to budget for, when in reality it IS the wrong-tempo signature. Chase & Status — Come Back (true ~174.98 BPM) was snapped to 175.00 producing ~43 ms cumulative phase drift over the 5-min track, audible at the end. Reverted `snap_bpm_to_integer_if_safe` to the strict `delta_ms <= INTEGER_SNAP_RMS_SLACK_MS = 3.0` linear comparison from Round 8. Through the relationship `drift_rms ≈ Δbpm × duration × kept_frac / sqrt(12) / bpm × 1000`, the 3 ms strict bound implicitly caps cumulative phase drift at ~15–25 ms for typical real-music kept fractions, which matches DJ beatmatching tolerance. Brown Paper Bag (174.0057 → 174, Δ 0.20 ms) still snaps correctly. Kept Round 9's `INTEGER_SNAP_MIN_KEPT_RATIO = 0.85` `kept_fraction` guard and `expected_bpm_shift_rms_ms` helper (the helper is now used for diagnostic logging only — the `dub-bpm: integer-snap REJECTED` line shows expected geometric drift alongside observed Δ so reject reasons are visible from `dub diagnose`). Removed Round 9's `integer_snap_accepts_near_integer_dnb_via_model_slack` test; added `integer_snap_rejects_genuine_non_integer_chase_status` regression. |
| 1.1 | 2026-05-26 | **Round 9 — Integer-snap slack accounts for geometric drift.** (9a) `snap_bpm_to_integer_if_safe` was rejecting every snap on long real-music tracks within ~0.05 BPM of an integer because the absolute 3 ms RMS slack didn't budget for the geometric drift the snap mathematically introduces (`|Δperiod| * sqrt((N²-1)/12) * 1000` ms). Chase & Status — Come Back: bpm_raw 174.9756, snap to 175 introduces 7.5 ms geometric drift over 530 kept beats; observed ΔRMS 6.27 ms (less than the predicted drift, i.e. no structural disagreement); old slack 3 ms rejected. New helper `expected_bpm_shift_rms_ms(bpm_raw, bpm_snapped, n_observations)` returns the closed-form drift term; total slack is now `INTEGER_SNAP_RMS_SLACK_MS + expected_drift`, plus a new `INTEGER_SNAP_MIN_KEPT_RATIO = 0.85` guard that catches structural-mismatch cases (kept_fraction collapse) directly via the observation set rather than indirectly via RMS. Diagnostic eprintln now logs the slack breakdown (abs + drift over N kept beats, kept ratio). Bounded worst-case for a true non-integer track is ≈ 32 ms cumulative phase drift over a 4-min mix slot, well within DJ pitch-fader correction range. Scope: helper change only; entry tolerance (±0.10 BPM), `octave_self_verify`, and the FFI surface are unchanged. Regressions: `expected_bpm_shift_rms_matches_chase_status_geometry`, `expected_bpm_shift_rms_zero_for_noop_snap`, `integer_snap_accepts_near_integer_dnb_via_model_slack`, `integer_snap_rejects_when_kept_fraction_collapses`. |
| 1.0 | 2026-05-26 | **Round 8 — "Set the 1" is literal: the tap IS the downbeat.** (8a) `latch_beat_grid_at_downbeat` no longer snaps to the nearest ODF transient (Round 5) and no longer applies the per-beat amp-peak shift (Round 7 §7b). The grid anchor = `downbeat_secs` bit-exact. Restores the basic UI contract "what I click is where it goes" after the post-Round 7 Blaze Up Tha Dance regression where iterative re-tapping drifted the marker backward into quiet audio instead of converging on the visible peak. BPM preservation (Round 6 §6a) is preserved bit-identically. Scope: ONLY `latch_beat_grid_at_downbeat` (1–2 tap "set the 1" path). Multi-tap BPM derivation and auto-analysis are unchanged. Regressions: `latch_downbeat_uses_tap_exactly`, `latch_downbeat_uses_silent_tap_exactly`, `latch_downbeat_lands_exactly_at_user_tap_regardless_of_local_audio`. Updated FFI doc on `set_bar_phase` and the Swift `commitTapGrid` call-site comment to reflect the new bit-exact contract. |
| 0.9 | 2026-05-25 | **Round 7 — amplitude-peak cheat: leading edge of the loud region.** (7a) New `amplitude_peak_for_beat` helper replaces `argmax \|sample\|` with `argmax → walk-backward-from-max → leading edge of contiguous near-max region`, using a 1.5 ms backward `max` envelope. Sharp peaks land on the peak itself; sustained loud regions land on their leading edge (the user's explicit clarification of "where the transient is visually the largest, and if the transient is long at same loudness the grid would sit right at the beginning where its the loudest in the waveform"). Auto-path median (`amplitude_peak_offset_secs`) refactored to a thin wrapper around the helper. (7b) `latch_beat_grid_at_downbeat` no longer calls `shift_grid_to_amplitude_peak` — the per-beat amp-peak shift is now applied to the user's tapped beat alone, anchoring the visible downbeat exactly at the leading edge of the kick the user pointed at. Fixes Blaze Up Tha Dance's "set the 1 lands behind the peak" report. Regression suite: `amplitude_peak_for_beat_lands_on_sharp_impulse`, `amplitude_peak_for_beat_lands_on_slow_attack_body_peak`, `amplitude_peak_for_beat_lands_on_leading_edge_of_flat_loud_region`, `amplitude_peak_for_beat_returns_none_for_silence`, `latch_anchor_lands_at_visible_peak_not_at_track_median`. |
| 0.8 | 2026-05-25 | **Round 6 — set-the-1 phase contract, tap-as-hint, octave self-verification.** (6a) `latch_beat_grid_at_downbeat` no longer refits BPM — `refine_period_at_anchor` and the BPM-refitting LSQ pass are gone, BPM is preserved bit-identical to the caller's input, only the anchor moves. Fixes Apocalypse's 177.72 → 178.70 jump after set-the-1. (6b) `analyze_beat_grid_from_taps` reinstates a constrained search: tap-interval weighted median seeds a ±3 % LSQ refinement quantised to 0.1 BPM, filtered by `kept_fraction`, then integer-snapped. Window is narrower than the gap to neighbour metric levels so the search cannot drift octaves. Supersedes the M11d.7a "tap median IS the BPM" override and the original PRD-BEATS §6.1 ±15 % search. Fixes Bangin's 86.232 BPM tap-jitter case. (6c) Profile-independent `octave_self_verify` runs after `snap_bpm_to_integer_if_safe`: re-fits at `bpm/2` and `bpm*2`, swaps to the alternate when `rms_alt < 0.65 * rms_main` AND `kept_alt >= kept_main` AND `kept_alt >= 0.5`. Catches untagged tracks the profile rules can't help. (6d) New `OctaveProfile::HipHop` for Hip-Hop / Rap / Trap / R&B / Boom-Bap tags, lower-octave bias via `profile_doubletime_rejected`. (6e) New `OctaveProfile::DrumAndBass` for DnB / Jungle tags, upper-octave preference via new `profile_halftime_rejected`. Profile rules + self-verify + caller contracts layer in increasing authority; `octave_self_verify` skips when `fixed_anchor` is supplied so 6c can never overrule 6a / 6b. |
