# Timecode fixtures — real vinyl

Ten-second excerpts of `dub capture` recordings from the SL 3, used by
`crates/dub-engine/src/real_vinyl_tests.rs`. **These are tracked** (~5 MB), so
the tests run in CI; the 60 s captures they were cut from live in `full/` and
are gitignored.

## Why real captures and not a generated carrier

Every other timecode test in the workspace feeds the decoder a
`dub_timecode::signal::Generator` carrier — a clean sine pair, with white noise
if asked. Real vinyl has surface noise with **lag-1 autocorrelation ≈ 0.8**,
dust ticks, wow and flutter, channel imbalance, and a hand on the platter.

That gap has already cost a rig session. `LESSONS.md` records it: the lag-1
phase-difference estimator shrinks under correlated noise, and coherence of
0.999 still read **−0.31 % pitch at a true zero** on a real deck. Synthetic
noise is white by construction, so no generated test could produce that bias.
`steady_nominal_has_no_pitch_bias` would have caught it in CI.

## The fixtures

| File | From | What it is |
|---|---|---|
| `serato-cv02-steady-nominal.flac` | deck B, 45–55 s | Platter at nominal, untouched. Decodes 100 % locked, rate 0.9918–1.0101, 94 % absolute. The pitch-bias reference. |
| `serato-cv02-scratch.flac` | deck A, 32–42 s | Real back-and-forth scratching, rate −1.32 to +1.30, 92 % locked, 0 position jumps. Absolute lock drops to 11 % — scratching kills the LFSR, which is itself worth knowing. |
| `serato-cv02-needle-drop.flac` | deck A, 0–8 s | The stylus landing: silence, then amplitude climbing, then lock. Guards "never auto-play with the needle up". |

Measured on the steady fixture: **mean rate 0.99966, i.e. −0.034 % bias.** The
test's threshold is 0.15 %, verified by injecting ±0.3 % with
`ffmpeg -af asetrate` — that reads +0.264 % / −0.334 % and fails, so the test
is sensitive at the scale the historical bug lived at.

## Format

24-bit FLAC, converted from the original 32-bit float captures and **verified
bit-exact** by decoding back and comparing sample for sample. The captures sit
exactly on the 24-bit grid because `dub capture` applies no gain.

## Working with them

    ./target/release/dub decode-timecode testdata/timecode/serato-cv02-scratch.flac --head 3000

`decode-timecode` reads anything symphonia decodes, so the FLAC works directly.
Its summary block prints the lock / absolute / rate-range numbers quoted above.

## What is still missing

Worth capturing on the next rig session, in rough order of value:

1. **A stalled platter, needle down** — the amplitude gate's own case. Nothing
   here holds the record still.
2. **A needle lift mid-play**, then a re-drop. The lift policy took three SL 3
   iterations (`LESSONS.md`) and no fixture exercises it.
3. **Pitch extremes** — the same steady take at ±8 %. Carrier-frequency scaling
   is a documented silent-failure landmine.
4. **Traktor MK1 and MK2** — every fixture here is Serato CV02, so two of the
   three shipped decoders have no real-vinyl coverage at all.
5. **A worn or dusty record** — tick immunity and the sticky-block window.

Cut new excerpts from `full/` with
`ffmpeg -ss <start> -t <len> -i full/<capture> -c:a flac -sample_fmt s32 -bits_per_raw_sample 24 <out>`,
then read the decode summary before committing to the numbers.
