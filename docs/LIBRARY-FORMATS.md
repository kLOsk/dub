# Library import formats

> Field notes for parsing external DJ library formats. Filled in as the
> import adapters in `crates/dub-library/src/` land.

## Status

This document is a stub. It will be filled in M11–M12 (PRD §12) as we
implement the importers. Each section below should end up with:

- Where the format lives on disk
- Schema overview (with examples)
- How beatgrids and cue points are encoded
- Known gotchas / wire-format quirks
- Fuzz target reference (PRD §2.2.5)

## Sources to import

### Serato

- **Database:** `_Serato_/database V2`
- **Crate files:** `_Serato_/Subcrates/*.crate`
- **Per-track metadata:** ID3 GEOB tags inside the audio file itself
  - `Serato Markers_` (cues, hot cues)
  - `Serato Markers2` (newer format)
  - `Serato BeatGrid`
  - `Serato Overview` (waveform overview)
  - `Serato Autotags` (BPM, gain)

### Traktor

- **Collection:** `~/Documents/Native Instruments/Traktor X.X.X/collection.nml`
- XML format. Big monolithic file.
- Beatgrids encoded as `<TEMPO>` + `<CUE_V2>` (downbeat anchor at type=4).

### rekordbox

- **Database:** `~/Library/Pioneer/rekordbox/master.db` (encrypted SQLite, DB6)
  - Decryption key is community-known; clean-room implementation required.
- **XML export** (alternative path): user-exported XML file.
- DB6 schema: many tables; key ones are `djmdContent`, `djmdCue`, `djmdBeatGrid`.

### iTunes / Apple Music

- **Library:** `~/Music/iTunes/iTunes Library.xml` (legacy) or
  `~/Music/Music/Library.xml` (Apple Music export).
- Plain XML. BPM often missing. Beatgrids never present.
- Useful primarily for crate / playlist structure.

### Lexicon DJ

- **Lexicon owns its own database** but its primary mode is *exporting* to
  Serato / Traktor / rekordbox / iTunes.
- We don't read Lexicon's native format directly; we read whichever target
  format Lexicon has exported to. Lexicon → rekordbox.xml → us.

## Common concerns

- **Dub never modifies source library files or databases.** Read-only always.
- **Dedupe by canonical identity:** `(audio_fingerprint_hash, file_size, duration_ms)`.
  Same track imported from multiple sources merges into one Dub entry, with
  per-source metadata preserved.
- **Beatgrid priority:** if multiple sources have a grid for the same canonical
  track, prefer Serato (most accurate for the urban music scene), then
  rekordbox, then Traktor, then auto-detect.
- **Failure modes:** corrupted file → graceful error, never crash. PRD §2.2.5
  fuzz targets must cover every parser in this doc.

## See also

- `docs/PRD.md` §8 — Library
- `crates/dub-library/src/` — implementations
