# BPM detector — V2 investigation & the path forward

> Status: **§1–§7 are superseded on tempo by §8 (2026-11-19).** They
> record what was tried against the double/half-tempo and downbeat
> problems and conclude that no classical replacement beats the tuned
> rule set, on a 94-song corpus those rules were fitted to. §8 scores the
> same rules against 1,758 grids locked in Traktor (62 %), finds the cause,
> and records the replacement (96 %). Read §8 first; §1–§7 still stand for
> the experiments they describe and for the downbeat.

## 1. Problem

Two long-standing weaknesses in `dub-bpm`:

1. **Octave errors (double / half tempo).** A 174 BPM DnB track detected
   as 87; a 90 BPM rap track as 180; a 134 BPM grime track as ~90.
2. **Downbeat detection** ("which beat is the 1") is heuristic and fragile.

The `OctaveProfile` genre prior exists precisely because the octave
decision is, in the general case, perceptually ambiguous — the profile
encodes the user's genre convention to break the tie.

## 2. Baseline (measured)

The Classic detector (`tempo.rs`: log-band spectral-flux ODF →
autocorrelation → harmonic-mean windowed-energy pick → a forest of
genre-specific octave-correction rules + perceptual prior) was run over a
94-track real-music corpus (`crates/dub-bpm/tests/fixtures/*.tsv`, audio
local to the maintainer; 5 % BPM tolerance):

| Detector | Failures / 94 |
|---|---|
| **Classic** | **11** |

The 11 failures are concentrated in DnB half-time false peaks (174 → 87),
triplet false peaks (174 → 116), and a few intrinsically-ambiguous edges
(dubstep 70/140, a reggae-break-in-DnB track).

## 3. What was tried (V2, non-ML structural detector)

A parallel detector reusing the identical onset front-end (the ODF is
good), replacing only the octave decision with an explicit **beat-grid
model**: a phase-coherent comb filter that re-anchors each beat to the
local onset (drift-free, fractional-period), measuring on-beat energy,
beat coverage, beat/off-beat ratio, and strong/weak alternation. Three
iterations, each measured on the same corpus:

| Approach | Failures / 94 | Notes |
|---|---|---|
| Classic (baseline) | **11** | — |
| V2 single-pass (max beat-consistency) | 32 | over-doubles slow material (no alternation feature) |
| V2 octave walk (relative comparison) | 45 | over-halves 4/4 material |
| V2 octave walk, tamed halving | 45 | no improvement |
| Narrow ensemble (Classic + V2 second opinion) | 29 | 0 of Classic's fails fixed; 18 regressed |

**V2 did consistently fix 3 hard cases Classic gets wrong** — the
`Dangermouse – Daydream` 2× rap error and the `Evacuation` / `Coda`
triplet errors. The structural mechanism is real. But every configuration
regressed far more tracks than it fixed.

## 4. Why a non-ML detector does not beat Classic (root cause)

The decision is fundamentally a **classification with overlapping
classes**, and there is no fixed threshold that separates them. Measured
example (`Double 99 – RIP Groove`, true 128, seeded at the half-tempo 63):

```
walk: p=82 bpm=63  offbeat_ratio=0.666  alternation=0.035  coverage=0.853
```

- A **half-tempo** grid here has off-beat/on-beat ratio **0.666**.
- A **true-tempo** grid with eighth-note content has ratio **~0.5–0.6**.

These overlap. Lowering the "double it" threshold to catch this track
makes genuine-tempo tracks with subdivisions incorrectly double. The same
holds for `alternation`: a normal 4/4 backbeat (strong 1&3 / weak 2&4)
produces the *same* signal as "every other beat is an inserted one,"
which means **opposite** things — keep vs halve — depending on genre.

This is exactly why Classic carries *genre-specific* rules (skank-reject
for reggae → go down; hip-hop-double → go down; dnb-cluster → stay up):
each rule is a hand-placed decision boundary for one genre, because no
single global boundary exists. The narrow ensemble fails for the same
reason one level up — the beat-consistency score it arbitrates on is
itself octave-ambiguous (half-tempo grids frequently score *cleaner*), so
it picks the wrong octave and regresses 18 tracks while fixing none.

**Conclusion.** Classic is already a heavily-optimised instance of the
standard non-ML method (tempogram + perceptual resonance prior + per-genre
correction). Beating it with another non-ML method requires equivalent
per-genre tuning — i.e. reproducing the rule-forest — which is the
overfitting we set out to escape. The genre-rule pile, ugly as it is, is
doing real work and should stay until something categorically better
replaces it. For the residual ambiguous cases, **tap-to-grid is the
correct override** and already exists.

## 5. The path that does move the needle: a learned beat tracker

The methods that beat tuned heuristics on octave + downbeat
(`madmom`, `BeatNet`, Böck et al.) are **ML**: a small temporal model
(RNN / TCN) emits a per-frame beat / downbeat *activation*, and a
Dynamic Bayesian Network (DBN) jointly infers tempo, beat phase, **and
bar position** from it. The network *learns* the overlapping decision
boundary from labelled data instead of guessing a threshold — and
**downbeat falls out of the same model**, which directly addresses
problem #2.

### Feasibility in this codebase

- **Runtime:** pure-Rust ONNX inference via the `tract` crate — no Python
  at runtime, no C deps. Fits the no-C-build-dep posture and is offline
  (latency is a non-issue for whole-track analysis).
- **Front-end:** the existing `dub-spectral` log-band magnitude pipeline
  already produces most of what these models take as input; the feature
  adapter is modest.
- **Integration:** drops in behind the exact same seam this PoC used —
  a detector-selector parameter alongside `OctaveProfile`, Classic
  remaining the default — and is validated with the same A/B corpus
  harness pattern (`tests/corpus_ab.rs`, now removed but trivially
  re-created).
- **Downbeat:** the DBN's bar-position output replaces the heuristic
  kick-ODF downbeat picker.

### Open decisions (the reason this is a plan, not a patch)

1. **Model + weights.**
   - *Option A — pretrained:* adopt an existing model (e.g. a
     madmom/BeatNet-derived beat+downbeat net) exported to ONNX. Fastest
     to a result. **Licensing must be verified up front** — several of
     these projects' *code* is permissive (BSD/MIT) but their *trained
     models* historically ship under research/non-commercial terms, which
     would be incompatible with shipping in Dub. This is the single
     biggest gating question.
   - *Option B — train our own:* train a small TCN on an openly-licensed
     beat/downbeat dataset (e.g. GTZAN-rhythm, Ballroom, plus the
     maintainer's own labelled corpus). Clean licensing, more work, needs
     a training pipeline (out-of-repo, one-time).
2. **Dependency weight:** `tract` + an embedded model add binary size
   (model is typically a few MB) and a new dependency to vet against the
   GPL-clean stance (`tract` is permissively licensed — verify current
   terms).
3. **RT posture:** inference stays **off** the audio thread (it already
   would — analysis is off-RT), so no RealtimeContext implications.

### Recommendation

Pursue Option A *if and only if* a permissively-licensed pretrained
beat+downbeat model can be confirmed; otherwise Option B. Either way,
gate it behind the detector toggle and the corpus A/B harness, and only
promote it over Classic if it wins on the corpus — the same discipline
this investigation followed.

## 6. Disposition of the experimental code

The V2 detector (`tempo_v2.rs`), its offline entry points, and the
`corpus_ab.rs` harness were **removed** after measurement — they did not
earn promotion, and keeping a second non-winning detector in the tree
adds maintenance cost for no benefit. This document preserves the
findings. The corpus fixtures and the `real_music_corpus` gate remain;
the A/B harness is ~200 lines and trivially reconstructed when the ML
work begins.

## 7. Field survey — every other detector, and what is worth borrowing

After the V2 work (§3–§4) we surveyed how the rest of the field detects
tempo and beats, to answer one question: **did anyone solve the octave +
downbeat problem in a way we can borrow before committing to the §5 ML
tracker?** Tools examined: Mixxx (Queen Mary `qm-dsp`); Mark Hills'
`bpm-tools` (the xwax companion — same author, same audience); and the
commercial DJ ecosystem (Serato, rekordbox/Pioneer/AlphaTheta, and the
Deep Symmetry Beat Link / dysentery stack), plus a DJ-vendor detection
**patent sweep**. Each finding below was cross-checked against primary
source (the other project's code/format/patent) and against our own tree.

**Bottom line:** no external detector beats the tuned Classic detector on
dub's genres, and the categorical octave+downbeat win remains §5's learned
tracker. But the survey is not empty — it yields **two classical-DSP
experiments worth running first** (cheaper than the ML build, and one of
them reopens a door §4 closed), **one high-leverage interop win** (import
an analyzed track's curated "1"), **one downbeat lead from a patent**, and
**one firmly negative result** (a tempting idea that is just V2 again).

### 7.1 Mixxx — Queen Mary `qm-dsp`

Architecturally the most sophisticated: a complex-spectral-difference ODF
→ per-frame comb-filter ACF with a **Rayleigh tempo prior** (closed-form,
centred ~120 BPM, zero genre constants) → **Viterbi** period smoothing →
**Davies–Plumbley dynamic-programming** beat placement. Variable-tempo
capable; an "assume constant tempo" toggle irons it to one BPM downstream.

It is **not** smarter for dub's job. The single Rayleigh prior
systematically halves DnB and doubles slow tracks — Mixxx's own 2026
benchmark and bug tracker (issues #15763, #15848) document exactly the
reggae/DnB mis-octaving the Classic rule-forest exists to fix. dub is
**ahead** on the axes dub serves: it splits a kick-band ODF and
equal-weights the log bands specifically to stop hi-hats driving 2× errors
(Mixxx is still patching that "Boom-Tshak" class), and it actually
attempts a downbeat (Mixxx's shipping path picks none — its anchor is
`fmod(firstBeat, beatLength)`, no bar awareness).

Worth borrowing:

- **Local adaptive-threshold ODF detrend** *(high value, small, low
  risk).* `qm-dsp` subtracts an asymmetric sliding-window moving mean
  (`p_pre = 8`, `p_post = 7`) and half-wave rectifies, before *and* after
  the ACF. dub detrends with a **single global mean** over the whole ODF
  (`tempo.rs:650`), which cannot remove slow baseline drift — build-ups,
  filter sweeps, risers — the exact material that drags an
  autocorrelation. ~10 lines, pure DSP, strictly upstream of the lag pick
  (no octave-decision risk).
- **DFT×ACF cross-tempogram** *(high value; reopens §4's octave verdict).*
  A Fourier tempogram has peaks at integer **multiples** of the true
  period; the ACF has peaks at **sub-multiples**. Multiplying them keeps
  only peaks present in both, suppressing the half-tempo sub-harmonic peak
  that causes the 174→87 failures directly. This is the one **classical**
  attack on the octave problem we never tried — §4 concluded "non-ML can't
  beat Classic" having tested a comb-filter beat-grid and a Rayleigh-style
  prior, but **not** the cross-tempogram (Mixxx contributors prototyped
  it, PR #2877). The §5 "ML is the only categorical win" claim is
  under-supported until this is ruled out on the corpus.
- **Band-gated fraction snap** *(readout polish).* Mixxx snaps the readout
  to the simplest fraction (integer → ½ → ⅔ → ⅓) only if it stays inside a
  measured ±25 ms tolerance band. dub snaps integer-only and leaves
  87.5/116.67/174 fractional. Adopt a narrowed, readout-only version
  (after the octave is fixed; drop the `<85→×2` / `>127→×⅔` branches,
  which are soft octave biases that would fight pass-2).

Skip: the **DP beat decoder** (phase-only; touches none of the 11/94
octave fails; overlaps `lsq_refit_grid`; the §4 V2 caution applies); the
**Rayleigh-for-rules swap** (regresses dub's genres — Mixxx ships it and
mis-octaves them); **region segmentation** (`retrieveConstRegions` — its
cumulative-phase-error insight already exists as
`expected_bpm_shift_rms_ms`, and its octave gate is a no-op for 174↔87).
One prerequisite gap surfaced: dub's corpus test is **BPM-match-only** —
there is no beat-location metric (F-measure / CMLt), so any
phase/DP/offset change is currently **unmeasurable**; add one before
touching grid placement.

### 7.2 xwax `bpm-tools` — Mark Hills (lineage)

Mark Hills wrote both xwax (dub's ancestor) and `bpm-tools`, for the same
audience, which makes its choices a design signal. It is the **opposite**
of sophisticated: pure time-domain, no FFT — a PPM envelope follower over
`|sample|` (attack 1/8, release 1/512), decimated 128:1, then a
**Monte-Carlo "autodifference" comb** that picks the single global BPM
minimising on-beat similarity minus off-beat similarity, over a
deliberately narrow **84–146 BPM** default band. GPLv2-only — algorithm
reference, not vendorable.

- **Negative result (the important one): do not build the off-beat
  penalty.** `bpm-tools`' one structural trick — reward on-beat match,
  penalise off-beat match — *looks* like it could replace the genre
  rule-forest. It can't: it is **mathematically the same quantity as V2's
  `beat/off-beat ratio` feature (§3)**, which we already measured to
  regress (29–45 fails vs 11). The §4 root cause applies verbatim — a
  half-tempo grid's off-beat ratio (0.666 on *RIP Groove*) overlaps a
  true-tempo-with-eighths grid (0.5–0.6), so no global term separates the
  classes. `bpm-tools` only survives the bare objective because its
  sub-2:1 band forecloses most octave ambiguity up front, which dub's
  60–200 search deliberately does not.
- **Narrow band as octave control** *(already available).* Hills makes the
  search range the primary octave defence. dub already exposes this as
  `BpmRange` ("the recommended escape hatch"). A hardcoded narrow default
  would regress the multi-genre library (reggae ~70 and DnB ~170 cannot
  share a sub-2:1 band); the only residue is a product lever — a per-crate
  / per-source default range — gated on genre metadata often absent at
  import.

The lineage signal stands: the canonical timecode-vinyl author, facing
this exact problem for this exact audience, shipped **no** rule-forest,
**no** prior, **no** downbeat — he constrained scope (narrow band, tagging
only) instead. dub's harder remit (wide multi-genre analysis with no genre
label, plus a downbeat/grid requirement `bpm-tools` never attempts)
genuinely demands more — but it is independent confirmation that the
rule-forest is fighting an inherently hard problem, not one we over-built.

### 7.3 The commercial DJ ecosystem (Serato / rekordbox / Beat Link)

A common hope is that the established apps solved octave+downbeat in a
recoverable way. They did not — and the four targets do not even share a
layer:

- **Beat Link / Beat Link Trigger / dysentery (Deep Symmetry)** is a
  **pure consumer**: zero audio analysis, a UDP listener on PRO DJ LINK
  plus an ANLZ file parser (`crate-digger`). It reads the tempo and
  beat-within-bar a player *reports* and downloads the grid rekordbox (or,
  on a CDJ-3000, the player's own firmware) *already computed*. There is
  no detection technique anywhere in it to borrow — only a clean-room
  **protocol/format** reference (GPL Java/Clojure, not vendorable).
- **Serato** is a fully opaque detector: no vendor patent, paper, or blog
  on its algorithm. Its only public detector signal is the **"BPM Range"**
  analysis prior (58–115 … 98–195) — the *same class* of fix as dub's
  prior+rules. What is recoverable is the on-disk **"Serato BeatGrid"
  GEOB** tag (reverse-engineered by Holzhaus, mirrored in Mixxx
  `beatgrid.cpp`): **big-endian**, `N−1` non-terminal markers (position +
  beats-till-next, **no BPM** — derive it) and one terminal marker
  (position + BPM). **No explicit downbeat or beat-within-bar field** —
  the "1" is `marker[0]` by convention only.
- **rekordbox (Pioneer/AlphaTheta)** is the headline. Its modern detector
  is undocumented and, since v6, partly **deep-learning** (Qosmo) and
  cloud-cached — itself corroborating §5. The only *documented* Pioneer
  detection method is the **expired** US5614687A (multiband onset,
  peak-hold slice, consensus gate, dance-range octave fold; **onset-only,
  downbeat-blind**) — no further along than dub's ODF. But its **export
  format** carries what we want: the ANLZ **`PQTZ`** beat-grid tag stores,
  per beat, an explicit **`beat_number ∈ 1..4` where "beat 1 is the
  downbeat,"** plus per-beat tempo — a **materialised, human-curated "1."**

The **patent sweep** found the only public detection *techniques*, all
classical (the family Classic already lives in): Pioneer US8344234B2
(envelope→FFT→harmonic-weighted octave score), NI US7615702B2
(octave-folded interval histogram + rational-ratio disambiguation +
**composite 2-and-3-onset intervals**), AlphaTheta US11176915B2
(**snare-on-2&4 + bass-on-1** → 0–3 beat downbeat shift), MS US7132595B2
(Canny-onset → autocorr → phase-tree downbeat). No vendor has published an
ML detector.

| Tool | Role | Detection technique published? | Downbeat in its format | Use to dub |
|---|---|---|---|---|
| **Beat Link / dysentery** | Consumer (no audio analysis) | No — none exists; replays an offline grid | Reads stored 1–4 live (master player only) | None for the detector; interop precedent only |
| **Serato** | Detector + GEOB storage | No — no patent/paper/blog; only a "BPM Range" prior | `marker[0]` by convention; no explicit field | Format only — the M11e GEOB importer (big-endian; derive non-terminal BPM) |
| **rekordbox** | Detector + ANLZ storage | Partial — only expired, onset-only US5614687A; modern engine undocumented+ML | **Explicit** per-beat `1..4` in `PQTZ` | **High** — import `PQTZ.beat_number → bar_phase` |
| **Patent sweep** | Detector techniques | Partial — concrete *classical* methods only | AlphaTheta snare/bass → 0–3 shift; MS phase-tree | Two leads: snare-2&4 + bass-on-1 downbeat (resolved — dub follows the published prior-art method, not the AlphaTheta patent's claims), NI composite-interval |

**Interop reality check.** dub-library already has the *schema* for this —
`track_beatgrids.source` reserves `serato`/`traktor`/`rekordbox`, the v5
migration added the first-class `bar_phase` scalar, and the design fixes a
`serato > rekordbox > traktor > auto` priority — **but the binary
beatgrid importers are not yet written.** The only beatgrid rows produced
in production today come from `auto` (dub-bpm) and `user_tap`. So importing
rekordbox's PQTZ "1" is a **write-the-parser** task into a slot that
already exists, not a wiring change — pure-Rust, open Kaitai spec, zero
FFI/license exposure.

**What it validates.** Every commercial format is variable-tempo native
(rekordbox per-beat tempo; Serato chained markers) while dub emits a
constant grid — so dub's importers must merely *tolerate* per-beat drift,
not collapse it; the constant-grid choice for detection is unchallenged.
Tap-to-grid / grid-lock is vindicated as the industry-standard answer to
the residual "1": Serato ("Set" + "Grid Slip"), rekordbox, and the whole
Pioneer chain treat the downbeat as **authored human metadata, not a
detector output.** And every vendor confirms downbeat detection is hard —
all store a "1," none publishes a robust way to find it, and the good
detectors are undisclosed/ML. That is §5, restated by the whole industry:
**import the "1" where it exists, let the user tap it otherwise, and the
categorical win is the learned tracker.**

### 7.4 Consolidated — what to actually do

Ranked, merging all sources, and consistent with §5 (none of this replaces
the learned tracker; these are cheaper shots to take first, plus interop
that reduces how often detection runs at all):

1. **Prototype the DFT×ACF cross-tempogram** on the 94-track corpus
   *(classical; the one shot that could move the octave fails before ML).*
2. **Add the local sliding-mean ODF detrend** *(small, low-risk; removes
   baseline drift the global-mean detrend can't).*
3. **Add a beat-location metric (F-measure / CMLt)** to the corpus harness
   *(prerequisite — phase/downbeat changes are currently unmeasurable).*
4. **Write the rekordbox `PQTZ` → `bar_phase` importer** (then Serato
   GEOB) *(interop; seeds/overrides the fragile heuristic "1" with a
   curated one; schema already reserved).*
5. **Trial the NI composite-interval histogram** (sums of 2 and 3 onset
   intervals + rational-ratio disambiguation) against the DnB
   half-time / triplet fails *(classical lead for §2).*
6. **Implement the standard snare-2&4 + bass-on-1 downbeat rule (published
   MIR prior art — Goto/Davies-Plumbley)** as a
   principled superset of the kick-ODF picker.
7. **Readout-only band-gated fraction snap** for clean 87.5/174 readouts.
8. **Do not** rebuild the off-beat / anti-phase penalty term — it is V2
   (§3–§4), already measured to regress.

The two negatives are as valuable as the positives: the **off-beat
penalty = V2** finding closes a door that would otherwise keep looking
open, and the **commercial-ecosystem dead end** confirms there is no
proprietary detector to reverse-engineer our way past — the leverage is
classical experiments #1/#2/#5, interop #4, and ultimately §5's tracker.

### 7.5 Prototype status — built and gated this session

Four §7.4 items were implemented. Unlike the V2 detector (§6, removed),
these are **kept** behind opt-in switches with the default path byte-for-
byte unchanged; the maintainer's corpus is the gate for promoting any of
them. All ship with unit tests and pass `clippy -D warnings`.

| Item | Where | Switch | Status |
|---|---|---|---|
| **#5 snare-2&4 + bass-on-1 downbeat** | `dub-bpm/src/downbeat.rs` (`refine_downbeat_backbeat`) | opt-in API call | **Built + 8 tests.** Band-splits the audio (snare 300 Hz–2.5 kHz, kick < 240 Hz), onset-envelopes each, votes per-bar-position energy over the whole track, returns a `bar_phase`. Resolves the case the kick-ODF picker cannot (kick on 1 & 3). Caller applies the phase when its confidence clears a bar. |
| **#3 beat-location metric** | `dub-bpm/src/eval.rs` + `tests/beat_location_corpus.rs` | `DUB_BPM_BEAT_CORPUS` | **Built + 11 tests.** F-measure (±70 ms), Cemgil, CMLt/CMLc/AMLt/AMLc. This is the measurement substrate the others need — it makes a phase/downbeat change (e.g. #5) gradable, which BPM-only scoring cannot. |
| **#2 local sliding-mean detrend** | `dub-bpm/src/tempo.rs` (`detrend_odf`) | `DUB_BPM_DETREND=local` | **Built + tests; corpus A/B NEGATIVE (2026-11-14, §7.5a).** Asymmetric `p_pre=8 / p_post=7` window. Default global-mean path unchanged — and it stays the default. |
| **#1 DFT×ACF cross-tempogram** | `dub-bpm/src/tempo.rs` (`tempogram_weights`, Goertzel) | `DUB_BPM_TEMPOGRAM=1` | **Built, but smoke-test NEGATIVE.** See below. |

**#1 did not pan out — and that is a result.** A blanket Goertzel-tempogram
weight on the harmonic-mean ACF score suppresses a *pure* sub-harmonic
artifact, but *boosts* a wrong octave that is a **real** periodicity. With
the flag on, the synthetic `genre_octave` hip-hop-90 fixture flips to **180
BPM** (the real hi-hat ostinato rate), confidence 0.68, while pure click
tracks (`known_bpm`, 12/12) are unaffected. This is the §4 overlapping-
classes problem again: when the false octave carries genuine ODF energy
(hi-hat at 2×, DnB's kick-kick spacing at ½×), no parameter-free spectral
product separates it from the true tempo. So the cross-tempogram is **not**
the clean classical octave fix §7.4 hoped — it downgrades to "only a
sub-harmonic-restricted variant, validated on the real corpus, is worth
pursuing," and it further reinforces §5's conclusion that the categorical
octave+downbeat win is the learned tracker, not another DSP heuristic.

**Net after building:** the durable wins from this pass are **#3** (a real
measurement capability dub lacked) and **#5** (a principled downbeat that
covers the kick-on-1&3 case the heuristic misses). **#2** is a clean,
low-risk experiment awaiting the corpus. **#1** is empirically parked.

### 7.5a The corpus A/B, and what is actually left (2026-11-14)

**#2 local detrend is negative.** Run profile-aware over both sets, the
switch the only difference:

| Set | Shipped | `DUB_BPM_DETREND=local` |
|---|---|---|
| 94-track corpus, tempo failures (5 %) | 11 | **19** |
| 31 `user_tap` grids, tempo within 1 % | 28 | 28 |
| same, lines within 25 ms of the hand-set 1 | 21 | 22 |

It fixes none of the 11 and breaks 8 — 2 dnb (175 → 87.5, 177 → 118),
3 rap (two ×2, one 88 → 99), 3 reggae (×3/2, two ×4/3) — every one an
octave-family flip: a local mean lifts the weak in-between onsets the
global mean leaves under the floor, and the picker takes them for beats.
On the hand-set grids three tracks moved, one much better (−216 → +13 ms),
one much worse (+160 → +306 ms). Parked with #1.

**The 11 are 4 with the genre's profile.** The fixture manifests run dnb
and rap with profile `default` — no genre hint, the hard case. A track
tagged *Drum & Bass* or *Hip-Hop* in the library gets its profile, and
with those: dnb 8 → 2 (the two ~180s read 90), rap 1 → 0. The octave
problem is mostly a **missing-tag** problem: it bites untagged files and a
THRU deck, which has no tag — not a tagged library. That weakens §5's
case for a learned tracker on octave grounds.

**What the hand-set grids say.** `track_beatgrids` holds 31 `user_tap`
rows in the maintainer's library — biased toward misses, since they are
the tracks he corrected. The shipped detector against them:

| Result | Tracks |
|---|---|
| tempo, lines and the 1 all right | 13 |
| lines right, the 1 wrong | 8 |
| tempo right, lines > 25 ms off the hand-set beat | 7 |
| tempo wrong (one ×3/2, one +5 %, one test file) | 3 |

One of the 21 well-placed grids drifts over its track (170.000 against a
tapped 169.939). So the residual is **the 1** (§5's remaining case) and
**line placement** — a DSP matter: two of the seven sit within 0.05 beat
of exactly half a beat off, the off-beat lock.

**The seven misplaced grids, with the audio as referee.** Each grid —
the detector's and the hand-set one — was folded against the track's
onsets in 10 s windows, giving the shift that would put its lines on the
kicks there. A right grid reads a steady few ms all the way along:

| Track | What it is |
|---|---|
| *Cheddar*, *All The Critics In New York* | **The detector is right** (0–6 ms end to end); the taps' tempos drift ~120 ms over the track. Neither tap is the active grid. |
| *Excel – Apocalypse* | **Tempo bug, fixed below.** 173.78 against a true 175.00; the hand grid reads +12 ms for four minutes. |
| *Cherry Bomb*, *Ain't No Mountain High Enough* | **Live bands.** Neither grid fits; both wander by hundreds of ms. No even grid can — this is the following grid (`dub_bpm::BeatFollower`'s pace, offline). |
| *Bredren – Evacuation* | Right tempo, a steady 44 ms: the detector's lines on the first full-band transient, the hand-set 1 on the kick's body. |
| *Janet Jackson – If (Kaytranada remix)* | Right tempo, a steady 28 ms early: swung, soft kicks; the hand grid sits on the kick's onset to 0–3 ms. |

So the "line placement" residual is one tempo bug, two placement
offsets of 28–44 ms, and two tracks no uniform grid can serve.

**The tempo bug: the refine could not reach the tempo.** The estimator's
first guess comes from whole-frame ACF lags — at 175 BPM one frame is
3 % — and on *Apocalypse* its interpolated answer was 173.46, 0.9 %
under. `refine_full_pipeline` searched ±0.5 % of that, so 175 was never
tried and the LSQ settled on 173.78. Folded over the whole track, 175.02
scores 1.64× the best tempo in that range. The refine now looks ±2 %
first (`dominant_tempo_beyond_refine_range`) and starts from a tempo
outside the ordinary range only when it folds ≥ 1.25× everything inside
it **over the whole track and over each half** — a band holds some tempo
for a stretch; a stretch is not a tempo. Otherwise the search runs as it
always did. Measured on the 125 tracks: 9 analyses fold ≥ 1.5×, one
1.15×, the rest ≤ 1.03× (the two live bands 1.00 and 1.01); three tempos
change, all verified against the audio — *Apocalypse* 173.78 → 175.000
(line at the hand-set 1: +160 → +11 ms; F at 20 ms 0.12 → 1.00),
*Backbone* 175.4 → 176.000 (the hand-set tempo), *King Tubby – Burial*
174.21 → 175.000 — and no other track moves. ~8 ms more per analysis.
Stored `auto` grids are not versioned, so an analysed track keeps its
grid until it is analysed again.

**The two placement offsets.** *Evacuation*'s hand grid dates from
2026-05-25, before the kick-edge snap existed (2026-06-22): it sits on
the kick's body, the old behaviour, and the detector's line is on the
broadband edge at every window size — the detector is right. *If* was
real: the fit put the lines 42 ms ahead of the kicks, and
`kick_edge_offset_secs` looks ±40 ms round each line, so it found the
smaller thing before the kick (median +13.6 ms) and stopped, the kick
just out of view; the same code at ±60 ms reads +39.8. Rather than widen
the window — it must not reach a neighbouring hit — the shift now looks
up to three times, each from where the last look moved the lines. *If*:
−28.4 → +0.3 ms at the hand-set 1. Of the other 30 hand-set grids only
the two live bands move (by ~10 ms, on grids no position can fix).

**The wrong 1s were one rule, not a judgement call.** The 1 is chosen by
"the first audible beat is the 1" (`first_audible_downbeat_phase`), and
the backbeat (`refine_downbeat_backbeat`) only ran when that found
nothing — never, in practice. Against the 23 hand-set grids whose lines
are right:

| Stage | Agrees with the hand-set 1 |
|---|---|
| first audible beat (what shipped) | 15 |
| backbeat, snare 2&4 + bass on 1 | 18 |
| whole-track kick vote | 6 |

All eight misses were the first-beat rule on a track that opens on a
pickup. The backbeat was right on seven of them — six at confidence
0.16–0.30, *Lowkey* at 0.05 — and wrong on *Broken Drum*. Where the
first-beat rule was right and the backbeat disagreed, it was unsure:
0.01 (*Bangin'*) and 0.07 (*Armed & Dangerous*). The backbeat now
overrules the first beat when it disagrees at ≥ 0.10
(`DOWNBEAT_OVERRULE_MIN_CONFIDENCE`): **15 → 21 of 23**, nothing that was
right going wrong. The bar is fitted to the tracks it is scored on, and
they are tracks the maintainer corrected, so a library gains less than
that. §5's learned tracker is left with two tracks of 23 to win here.

**Tooling pitfall.** `examples/diagnose_bpm --manifest` reads the
manifest's `profile` column as notes and analyses with no profile, so it
reports 11 of 14 dubstep tracks failing where the profile-aware figure is
1. Use `tests/real_music_corpus.rs` for corpus numbers.

### 7.6 Sources

- Mixxx `qm-dsp` `TempoTrackV2` / `DetectionFunction` / `MathUtilities`:
  <https://github.com/mixxxdj/mixxx/tree/main/lib/qm-dsp>
- Mixxx `BeatUtils` (const-BPM ironing, fraction snap):
  <https://github.com/mixxxdj/mixxx/blob/main/src/track/beatutils.cpp>
- Mixxx Rhythm-analyzer DFT×ACF tempogram PR #2877:
  <https://github.com/mixxxdj/mixxx/pull/2877>
- xwax `bpm-tools` (`bpm.c`, GPLv2):
  <https://www.pogo.org.uk/~mark/bpm-tools.git>
- DJ Link / rekordbox ANLZ analysis (Deep Symmetry):
  <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html>
- `crate-digger` Kaitai spec (`PQTZ` `beat_number 1..4`, tempo BPM×100):
  <https://github.com/Deep-Symmetry/crate-digger/blob/main/src/main/kaitai/rekordbox_anlz.ksy>
- Beat Link `CdjStatus` (`getBeatWithinBar`):
  <https://deepsymmetry.org/beatlink/apidocs/org/deepsymmetry/beatlink/CdjStatus.html>
- Serato BeatGrid GEOB format (Holzhaus):
  <https://github.com/Holzhaus/serato-tags/blob/main/docs/serato_beatgrid.md>
- Patents: US5614687A, US8344234B2, US11176915B2 (AlphaTheta downbeat),
  US7615702B2 (NI), US7132595B2 (MS) — via
  <https://patents.google.com>
- Qosmo × Pioneer DJ (ML analysis in rekordbox 6):
  <https://qosmo.jp/en/news/pioneer-dj-2>

## 8. Scored against Traktor, and replaced (2026-11-19)

### 8.1 Why this was reopened

The operator added his working library — 2,654 songs, 2,169 of them with a
Traktor grid — and found the analysis "pretty poor": half, double and
"three-quarter" tempos. §2's verdict (11 of 94, or 4 of 94 with the
genre's own profile) said tempo was all but solved.

### 8.2 The reference

His Traktor grids, by his own rule (2026-11-19):

- **Lock set** — the grid is right, or good enough to mix on, for tempo
  and for where the lines sit. 1,768 songs.
- **No lock** — the song drifts (a live drummer: reggae, funk, oldies,
  rock). It was his "careful with this one" mark. 390 songs; not scored.
- **The marker is not the 1.** He never corrected which beat Traktor
  called the 1, only that the grid sat on the beat and not half a beat
  off. So Traktor is no reference for the downbeat.
- **Serato's grids are no reference at all** — he never worked in it.

Traktor's positions are on its own MP3 timeline; `mp3_frames.rs` converts
them (checked against his set-the-1 taps: +27 ms median before, +1.4 ms
after).

### 8.3 The rules, scored

`analyze_beat_grid_with_profile` as shipped, each song with the profile
its genre tag gave it, 1,758 locked songs:

| vs Traktor | Share |
|---|---|
| Same tempo (within 2 %) | 62.4 % |
| Two-thirds (128 read as 85.3) | 16.0 % |
| Half | 11.0 % |
| Four-thirds (70 read as 93) | 6.9 % |
| Other | 2.6 % |
| Double | 0.9 % |

By profile: FourOnFloor 98 %, DrumAndBass 94 %, HipHop 79 %, Dancehall
64 %, RootsReggae 57 %, **Default 46 %** (36 % of the songs — his genre
tags are crate-like labels: Bmore, Funk, NuDisco, Baile, Cheese), **Dub
10 %**.

Three causes, in `tempo.rs` / `octave_profile.rs` as they then were:

1. **`skank_doubletime_rejected` ran for every profile but FourOnFloor.**
   It rejects a 118–160 BPM candidate when a 2:1 sibling in 60–80 scores
   even 0.1 % higher. On four-on-the-floor music the half tempo scores
   about as high as the tempo by construction (the bar accent), so the
   true tempo was rejected on a coin flip; the prior then gave 64 BPM a
   weight of 0.3, and the two-thirds candidate won. The genre profile
   "helped" house (98 % with it, 30 % without) only by switching this
   rule off.
2. **`dubstep` mapped to the Dub profile**, which prefers ~70. Every
   dubstep song in his Traktor library is 120–150.
3. **The prior's 60–95 ramp** (weight 0.43 at 70 BPM) cost slow hip-hop
   and roots their tempo to the four-thirds candidate.

Also: `"ska"` matched inside "Skate Punk" and sent 21 songs to
RootsReggae.

When the tempo family *was* right, the rest was good: 96 % within
0.05 BPM of Traktor, 87 % of lines within 25 ms.

### 8.4 What was tried (scratch copy, 700 locked songs)

| Variant | Same tempo as Traktor |
|---|---|
| As shipped | 63.7 % |
| Skank rule only for the reggae profiles | 82.4 % |
| …and dubstep kept at 140 | 91.0 % (fixes 191, breaks 1) |
| …and 3:2 / 4:3 candidates in `octave_self_verify` | 91.7 % (trades 4:3 for doubles) |
| …and the prior lifted to 1.0 over 60–95 for hip-hop and roots | 91.5 % (fixes 27, breaks 23) |
| **No rules, no prior, one octave 78–156; octave ignored** | **97.8 %** (59.1 % for the shipped code scored the same way) |

The last row is the finding: the onset function and the harmonic-mean
score were never the problem. Inside one octave the best-scoring lag is
the right tempo family 98 times in 100; everything above it was the
rules undoing that. `GridQuality` (rms, kept fraction) does not separate
right from wrong tempos — rms median 20 ms against 36 ms, heavily
overlapping — so it cannot referee between candidates, which is why the
3:2 / 4:3 self-check bought nothing.

### 8.5 What shipped

`TempoOctave` (`[low, 2 × low)`), searched directly; the octave per genre
family in `OctaveProfile::tempo_octave`, each the octave holding the most
of that family's locked grids; the DJ's tempo range for everything else
(PRD-BEATS §6.0). Removed: `tempo_prior_weight`, the triplet / skank /
dancehall / hip-hop rejection passes, `linked_halftime_penalty`, every
`profile_*_rejected`, `octave_self_verify`.

All 1,767 readable locked songs, the shipped code
(`tests/traktor_grid_corpus.rs`):

| vs Traktor | Share |
|---|---|
| Same tempo | 96.3 % |
| Half | 1.0 % |
| Four-thirds | 1.0 % |
| Two-thirds | 0.9 % |
| Double | 0.4 % |
| Other | 0.4 % |

By family: DrumAndBass 98 %, FourOnFloor (with dubstep) 98 %, Default
97 %, HipHop 96 %, Dancehall 93 %, Dub and RootsReggae 89 %.

Placement, on the 1,631 songs within 0.05 BPM of Traktor: 84.7 % of
lines within 25 ms (median 8.7 ms); **105 songs (6.4 %) sit half a beat
off**. That is now the largest single defect and the one the operator
names as mattering most.

The octaves were chosen on the same songs they are scored on — five
round numbers, but in-sample all the same.

### 8.6 The 94-song corpus was partly wrong

Eight of its "expected" tempos were the old detector's own four-thirds
readings, entered as truth: *Skankin' Sweet* and *Spanish Town Rockin'*
at 104 (Traktor, locked: 78), *Duppy Conqueror* and Barrington Levy's
*Murderer* at 95 (71), *Hills And Valleys* 96 (72), *Still Blazing* 97
(73), *Welcome to Jamrock* 102 (77), Dead Prez *Hip Hop* 103 (77), Chief
Keef *I Don't Like* 88 (66). *OG Reppa* and *We Multiply*, "known fails"
at an expected 174, are 135 in Traktor — and read 135. Its dubstep rows
expected 70. The manifests now carry Traktor's values and each song's own
genre family, and read 4 known misses of 94: *Gold Dust* (two-thirds — the
one song the removed `linked_halftime_penalty` existed for), *I Don't
Like* (66, just under the hip-hop octave, reads 132), *Keep It Up* and
*Endorphin* (both unlocked in Traktor).

### 8.7 Half a beat off (same day)

105 of the 1,631 tempo-matched songs sat half a beat from Traktor's
lines. They are not one problem. Scoring 30-second stretches of each for
whether the kick band sits on Traktor's lines or Dub's: 38 have it on
Traktor's (Dub on the off-beat), 30 have it on **Dub's** (bass on the
off-beat — bassline house, dubstep — or Traktor's own grid half out),
31 show nothing either way, 6 move from one to the other mid-song.

Whole-song cues, each tried as "flip when the off-beat scores higher"
against the 105 and the 1,382 on the beat:

| Cue (off-beat ÷ on-beat) | Best rule fixes | …and breaks |
|---|---|---|
| Kick-band onsets (30–68 Hz) | 23 | 20 |
| Onsets in low, mid and high at once | 8 | 4 |
| Geometric mean of the 8 bands | 13 | 7 |
| Mid-band onsets | 48 | 19 |
| Visual kick edges found | 1 | 1 |
| Kick/snare alternation between lines | 4–19 | 11–141 |
| Peak level in the 30 ms after the line | 39 | 9 |

None separates the groups: off-beat bass is as low, as wide and as
regular as a kick. What does is **where the song starts**: the first
sound over 10 % of the track's peak falls ≥ 0.4 beat from Dub's lines on
74 of the 105 and 92 of the 1,382 — it is evidently where Traktor puts
its marker. Alone it breaks more than it fixes; with "and the off-beat
is the louder" it fixes 53 and breaks 5, and fitted on half the songs
and tried on the other it held (26 of 52 fixed, 5 of 691 broken).

Shipped as `move_off_the_off_beat`. Measured on the real code: **105 →
58 half a beat off**, lines within 25 ms 84.7 % → **87.0 %**, 90th
percentile 39 → 29.5 ms. About 30 of the 58 left are the songs where
Traktor's own lines sit off the low end; nothing here can call those.

### 8.8 Tap tempo: the taps are a hint (same day)

The operator: "the dj always taps wrong, he's never as accurate as the
transients… with the tap dub should understand: this is roughly the
correct bpm and the correct transients for the beats." His logged
session on a 129 song: seven taps at 118–121, committed 121 — the fit
only looked ±3 % around the tapped tempo.

Now (`analyze_beat_grid_from_taps`, PRD-BEATS §4.2): the song's own
tempo is looked for within ±12 % of the tapped one by folding the
whole-song onset function at every tempo in reach, and the lines are
placed by the whole song in the phase the taps point at.

Each locked song "tapped" 8 % out (under and over alternately), one tap
35 ms late:

| | Share of 1,767 |
|---|---|
| Best-folding tempo in reach is Traktor's (within 0.4 %) | 99.3 % |
| Grid comes back within 0.05 BPM of Traktor | **94.3 %** |
| …and with its lines within 25 ms | **85.9 %** |
| (plain analysis, no tap: within 0.05 BPM / lines too) | 92.3 % / 80.3 % |

Two settings came from this run. The dominance a tempo needs before it
overrules the tap was 1.3 by guess and sent 65 right answers to the
fallback; right picks stand a median 2.3× clear (5th percentile 1.38),
the 13 wrong ones 1.06 — it is 1.1. And the tap path rounded to whole
BPMs on trust (`IntegerSnapPolicy::TAP`, built for a tempo that was the
DJ's): with it only 81.6 % came back within 0.05 BPM. A tempo that is
the song's is rounded as an analysed one.

The tempo is read from the taps' **playhead positions**. Since Round 11
it had come from the wall-clock tap rate, justified by a test in which
the DJ taps the track's native tempo while hearing it 8 % slow. He taps
what he hears; in the track's own time that is one beat per tap at any
pitch.

### 8.9 The drift warning (same day)

The operator, on *Hey Ya*: "it says may drift over a long mix but i find
the grid perfect". The stored reading was 5.6 ms/min; the mark fired at 3.

The grid does drift: against the waveform its hits sit 4 ms before the
lines in the first sixth of the song and 9 ms after them in the last,
about 4 ms a minute. The tempo, 79.49, is Serato's, rounded to two
decimals; the song is nearer 79.486. Nobody hears 15 ms over four
minutes.

Scored on the 1,767 locked grids, each measured as set-the-1 measures a
grid (`fit_grid_at_playing_tap` at Traktor's tempo), and again with the
tempo made 6, 12 and 20 ms/min slow:

| Reading | Limit | Locked grids marked | 6 ms/min caught | 12 caught | 20 caught |
|---|---|---|---|---|---|
| least squares (as shipped) | 3 | 27.8 % | — | — | — |
| least squares | 10 | 6.7 % | 11.5 % | 58.3 % | 83.8 % |
| median over pairs ≥ 20 s apart | 8 | 6.7 % | 11.1 % | 92.5 % | 96.7 % |
| median over pairs | 10 | 5.4 % | 7.0 % | 86.9 % | 95.2 % |
| least squares, strays over 40 ms dropped | 8 | 4.1 % | 8.2 % | 86.9 % | 91.5 % |
| medians of eight stretches | 8 | 10.3 % | 16.1 % | 89.8 % | 94.0 % |

The least-squares slope under-reads a real drift (a 12 ms/min drift reads
a median 10.4, and under 4 on one song in ten) and invents one where the
arrangement changes. The median over pairs reads 11.9, and 10.6 or more
on nine in ten.

The locked grids that still read 8 or more are mostly real: for two in
three, a second reading that shares nothing with the first (where the
waveform's level jumps, folded per sixth of the song) agrees in sign and
size. He mixed on them; a played drummer wanders.

Dub's own grids were also read. With least squares, every one of the 237
marked at 3 had had its tempo rounded to a whole number; the fit that
sets a tempo minimises this reading, so the mark says little about Dub's
own grids either way — of 162 whose tempo sat 10 ms/min or more from
Traktor's, 18 were marked.

Shipped: the median reading, `GridQuality::DRIFT_WARNING_MS_PER_MIN = 8`,
one rule for the deck header and the library column. The benchmark, on
the code as shipped, has 7.0 % of the locked grids marked and 92.3 % of
the 12 ms/min drift caught, and holds both. It also has Dub marking
4.7 % of its own grids — more than least squares did (2.6 % at 8),
since the median is not what the tempo fit minimises. Whether those 83
are the songs where Dub's tempo is off Traktor's was not checked.

### 8.10 Not done

- **Dub's own tempo against Traktor's.** Where both read the same tempo,
  the two grids part by 9.5 ms/min or more on one song in ten (p95 19).
  Which of the two is nearer the song was not measured; the drift mark
  does not show it (§8.9).

- **Half-a-beat-off grids, the 58 left (3.6 %).** §8.7. A tap on the
  beat of a playing deck fixes one by hand (PRD-BEATS §4.1).
- **The 1.** Unscored: Traktor is no reference for it. §5's learned
  tracker remains the only categorical answer on record.
- **Two-thirds and four-thirds (1.9 %)** still happen where sixteenth-note
  content makes those lags as periodic as the beat.
- **Thru decks** read in the default octave; the DJ's tempo range does not
  reach them yet. The dnb baseline record still reads 116.5, two-thirds.
- **None of the tap work has been tried on a deck.** The numbers above
  are simulated taps on files.

