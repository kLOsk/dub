# Dub — Beat-Grid, Tempo and Waveform Spec

> **Sub-spec of [PRD.md](PRD.md).** Source of truth for everything BPM,
> beat grid, downbeat, tap-to-grid, and the visual contract between
> the grid and the waveform. Implementation lives in
> `crates/dub-bpm/`, `crates/dub-ffi/`, and `apple/Dub/Performance/`.

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

### Shipped: rounds 4–10 (corpus-driven hardening)

Round 4 closed the two structural items round 3 deferred — the **waveform
sidecar** (`analysis_cache.waveform_sidecar_path` writer; instant waveform on
load, spec gate 13) and the **`bar_phase` refactor** (first-class `BeatGrid`
field + schema v5 `track_beatgrids.bar_phase`; `relatch_beat_grid_at_downbeat`
became the pure-rotation `set_bar_phase`; both renderers test `idx mod
beats_per_bar == bar_phase`).

Rounds 5–10 were a sequence of real-music regression fixes (Oppidan, Roni Size,
Baddadan and the 94-track corpus): **universal downbeat scoring**
(`score_grid_weighted` over the whole track, not just beat 0), the **set-the-1
phase contract** ("the tap *is* the downbeat", bit-exact, no snap), **tap-as-hint
constrained search**, **octave self-verification**, the genre **OctaveProfile**s
(HipHop / DrumAndBass / FourOnFloor), the **amplitude-peak leading-edge** anchor,
and the **integer-snap safety net** (round 9 added geometric-drift slack, round 10
reverted it as wrong and kept the strict 3 ms model). Round 10 also made the
deck-header "Reset to auto" a full re-analyze rather than a stale-cache rotation.

The durable lessons (the octave ceiling is structural; the library grid is the
single source of truth; honesty contract) are in
[`LESSONS.md`](../history/LESSONS.md#bpm--beat-grid). The per-round narrative and the
specific regression tracks are in git history — `git log crates/dub-bpm`.

## 14. Document conventions

- Times are in seconds unless noted; intervals in milliseconds
  for ergonomic numbers.
- BPM is the canonical tempo unit; `period` is derived.
- Beats are 0-indexed in the grid array; bar positions are
  1-indexed in user-facing copy (matching DJ vernacular: "the 1",
  "the 2", "the 3 and the 4").
- "M11d.7 round 1/2/3" refers to the work logged in
  [`SHIPPED.md`](../history/SHIPPED.md). Round 3 is the structural correction
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
