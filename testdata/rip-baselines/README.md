# Rip tuning baselines

Three real record sides captured on the SL 3, and the ground truth behind
every constant in `crates/dub-rip/src/gaps.rs`. The audio itself is
gitignored (748 MB); this file is the index.

| File | Record | Speed | Captured | Tracks | Why it earns its place |
|---|---|---|---|---|---|
| `rip-baseline-a.flac` | reggae 12" | 33 | 2026-08-22 | 4 | The first side. Generous case — wide gaps, music at −18 dBFS, 22 dB of contrast. Fitting the gates to this alone is what went wrong. |
| `rip-baseline-b.flac` | soul sampler | 33 | 2026-08-28 | 10 | Quiet-mastered (music −25.3 dBFS) with gaps at −38 to −44, which is what retired the `music − 18 dB` cap. Also carries a mid-side needle re-drop. |
| `rip-baseline-c.flac` | drum'n'bass | 45 | 2026-08-28 | 1 | One track a side, long fade-in, and a lead-out tick that used to split a phantom second track out of the run-out. |

## Using them

    ./target/release/dub rip-tune testdata/rip-baselines/rip-baseline-b.flac

Useful flags: `--head N` (per-second cell peaks at the head, the statistic the
lead-in gate reads), `--profile N` (level shape over time), `--quietest N` (the
most gap-like stretches and the line each would need), `--margin-db` /
`--lead-in-margin-db` / `--min-contrast-db` / `--silence-drop-db` to sweep.

Expected under the shipped defaults — if a change moves any of these, it moved
a constant that three records agreed on:

| | side start | side end | tracks |
|---|---|---|---|
| a | 0:07.0 | 10:48.1 | 4 |
| b | 0:09.6 | 35:26.7 | 10 |
| c | 0:08.4 | 5:47.5 | 1 |

## Format

24-bit FLAC, converted from the original 32-bit float `dub capture` WAVs and
**verified bit-exact** by decoding back and comparing sample-for-sample — the
captures sit exactly on the 24-bit grid (`dub capture` applies no gain), so
nothing was lost. 1.5 GB → 748 MB.

The synthetic fixtures in `gaps.rs` encode these records' *measured levels*, so
the test suite does not need these files. They are here for the next retune, or
the next argument about why a constant is what it is.
