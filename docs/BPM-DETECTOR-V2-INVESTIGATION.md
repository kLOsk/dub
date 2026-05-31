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
