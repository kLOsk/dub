# BPM detector — V2 investigation & the path forward

> Status: **investigation complete, experimental code removed.** This
> document is the durable artifact. It records what was tried against the
> double/half-tempo and downbeat problems, the measured results, why a
> classical (non-ML) replacement does not beat the tuned Classic detector,
> and the concrete plan for the approach that can.

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
| **#2 local sliding-mean detrend** | `dub-bpm/src/tempo.rs` (`detrend_odf`) | `DUB_BPM_DETREND=local` | **Built + tests.** Asymmetric `p_pre=8 / p_post=7` window. Default global-mean path unchanged. Awaiting corpus A/B. |
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
