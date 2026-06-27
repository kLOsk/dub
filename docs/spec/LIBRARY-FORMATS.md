# Library import formats

> Field notes for parsing external DJ library formats. Filled in as the
> import adapters in `crates/dub-library/src/` land.

## Status

Format notes for external DJ libraries. Shipped: Dub-native filesystem import
(M11c), the **Serato importer (M11e)**, the **Traktor NML importer (M12b)**, the
**iTunes / Apple Music importer (M12c)**, and the **rekordbox XML importer
(M12d)**. Lexicon is still forward-looking. Fill each section as its importer
lands, and keep format-specific quirks here rather than expanding PRD §8.

Each section should end up with:

- Where the format lives on disk
- Schema overview (with examples)
- How beatgrids and cue points are encoded
- Known gotchas / wire-format quirks
- Fuzz target reference (PRD §2.2.5)

## Sources to import

### Serato — **shipped (M11e)**

- **Folder:** `~/Music/_Serato_` (boot volume); each external drive has its own
  `/Volumes/<drive>/_Serato_`.
- **Database:** `_Serato_/database V2` — metadata + the master track list.
- **Crate files:** `_Serato_/Subcrates/*.crate` — the crate tree.
- Parser: `crates/dub-library/src/serato/` (pure); adapter:
  `crates/dub-library/src/serato_import.rs`.

**Container format** (both `database V2` and `.crate`): a flat sequence of tags,
each `[4-byte ASCII type][4-byte big-endian u32 length][payload]`. Text payloads
are **UTF-16 big-endian**; paths are **relative to the volume root** (no leading
slash). `database V2` = `vrsn` then one `otrk` per track; each `otrk` payload is
itself a tag sequence:

| tag | field | tag | field |
|-----|-------|-----|-------|
| `pfil` | file path (rel. to volume) | `tbpm` | BPM (text) |
| `ttyp` | file type | `tkey` | key (musical, e.g. `Em`) |
| `tsng` | title | `tcmt` | comment |
| `tart` | artist | `tcom` | composer |
| `talb` | album | `tgrp` | grouping |
| `tgen` | genre | `tlbl` | label |

*(Validated against a real export: paths reconstruct as `mount_point + pfil`;
keys come back `Em` / `Bm` / `Ebm` — converted to Camelot for display, raw kept
in `track_keys.original_notation`.)*

**Crate nesting** is encoded in the `.crate` *filename* with `%%`
(`Hip Hop%%90s.crate` → folder "Hip Hop" › crate "90s"); each `.crate` is
`vrsn` + sort/column tags + one `otrk` per member carrying a `ptrk` path.

**Beat grid / cues / loops / gain** are **not** in `database V2` — they live in
ID3v2 `GEOB` frames inside each audio file (read via the `id3` crate; MP3 / AIFF
/ WAV — MP4 / FLAC deferred):

- `Serato BeatGrid` — raw binary. `01 00` + u32 total-marker-count; the first
  count-1 markers are non-terminal (`f32 position_secs` + `u32 beats_to_next`),
  the **last** is terminal (`f32 position_secs` + `f32 bpm`). Dub takes the first
  marker's position as the downbeat anchor + a single BPM. *(Validated: a
  constant-tempo track is `count == 1`, 15 bytes, e.g. anchor `0.401`s / `91.0`
  BPM — matching `tbpm`.)*
- `Serato Markers2` — `01 01` + base64 (sometimes padded/newline-wrapped → use a
  padding-indifferent decoder) of NUL-terminated-name entries (`name\0` +
  `u32 len` + body). `CUE` body = `00 index(1) position_be_u32_ms 00 color(3)
  00 00 name\0` → hot cue (ms). `LOOP` body = `00 index start_ms end_ms …`.
  `COLOR` / `BPMLOCK` skipped.
- `Serato Autotags` — short header + ASCII BPM / auto-gain / gain (NUL-separated;
  parsed tolerantly). Gain → `track_metadata_source.gain_db`.

### Traktor — **shipped (M12b)**

- **Collection:** `~/Documents/Native Instruments/Traktor X.X.X/collection.nml`
- One monolithic XML file. Parsed by `crates/dub-library/src/traktor.rs`
  (pure, streaming, `quick-xml`); imported by
  `crates/dub-library/src/traktor_import.rs`.

**Document shape**

```xml
<NML VERSION="19">
  <COLLECTION ENTRIES="N">
    <ENTRY TITLE="…" ARTIST="…">
      <LOCATION DIR="/:Users/:dj/:Music/:" FILE="track.mp3" VOLUME="Macintosh HD"/>
      <ALBUM TITLE="…"/>
      <INFO GENRE="…" COMMENT="…"/>
      <TEMPO BPM="174.500000"/>
      <MUSICAL_KEY VALUE="21"/>
      <CUE_V2 NAME="AutoGrid" TYPE="4" START="0.0"     LEN="0"      HOTCUE="-1"/>
      <CUE_V2 NAME="Drop"     TYPE="0" START="16000.0" LEN="0"      HOTCUE="1"/>
      <CUE_V2 NAME="Roll"     TYPE="5" START="32000.0" LEN="4000.0" HOTCUE="0"/>
    </ENTRY>
  </COLLECTION>
  <PLAYLISTS>…</PLAYLISTS>
</NML>
```

**Paths.** `<LOCATION>` splits the path across attributes: `VOLUME` is the
volume *name*, `DIR` uses `/:` as its separator (and has a leading + trailing
`/:`), `FILE` is the basename. Reconstruction: `DIR.replace("/:", "/")`, then
map the volume — `"Macintosh HD"` (the boot volume) → filesystem root,
anything else → `/Volumes/<name>`. *(Validated against a real export: e.g.
`VOLUME="Macintosh HD" DIR="/:Users/:klos/:Downloads/:…/:" FILE="x.wav"` →
`/Users/klos/Downloads/…/x.wav`.)*

**Beatgrid.** `<TEMPO BPM>` + the first `<CUE_V2 TYPE="4">` (AutoGrid) as the
downbeat anchor. Imported as `track_beatgrids(source='traktor')` with
`bar_phase = 0` (a Traktor grid anchor *is* beat 1).

**Cues / loops.** `<CUE_V2>` `START` and `LEN` are in **milliseconds**
(validated — a non-zero AutoGrid at `2150.96` ms decodes to `2.15 s`).
`TYPE`: `4` = grid anchor (→ beatgrid, not a cue), `5` = loop (→ `track_loops`,
`out = START + LEN`), anything else = cue (→ `track_cues`). `HOTCUE ≥ 0` is the
pad slot (→ `cue_index`, `kind='hot_cue'`); `HOTCUE = -1` is an unslotted
memory marker (`kind='memory'`, indexed above the pad range so it can't alias
a real pad). `NAME="n.n."` is Traktor's "no name" sentinel → dropped.

**Key.** `<MUSICAL_KEY VALUE>` is `0–23` chromatic (`0–11` = major C…B,
`12–23` = minor C…B), mapped to Camelot (e.g. `21` = A minor = `8A`). The
raw `VALUE` is preserved in `track_keys.original_notation`. *(The 0–23
ordering is the assumption; the file carries no text key to cross-check it
against, so treat the exact per-value mapping as best-effort until a tagged
export confirms it.)*

**Playlists.** `<PLAYLISTS>` is a `<NODE>` tree: `TYPE="FOLDER"` (nesting),
`TYPE="PLAYLIST"` (members via `<ENTRY><PRIMARYKEY KEY="…"/>`), `TYPE="SMARTLIST"`
(dynamic — skipped, we can't resolve it to fixed tracks). The `$ROOT` folder is
transparent. A `PRIMARYKEY` `KEY` is a single `VOLUME/:dir/:file` string that
reconstructs to the *same* path as the matching `<LOCATION>` (the join key).
The tree is mirrored into the read-only `imported_crates` /
`imported_crate_tracks` tables (truncate-and-rewrite per source); folders
become parentless/parented crate rows, playlists carry the members.

**Import behaviour.** Idempotent by `(volume_uuid, relative_path)` — the *same*
identity the folder importer uses, so Traktor metadata/grid/key/cues enrich a
track the DJ already imported rather than duplicating it. Lazy (PRD §8.4): no
decode, no fingerprint at import; a track first seen via NML lands with
`fingerprint_id = NULL`. A reference whose file isn't on this machine is
reported in `ImportSummary::skipped` (with a reason), never inserted. Run it
headless with `dub import --traktor <collection.nml>`.

**Fuzz target.** `fuzz/fuzz_targets/fuzz_traktor_nml.rs` over `parse_nml`
(PRD §2.2.5). The parser's contract is *never panic / hang / OOB on any bytes*
— only `Ok(ParsedCollection)` or `Err(ParseError)`.

### rekordbox — **shipped (M12d)**

- **Library:** the XML export (`File → Export Collection in xml format`), e.g.
  `~/Documents/rekordbox/rekordbox.xml`. We deliberately read the **XML export**,
  not the encrypted `~/Library/Pioneer/rekordbox/master.db` (SQLCipher, DB6):
  the XML is the documented, GPL-clean interchange format and reuses `quick-xml`,
  where `master.db` would need a reverse-engineered key + C crypto + an
  undocumented, version-fragile schema. (`master.db` decode is parked; if it's
  ever wanted it stays confined behind an opt-in feature.)
- Parser: `crates/dub-library/src/rekordbox.rs` (streaming `quick-xml`); adapter:
  `crates/dub-library/src/rekordbox_import.rs`.

**Shape.** `<DJ_PLAYLISTS>` → a `<COLLECTION>` of `<TRACK>` elements + a
`<PLAYLISTS>` `<NODE>` tree. A `<TRACK>` carries all metadata as attributes
(`Name` / `Artist` / `Album` / `Composer` / `Genre` / `Comments` / `Year` /
`TrackNumber` / `AverageBpm` / `Tonality` / `TotalTime` / `Location`); `Year` /
`TrackNumber` `0` = unset. **`TotalTime` is integer seconds** (coarser than
iTunes' ms). A track with a grid/cues is a container with `<TEMPO>` /
`<POSITION_MARK>` children; otherwise it self-closes.

**Paths.** `Location` is a percent-encoded `file://localhost/…` URL → strip the
scheme + `localhost`, percent-decode → absolute path (same shape as iTunes).

**Grid.** The first `<TEMPO Inizio Bpm Battito>` is the grid anchor (`Inizio`
seconds, `Bpm`, `Battito` 1–4 → 0-based bar phase). Extra `<TEMPO>`s are
per-beat / variable-tempo markers we don't model. `AverageBpm` is the metadata
BPM (falls back to the grid tempo when `0`).

**Cues / loops.** `<POSITION_MARK Type Start End Num Red Green Blue>`: `Type` 0 =
cue, 4 = loop (1/2 fade, 3 load → skipped). `Num` −1 = memory cue, 0–7 = hot-cue
pad slot. `Start`/`End` seconds; `Red`/`Green`/`Blue` 0–255 → `#RRGGBB`. Memory
markers get an index ≥ 8 so they never alias a hot pad.

**Key.** `Tonality` is stored verbatim (rekordbox's own notation — Camelot `8B`,
Open-Key `5d`, or classical `Abm`; `track_keys` keeps it as-is).

**Playlists.** `<NODE Type="0">` = folder (the top `Name="ROOT"` is transparent),
`Type="1"` = playlist whose `<TRACK Key="…"/>` children reference collection
tracks by `TrackID` (`KeyType="0"`). Location-keyed playlists (`KeyType="1"`) are
rare and import with no members.

**Import behaviour.** Idempotent by `(volume_uuid, relative_path)` — shared
identity with the other importers — lazy; `metadata_source('rekordbox')` +
imported grid / key / cues / loops + the playlist mirror. *(Validated against a
real rekordbox 7.2 export: 18 tracks all resolved, 14 grids; the export carried
no `POSITION_MARK` cues, `Tonality`, or populated user playlists, so those paths
are pinned by synthetic fixtures — re-confirm against an export that has them.)*
Run headless with `dub import --rekordbox <rekordbox.xml>`.

**Fuzz target.** `fuzz/fuzz_targets/fuzz_rekordbox_xml.rs` over `parse_xml`.

### iTunes / Apple Music — **shipped (M12c)**

- **Library:** `~/Music/iTunes/iTunes Library.xml` (legacy) or
  `~/Music/Music/Library.xml` (Apple Music "Share Library XML" export).
- Apple **plist** (XML). No beat grids, no cues — metadata + playlists only.
- Parser: `crates/dub-library/src/itunes.rs` (streaming `quick-xml`); adapter:
  `crates/dub-library/src/itunes_import.rs`.

**Shape.** A top `<dict>` whose `Tracks` key is a dict of `trackID → <dict>`
and whose `Playlists` key is an array of playlist `<dict>`s. Plist values pair
positionally: `<key>NAME</key>` then a value element (`<string>`, `<integer>`,
`<true/>`, `<dict>`, `<array>`, …). The Tracks dict is streamed entry-by-entry
(never held whole). Track fields read: `Name` / `Artist` / `Album` / `Composer`
/ `Genre` / `BPM` (integer) / `Year` / `Total Time` (ms) / `Location`.

**Paths.** `Location` is a percent-encoded `file://` URL
(`file:///Users/dj/Music/a%20b.mp3` or `file://localhost/…`) → strip the scheme
+ optional `localhost`, percent-decode → absolute path. Non-`file://` (remote /
streaming) entries are skipped.

**Playlists.** The `Playlists` array → `imported_crates('itunes')`, nested via
`Parent Persistent ID` (resolved against `Playlist Persistent ID`, forward refs
handled). **Skipped:** the `Master` library playlist and any with a
`Distinguished Kind` (the built-in Music / Films / Downloaded / Audiobooks /
TV Programmes lists) — only user playlists + folders become crate nodes.

**Import behaviour.** Idempotent by `(volume_uuid, relative_path)` — shared
identity with the other importers. Lazy; metadata-source `'itunes'` only (no
grid/key/cue). *(Validated against a real export: 2022 tracks + 166 playlists
parsed; the 158 user playlists/folders mirror cleanly — iTunes allows duplicate
playlist names at one level, so `imported_crates` has no name-uniqueness
constraint, schema v6.)* Run headless with `dub import --itunes <Library.xml>`.

**Fuzz target.** `fuzz/fuzz_targets/fuzz_itunes_xml.rs` over `parse_library`.

### Lexicon DJ

- **Lexicon owns its own database** but its primary mode is *exporting* to
  Serato / Traktor / rekordbox / iTunes.
- We don't read Lexicon's native format directly; we read whichever target
  format Lexicon has exported to. Lexicon → rekordbox.xml → us.

## Common concerns

- **Dub never modifies source library files or databases.** Read-only always.
  Source files open with `O_RDONLY` semantics; no advisory locks; concurrent
  use of Serato / Traktor / rekordbox is fine.
- **External sources are browse-only until played.** Enabling Serato / Traktor /
  rekordbox / iTunes mirrors that app's library for *browsing* (its source node
  + playlist tree), but a track only enters Dub's collection ("All Tracks") when
  the DJ folder-imports it or **plays** it from a node — `tracks.in_collection`,
  schema v7; PRD §8.4.1. The importers still mint one `tracks` row per file so
  playlists and shared-identity dedupe resolve; those rows just start
  `in_collection = 0`. The importers never set the flag — only the folder
  importer and the play-history path do.
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
