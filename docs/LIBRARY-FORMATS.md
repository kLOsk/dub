# Library import formats

> Field notes for parsing external DJ library formats. Filled in as the
> import adapters in `crates/dub-library/src/` land.

## Status

This document is a format-notes stub for external DJ libraries. Dub-native
filesystem import has shipped; Serato / Traktor / rekordbox / iTunes importers
are still forward-looking. Fill each section as its importer lands, and keep
format-specific quirks here rather than expanding PRD §8.

Each section should end up with:

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
  Source files open with `O_RDONLY` semantics; no advisory locks; concurrent
  use of Serato / Traktor / rekordbox is fine.
- **Canonical track identity** lives in `tracks` as a stable UUID; the
  fingerprint match is `chromaprint_similarity ≥ 0.98 AND duration_delta <
  200 ms AND no version-token differs`. See PRD §8.1 for the version-token
  list (`clean / dirty / instrumental / acapella / radio / edit / extended
  / club / dub / vip / remix / remaster / mono / stereo / intro / outro
  / short / long / 7" / 12" / lp`).
- **Per-source metadata is preserved verbatim,** not destructively merged.
  Each source's row in `track_metadata_source` carries that source's
  opinion (artist / title / album / comment / bpm / key / gain /
  version_token); the browser's displayed value is chosen by the priority
  chain `serato > rekordbox > traktor > id3 > filename` but every source's
  raw value remains queryable. Re-import refreshes the per-source row only.
- **Beatgrid priority:** default order `Serato > rekordbox > Traktor >
  auto-detect`, user-configurable in Preferences (PRD §8.3). Per-track
  override available from the browser context menu. Every imported grid
  is cross-validated against `dub-bpm`; disagreement flags a ⚠ on the
  row.
- **Imported hot cues and loops** are written to `track_cues` /
  `track_loops` from v1.0 day one even though the v1 UI does not surface
  them (PRD §6.6, §8.6). This is what makes the rekordbox-XML export at
  M11f a lossless round-trip (Serato in → rekordbox XML out, cues
  preserved).
- **Path-by-volume-UUID.** `track_files` stores `(volume_uuid,
  relative_path_from_volume_root)`; resolution falls back through
  last-known mount → basename + fingerprint search → prompt the user.
  PRD §8.2.
- **Failure modes:** corrupted file → graceful error, never crash. PRD
  §2.2.5 fuzz targets must cover every parser in this doc.

## See also

- `docs/PRD.md` §8 — Library (philosophy, imports, schema, beatgrids,
  analysis, browser UX, export, schema-as-public-API)
- `docs/LIBRARY-SCHEMA.md` — public SQLite schema reference
- `crates/dub-library/src/` — implementations
