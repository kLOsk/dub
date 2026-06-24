# Dub library schema — public API reference

> This document is normative. The Dub SQLite schema is part of Dub's
> public API surface per [PRD §8.7](PRD.md#87-schema-as-public-api). We
> do not break the contract here without a documented migration path
> and a `schema_version` bump.

The Dub library is a single SQLite database at
`~/Library/Application Support/Dub/library.sqlite`. It holds the user's
canonical track identity, per-source metadata, beatgrids, cues, loops,
Dub crates, mirrors of imported source-library crates, fingerprints,
play history, and analysis cache.

Goals (in priority order):

1. **Survive file system events.** Renames, drive moves, re-encodes,
   ejected SSDs, drives mounting at slightly different paths between
   reboots. The `volumes` + `track_files` design (path-by-volume-UUID,
   PRD §8.2) means none of these break a track's identity.
2. **Per-source opinion preservation.** We never collapse Serato's
   "Dilla, J" and rekordbox's "J Dilla" into one row at import time.
   The browser picks a winner per column via a documented priority
   chain, but every source's verbatim opinion is queryable.
3. **Lossless round-trip on export.** Imported hot cues and loops are
   stored from v1.0 day one even though the v1 UI does not edit them,
   so the M11f rekordbox-XML exporter can round-trip them back into
   Serato / Traktor / rekordbox.
4. **Documented, open, queryable by third parties.** No encryption, no
   binary blobs except where standard (fingerprints), no proprietary
   formats. A user who wants to leave Dub takes their data with them.

## Schema versioning and migration policy

A single `schema_version` row tracks the current applied schema. The
v1.0 baseline is **version 1**; the current applied version is **6**.
Each bump is additive (new tables / columns with safe defaults / indexes),
so a third-party reader that ignores the new columns keeps working. Any
change to the table set, column set, indexes, FTS5 definition, or trigger
logic requires a version bump and a migration step.

### Version history

| Version | Milestone | Change |
| --- | --- | --- |
| 1       | M11a   | Initial schema. |
| 2       | M11d.4 | `track_files.is_missing INTEGER NOT NULL DEFAULT 0` + `track_files.last_checked_at INTEGER` + partial index `idx_track_files_missing` + index `idx_track_files_last_checked`. Backs PRD §8.5.5 missing-files scanner + Relocate panel. Additive only. |
| 3       | M11c.2 | `track_keys` table (one musical key per source per track) + `idx_one_active_key_per_track` partial unique index + `idx_track_keys_track_id`. `analysis_cache.has_active_key INTEGER NOT NULL DEFAULT 0`. Backs Camelot key detection. |
| 4       | M11d.7 | `tracks.grid_locked INTEGER NOT NULL DEFAULT 0` + `tracks.grid_drift_quality REAL`. Per-track beat-grid lock (locked tracks skip auto re-analysis on reload) + drift-quality indicator. |
| 5       | PRD-BEATS C2 (round 4) | `track_beatgrids.bar_phase INTEGER NOT NULL DEFAULT 0 CHECK (0 ≤ bar_phase < 16)`. Makes the downbeat phase a first-class scalar so "set the 1" is a pure rotation (BPM + anchor unchanged) instead of a grid rebuild. |
| 6       | M12c | Rebuild `imported_crates` / `imported_crate_tracks` without the `UNIQUE (source, parent, name)` constraint — external sources (iTunes) allow duplicate playlist names at one level. Truncate-and-rewrite mirror, so the drop+recreate loses nothing; also backfills the tables on DBs created before they existed. |

### Backward compatibility contract

* **Additive changes** (new tables, new columns with sensible defaults,
  new indexes) are version bumps but do not break third-party readers
  that ignore unknown tables / columns.
* **Renames, removals, type changes** require a version bump and a
  documented migration. We will not silently change a column's
  semantics. We will not delete data on migration; renamed columns
  retain their old column under an `_legacy` suffix for at least one
  full minor-version cycle.
* **Forward compatibility is not guaranteed.** A database touched by
  a newer Dub may have a `schema_version > SCHEMA_VERSION`; an older
  Dub that opens it must refuse to write but may read the tables it
  understands. Dub currently ships a strict
  `schema_version <= SCHEMA_VERSION` write guard
  (`LibraryError::SchemaTooNew`).

### Migration runner

The migration runner is a sorted list of `(target_version, sql_script)`
pairs. The runner reads the current version, applies every script
with `target_version > current_version` inside a single transaction
per script, and bumps `schema_version` at the end of each. The
migration runner is itself versioned: each migration script is
idempotent against partial application (every `CREATE TABLE` uses
`IF NOT EXISTS`, etc.) so a crash mid-migration leaves a consistent
state.

```sql
CREATE TABLE IF NOT EXISTS schema_version (
    -- single-row table; one INSERT at initial schema creation,
    -- one UPDATE per migration applied.
    version    INTEGER NOT NULL,
    applied_at INTEGER NOT NULL  -- unix epoch seconds
);
```

## Conventions

### Identifiers and timestamps

* **Canonical track identity** (`tracks.id`) is a 36-character lowercase
  RFC 4122 UUID stored as `TEXT`. We choose TEXT over BLOB because
  third-party tools that read `library.sqlite` benefit from a
  human-readable identifier in `EXPLAIN` output and ad-hoc queries.
* **Volume identity** (`volumes.volume_uuid`) is whatever the
  filesystem returns. On macOS this is the volume UUID from
  `NSURL`'s `volumeUUIDStringKey` resource key (also available via
  `statfs(2)` + `getattrlist(2)`). Stored as `TEXT`. We do not
  generate volume UUIDs ourselves.
* **All timestamps are unix epoch seconds (`INTEGER`)** unless
  documented otherwise. `play_history.timestamp` is unix epoch
  *milliseconds* because the Played From / Played Into analysis
  (M11d-history) needs sub-second event ordering when a load is
  followed rapidly by a play-start.
* **Integer surrogate keys** are `INTEGER PRIMARY KEY` (which in
  SQLite is the implicit `rowid` alias and gives us auto-increment
  for free). Canonical tracks use a TEXT UUID instead of an INTEGER
  surrogate because the UUID is the stable identity across export
  round-trips (M11f rekordbox-XML) and is the natural foreign key in
  per-source metadata rows.

### Soft enums (TEXT-as-enum)

Several columns hold one of a small enumerated set of values. We store
these as `TEXT` rather than `INTEGER` for self-documenting queries,
guarded by `CHECK` constraints. The cost of a few bytes per row is
negligible at our scale and the readability win on ad-hoc inspection
is substantial.

| Column | Allowed values |
|---|---|
| `track_metadata_source.source` | `'serato'`, `'traktor'`, `'rekordbox'`, `'itunes'`, `'id3'`, `'filename'` |
| `track_beatgrids.source` | `'serato'`, `'traktor'`, `'rekordbox'`, `'itunes'`, `'auto'`, `'user_tap'` |
| `track_keys.source` | `'serato'`, `'traktor'`, `'rekordbox'`, `'itunes'`, `'mixedinkey'`, `'id3'`, `'auto'`, `'user'` |
| `track_cues.source` | `'serato'`, `'traktor'`, `'rekordbox'`, `'itunes'`, `'user'` |
| `track_loops.source` | Same as `track_cues.source` |
| `imported_crates.source` | `'serato'`, `'traktor'`, `'rekordbox'`, `'itunes'` |
| `play_history.event_type` | `'load'`, `'play_start'`, `'play_end'`, `'transition_in'`, `'transition_out'` |

### Cascading deletes

Foreign keys use `ON DELETE CASCADE` for child rows whose existence is
meaningless without the parent (e.g. `track_files` rows orphan if a
`tracks` row is explicitly deleted by future v1.x merge / consolidate
operations). Foreign keys to siblings (`play_history.from_track_id`,
`tracks.duplicate_link_track_id`) use `ON DELETE SET NULL` so the
historical event survives the loss of the referenced track.

**`PRAGMA foreign_keys = ON`** must be set on every connection. SQLite's
default is OFF for historical reasons; the migration runner sets it
unconditionally.

### `PRAGMA`s

The library opens each connection with:

```
PRAGMA journal_mode = WAL;        -- concurrent readers, single writer
PRAGMA synchronous = NORMAL;      -- WAL recommendation; durability across crashes
PRAGMA foreign_keys = ON;         -- enforce all FKs declared in the schema
PRAGMA temp_store = MEMORY;       -- temp tables in RAM
PRAGMA mmap_size = 268435456;     -- 256 MB; the file is small, this is fine
```

WAL is critical: the importer worker thread writes while the browser
read query streams rows. Without WAL the writer blocks all readers
for the duration of an import.

## Tables

### `tracks` — canonical track identity

```sql
CREATE TABLE IF NOT EXISTS tracks (
    id                          TEXT    PRIMARY KEY NOT NULL,
    fingerprint_id              INTEGER REFERENCES fingerprints(id) ON DELETE SET NULL,
    duration_ms                 INTEGER,
    -- Set when version-aware dedupe (§8.1) detects a "potential
    -- duplicate" of an existing track. NULL otherwise. The link is
    -- bidirectional: A->B implies B->A.
    duplicate_link_track_id     TEXT    REFERENCES tracks(id) ON DELETE SET NULL,
    -- Set when the user manually merges two tracks (v1.x). The
    -- merge target is the survivor; the merged-from row is kept
    -- as a tombstone so re-import doesn't resurrect it.
    explicit_merge_target_id    TEXT    REFERENCES tracks(id) ON DELETE SET NULL,
    -- Beat-grid lock (schema v4, M11d.7). When 1, the loaded deck
    -- adopts the active grid as-is and skips auto re-analysis on
    -- reload. `grid_drift_quality` is the analyzer's drift slope
    -- (ms/min) over the kept beats; a ⚠ shows on BPM when the grid
    -- drifts >= 3 ms/min and is unlocked.
    grid_locked                 INTEGER NOT NULL DEFAULT 0 CHECK (grid_locked IN (0, 1)),
    grid_drift_quality          REAL,
    created_at                  INTEGER NOT NULL,
    updated_at                  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tracks_fingerprint    ON tracks(fingerprint_id);
CREATE INDEX IF NOT EXISTS idx_tracks_duplicate_link ON tracks(duplicate_link_track_id);
CREATE INDEX IF NOT EXISTS idx_tracks_merge_target   ON tracks(explicit_merge_target_id);
```

A `tracks` row is the canonical identity. `duration_ms` is denormalised
here for fast browsing-list rendering (computed from the active grid
or from any `track_files.sample_rate × file_size / bytes_per_sample`
estimate at import time; refined when the file is decoded for waveform
analysis).

### `fingerprints` — Chromaprint identity

```sql
CREATE TABLE IF NOT EXISTS fingerprints (
    id                INTEGER PRIMARY KEY,
    -- Chromaprint fingerprint as raw uint32 vector, little-endian.
    -- See "Fingerprint parameters" below for the exact Chromaprint
    -- configuration used.
    chromaprint_blob  BLOB    NOT NULL,
    duration_ms       INTEGER NOT NULL,
    -- Auxiliary signature data used for fast first-pass filtering
    -- before a full Chromaprint similarity comparison.
    file_size         INTEGER,
    sample_rate       INTEGER,
    channel_count     INTEGER,
    computed_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_fingerprints_duration ON fingerprints(duration_ms);
```

The fingerprint is **keyed against the canonical recording, not the
file**. Two `track_files` rows pointing at the same recording from
different drives share one `fingerprints` row via their `tracks.fingerprint_id`.
This is what makes the `analysis_cache` (keyed by `fingerprint_id`)
survive file moves and dedupe merges.

### `volumes` — path-by-volume-UUID directory

```sql
CREATE TABLE IF NOT EXISTS volumes (
    volume_uuid             TEXT    PRIMARY KEY NOT NULL,
    display_name            TEXT    NOT NULL,
    last_known_mount_point  TEXT,
    last_seen_at            INTEGER NOT NULL,
    -- 1 for the boot volume / internal drive, 0 for external drives.
    -- Drives the missing-files UI: external drive ejected = expected,
    -- internal drive "missing" = problem.
    is_internal             INTEGER NOT NULL DEFAULT 0 CHECK (is_internal IN (0, 1))
);
```

The `volume_uuid` is whatever the filesystem returns. On macOS it is
the volume UUID from `NSURL.resourceValues(forKeys: [.volumeUUIDStringKey])`,
or equivalently from `statfs(2)` + `getattrlist(2)` on the
`f_mntfromname`. We never generate volume UUIDs ourselves; if a volume
exposes no UUID (rare; some network filesystems), the import refuses
to register tracks from it with a clear error.

### `track_files` — files on disk

```sql
CREATE TABLE IF NOT EXISTS track_files (
    id              INTEGER PRIMARY KEY,
    track_id        TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    volume_uuid     TEXT    NOT NULL REFERENCES volumes(volume_uuid) ON DELETE CASCADE,
    -- Path relative to the volume root, no leading slash. Stored
    -- verbatim (case-sensitive on case-sensitive filesystems).
    relative_path   TEXT    NOT NULL,
    codec           TEXT,
    sample_rate     INTEGER,
    bit_depth       INTEGER,
    channel_count   INTEGER,
    file_size       INTEGER,
    mtime           INTEGER,
    last_seen_at    INTEGER NOT NULL,
    -- M11d.4 / schema v2 — set by the background scanner (PRD §8.5.5).
    -- 1 = `access()` confirmed the file is gone (or the parent volume
    -- is offline). 0 = file resolves cleanly on the current mount
    -- point. Reset to 0 by `relocate_track` on a successful re-attach.
    is_missing      INTEGER NOT NULL DEFAULT 0
                    CHECK (is_missing IN (0, 1)),
    -- M11d.4 / schema v2 — wall-clock (unix-seconds) of the most
    -- recent scanner probe. NULL means "never probed" — scanner
    -- prioritises those rows first via the
    -- `idx_track_files_last_checked` index below.
    last_checked_at INTEGER,
    UNIQUE (volume_uuid, relative_path)
);
CREATE INDEX IF NOT EXISTS idx_track_files_track  ON track_files(track_id);
CREATE INDEX IF NOT EXISTS idx_track_files_volume ON track_files(volume_uuid);
-- M11d.4 / schema v2 — partial index over rows the missing-files
-- footer query reads (`COUNT(*)` over track_files where every row
-- has is_missing = 1). Storing only `is_missing = 1` rows keeps the
-- index size bounded by the *missing* set, not the library.
CREATE INDEX IF NOT EXISTS idx_track_files_missing
    ON track_files(is_missing) WHERE is_missing = 1;
-- M11d.4 / schema v2 — backs the scanner's `ORDER BY
-- last_checked_at IS NOT NULL, last_checked_at ASC` predicate. NULL
-- rows sort first (never probed), then stamped rows ascending.
CREATE INDEX IF NOT EXISTS idx_track_files_last_checked
    ON track_files(last_checked_at);
```

One canonical track can have many `track_files` (different encodings,
backup copies, files on the touring SSD and the studio drive). The
resolution order on load (per PRD §8.2):

1. Volume UUID lookup → mount point + relative path.
2. Last-known mount point (volumes table) + relative path.
3. Basename + fingerprint search across known volumes.
4. Prompt the user.

**Missing-file lifecycle** (PRD §8.5.5, M11d.4): the background scanner
batches `track_files` rows ordered "stalest first" (`last_checked_at IS
NOT NULL, last_checked_at ASC`), probes each absolute path via the
platform `fileExists` primitive, and writes the verdict back through
`mark_file_state`. A track is reported as missing in the browser
footer iff *every* `track_files` row for it has `is_missing = 1` — a
track with one missing and one healthy file is still reachable. The
Relocate panel calls `relocate_track`, which inserts a *fresh*
`track_files` row at the user-supplied path (never deletes the
original) so the touring SSD coming back online resurrects the
previous path on the next scanner pass.

### `track_metadata_source` — per-source verbatim opinion

```sql
CREATE TABLE IF NOT EXISTS track_metadata_source (
    id              INTEGER PRIMARY KEY,
    track_id        TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source          TEXT    NOT NULL CHECK (source IN
                    ('serato', 'traktor', 'rekordbox', 'itunes', 'id3', 'filename')),
    artist          TEXT,
    title           TEXT,
    album           TEXT,
    genre           TEXT,
    comment         TEXT,
    composer        TEXT,
    year            INTEGER,
    track_number    INTEGER,
    bpm             REAL,
    -- Camelot / Open Key / classical notation; stored as the source
    -- reports it. Browser normalises for display only.
    key             TEXT,
    gain_db         REAL,
    -- 0-5 stars. Sources that use 0-100 or 0-255 are normalised on
    -- import. NULL for sources that don't carry ratings.
    rating          INTEGER,
    -- The version-token recognised by §8.1's parser
    -- ('clean' / 'dirty' / 'instrumental' / 'acapella' / 'radio' /
    -- 'edit' / 'extended' / 'club' / 'dub' / 'vip' / 'remix' /
    -- 'remaster' / 'mono' / 'stereo' / 'intro' / 'outro' / 'short' /
    -- 'long' / '7in' / '12in' / 'lp'). NULL if no token detected.
    -- Stored as canonical lowercase form.
    version_token   TEXT,
    imported_at     INTEGER NOT NULL,
    UNIQUE (track_id, source)
);
CREATE INDEX IF NOT EXISTS idx_metadata_source ON track_metadata_source(source);
```

Per PRD §8.1, displayed-value priority chain is
`serato > rekordbox > traktor > id3 > filename`. The browser computes
the displayed value at query time using a `COALESCE` chain over a
pivoted view; there is no materialized "winner" column to keep in
sync.

### `track_beatgrids` — one grid per source per track

```sql
CREATE TABLE IF NOT EXISTS track_beatgrids (
    id            INTEGER PRIMARY KEY,
    track_id      TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source        TEXT    NOT NULL CHECK (source IN
                  ('serato', 'traktor', 'rekordbox', 'itunes', 'auto', 'user_tap')),
    -- Downbeat anchor in seconds from track start.
    anchor_secs   REAL    NOT NULL,
    bpm           REAL    NOT NULL,
    -- Downbeat phase (schema v5, PRD-BEATS C2). `bar_phase ∈ [0,
    -- beats_per_bar)` is the index i such that beats[i], beats[i +
    -- beats_per_bar], … are the downbeats (bar position 1). Lets
    -- "set the 1" rotate the grid without touching bpm/anchor.
    bar_phase     INTEGER NOT NULL DEFAULT 0 CHECK (bar_phase >= 0 AND bar_phase < 16),
    -- Exactly one grid per track is_active at a time. Enforced
    -- by the partial unique index below.
    is_active     INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
    captured_at   INTEGER NOT NULL,
    UNIQUE (track_id, source)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_one_active_grid_per_track
    ON track_beatgrids(track_id) WHERE is_active = 1;
```

Single-anchor flex grid only in v1 per PRD §8.3.2. Multi-anchor warp
is a v2 consideration. The browser surfaces a ⚠ on tracks where the
imported and `auto` grids disagree by more than 5 % BPM or 50 ms
anchor over the first 32 bars (PRD §8.3 cross-validation; the
comparison is computed at query time, not stored).

### `track_keys` — one musical key per source per track (schema v3, M11c.2)

```sql
CREATE TABLE IF NOT EXISTS track_keys (
    id                INTEGER PRIMARY KEY,
    track_id          TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source            TEXT    NOT NULL CHECK (source IN
                      ('serato', 'traktor', 'rekordbox', 'itunes',
                       'mixedinkey', 'id3', 'auto', 'user')),
    -- Canonical Camelot, e.g. '8B' for C major, '5A' for C minor.
    key_notation      TEXT    NOT NULL,
    -- Whatever the source wrote verbatim ('C major', 'Cm', '5d',
    -- '8B'). Preserved so rekordbox-XML export round-trips exactly.
    original_notation TEXT,
    -- [0.0, 1.0] for auto-detected rows; NULL for imported / user
    -- rows (which by definition carry no algorithmic confidence).
    confidence        REAL,
    is_active         INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
    captured_at       INTEGER NOT NULL,
    UNIQUE (track_id, source)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_one_active_key_per_track
    ON track_keys(track_id) WHERE is_active = 1;
CREATE INDEX IF NOT EXISTS idx_track_keys_track_id
    ON track_keys(track_id);
```

Same `(source, is_active, captured_at)` shape as `track_beatgrids`.
PRD §8.3.2 cross-validation is relative-major aware: the browser only
flags ⚠ when two sources sit in **different** Camelot families (`8B`
vs `5A`); relative-major pairs (`8B` vs `8A` — C major vs A minor)
are a legitimate Krumhansl-Kessler template ambiguity and are not
flagged. The flag is computed at query time in `TRACK_ROW_SELECT`
via `COUNT(DISTINCT substr(key_notation, 1, length-1)) > 1`.

### `track_cues` — hot cues (performance cues — authored in v1)

```sql
CREATE TABLE IF NOT EXISTS track_cues (
    id              INTEGER PRIMARY KEY,
    track_id        TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source          TEXT    NOT NULL CHECK (source IN
                    ('serato', 'traktor', 'rekordbox', 'itunes', 'user')),
    -- 0-based slot index. Serato 0-7, rekordbox 0-7, Traktor 0-15.
    cue_index       INTEGER NOT NULL,
    position_secs   REAL    NOT NULL,
    name            TEXT,
    -- "#RRGGBB" hex string when source carries it; NULL otherwise.
    color           TEXT,
    kind            TEXT    NOT NULL DEFAULT 'hot_cue' CHECK (kind IN
                    ('hot_cue', 'memory', 'load', 'loop_in', 'loop_out')),
    imported_at     INTEGER NOT NULL,
    UNIQUE (track_id, source, cue_index)
);
CREATE INDEX IF NOT EXISTS idx_track_cues_track ON track_cues(track_id);
```

Hot cues are a **v1 performance feature** (PRD §6.2.1): the user sets,
recalls, and clears them on the CUE pads (keyboard 1–4 / pad controller),
persisted with `source='user'`. Imported cues (`serato` / `traktor` /
`rekordbox` / `itunes`) are also stored from v1.0 day one so the M11f
rekordbox-XML exporter round-trips them losslessly. *(Earlier drafts
deferred all cues to v2; that conflated hot cues with the CDJ-style cue
button — see PRD §6.6.)*

### `track_loops` — saved loops (stored from v1, surfaced in v1.x)

```sql
CREATE TABLE IF NOT EXISTS track_loops (
    id              INTEGER PRIMARY KEY,
    track_id        TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source          TEXT    NOT NULL CHECK (source IN
                    ('serato', 'traktor', 'rekordbox', 'itunes', 'user')),
    loop_index      INTEGER NOT NULL,
    in_secs         REAL    NOT NULL,
    out_secs        REAL    NOT NULL,
    name            TEXT,
    color           TEXT,
    is_locked       INTEGER NOT NULL DEFAULT 0 CHECK (is_locked IN (0, 1)),
    imported_at     INTEGER NOT NULL,
    UNIQUE (track_id, source, loop_index)
);
CREATE INDEX IF NOT EXISTS idx_track_loops_track ON track_loops(track_id);
```

Per PRD §6.6, saved loop slots are v1.x. Imported loops stored from
v1.0 day one for round-trip discipline (same rationale as
`track_cues`). v1 ships ephemeral loops only.

### `crates` and `crate_tracks` — Dub crates (user-editable)

```sql
CREATE TABLE IF NOT EXISTS crates (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL,
    parent_crate_id INTEGER REFERENCES crates(id) ON DELETE CASCADE,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (parent_crate_id, name)
);

CREATE TABLE IF NOT EXISTS crate_tracks (
    crate_id    INTEGER NOT NULL REFERENCES crates(id) ON DELETE CASCADE,
    track_id    TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    -- 0-based position within the crate. Reorder by rewriting the
    -- ordinal column; we don't use floating-point sort keys.
    ordinal     INTEGER NOT NULL,
    added_at    INTEGER NOT NULL,
    PRIMARY KEY (crate_id, track_id)
);
CREATE INDEX IF NOT EXISTS idx_crate_tracks_ord ON crate_tracks(crate_id, ordinal);
```

Dub crates are user-editable, full-color, nestable. Per PRD §8.5.1
these are explicitly separated from the read-only `imported_crates`
table so a Serato re-import never clobbers user edits.

### `imported_crates` and `imported_crate_tracks` — source mirror

```sql
CREATE TABLE IF NOT EXISTS imported_crates (
    id                          INTEGER PRIMARY KEY,
    source                      TEXT    NOT NULL CHECK (source IN
                                ('serato', 'traktor', 'rekordbox', 'itunes')),
    name                        TEXT    NOT NULL,
    parent_imported_crate_id    INTEGER REFERENCES imported_crates(id) ON DELETE CASCADE,
    imported_at                 INTEGER NOT NULL
    -- No UNIQUE(source, parent, name): see the schema-v6 note below.
);
CREATE INDEX IF NOT EXISTS idx_imported_crates_source ON imported_crates(source);

CREATE TABLE IF NOT EXISTS imported_crate_tracks (
    imported_crate_id   INTEGER NOT NULL REFERENCES imported_crates(id) ON DELETE CASCADE,
    track_id            TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    ordinal             INTEGER NOT NULL,
    PRIMARY KEY (imported_crate_id, track_id)
);
CREATE INDEX IF NOT EXISTS idx_imported_crate_tracks_ord
    ON imported_crate_tracks(imported_crate_id, ordinal);
```

Read-only mirror of source-library crate/playlist trees. Re-import
truncates and rewrites the subtree for the affected source; never
edited by the user.

**Schema v6** dropped the original `UNIQUE (source, parent_imported_crate_id,
name)` constraint: external sources (notably iTunes) legitimately have
duplicate playlist names at the same level — they key playlists by a persistent
id, not by name. Because the mirror is truncate-and-rewrite per source (no
upsert-by-name), the constraint served no conflict-resolution purpose. The v6
migration drops + recreates both tables (the mirror is rebuilt on the next
import), which also backfills them on any DB created before the tables existed.

### `play_history` — every event, milliseconds

```sql
CREATE TABLE IF NOT EXISTS play_history (
    id                  INTEGER PRIMARY KEY,
    track_id            TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    deck                INTEGER NOT NULL CHECK (deck IN (0, 1)),
    event_type          TEXT    NOT NULL CHECK (event_type IN
                        ('load', 'play_start', 'play_end',
                         'transition_in', 'transition_out')),
    -- Unix epoch MILLISECONDS — not seconds — so rapid event
    -- sequences (load -> play_start within ~200 ms) order
    -- deterministically.
    timestamp_ms        INTEGER NOT NULL,
    duration_played_ms  INTEGER,
    -- Mix-history edges. Set on transition_in / transition_out
    -- events to the other-deck track at the moment of transition.
    from_track_id       TEXT    REFERENCES tracks(id) ON DELETE SET NULL,
    to_track_id         TEXT    REFERENCES tracks(id) ON DELETE SET NULL,
    -- Opaque per-session marker. Same across all events of one
    -- gig / practice run. Lets the future Played From / Played
    -- Into analysis (v1.x) restrict to a single session if needed.
    session_id          TEXT
);
CREATE INDEX IF NOT EXISTS idx_play_history_track
    ON play_history(track_id, timestamp_ms DESC);
CREATE INDEX IF NOT EXISTS idx_play_history_timestamp
    ON play_history(timestamp_ms DESC);
CREATE INDEX IF NOT EXISTS idx_play_history_session
    ON play_history(session_id, timestamp_ms);
```

Capture starts at v1.0 day one (PRD §8.2). The user can disable capture
in Preferences (the table is still present, just not written to). The
v1.0 user-visible surfaces are "Last Played" sort, the "Recently
Played" + "Session History" smart crates, and the deck-header
"↝ usually" hint (M11d-history); the full Played From / Played Into
side panel is v1.x.

**Transition semantics (M11d-history).** The mixer is external
(PRD §1), so Dub never sees the crossfader; transitions are inferred
from deck transport by `dub_library::SessionTracker`:

* A **handover** `X → Y` is recorded at the moment deck X stops
  (pause, needle lift / carrier dropout, eject, or load-replace)
  while the other deck Y is still playing a *different* track.
* **Minimum-play gate:** the outgoing track must have accumulated
  ≥ 30 s of play since load. Timecode cueing (scratch holds, needle
  drops) flips transport constantly; the gate keeps those from
  writing false edges.
* **Duplicate suppression:** re-stopping the same record during one
  mix-out doesn't repeat the edge (consecutive identical `from → to`
  pairs are dropped); a genuine A→B→A→B juggle records all edges.
* **Instant doubles** (same track on both decks) never record.

Each committed handover writes **two rows** — `transition_out` on the
outgoing track and `transition_in` on the incoming track — and **both
rows carry both edge ids** (`from_track_id` *and* `to_track_id`), so
either row alone answers either direction of the §8.5 queries and the
per-track index serves both lookups.

`session_id` is one UUID per app run, minted by the tracker at
construction. `play_start` / `play_end` rows are written per transport
segment (a scratch-heavy session writes many short segments; honest
raw data, aggregated at query time). A play segment open at app quit
is not closed — the final `play_end` of a run can be missing.

### `analysis_cache` — derived per-recording data

```sql
CREATE TABLE IF NOT EXISTS analysis_cache (
    fingerprint_id          INTEGER PRIMARY KEY
                            REFERENCES fingerprints(id) ON DELETE CASCADE,
    -- ITU-R BS.1770 integrated loudness in LUFS (negative dB).
    lufs_i                  REAL,
    -- True-peak in dBTP per BS.1770.
    true_peak_dbtp          REAL,
    -- Path under ~/Library/Caches/Dub/waveforms/ to the M10.5j sidecar.
    -- Stored relative to that directory; missing-file detection
    -- on this is independent of the track_files volume-UUID path.
    waveform_sidecar_path   TEXT,
    -- Booleans cached for the prepared-flag computation. Sourced
    -- from the presence/absence of the corresponding data; we cache
    -- the booleans here to avoid joining across multiple tables on
    -- every browser-row render.
    has_lufs                INTEGER NOT NULL DEFAULT 0 CHECK (has_lufs IN (0, 1)),
    has_waveform            INTEGER NOT NULL DEFAULT 0 CHECK (has_waveform IN (0, 1)),
    has_active_grid         INTEGER NOT NULL DEFAULT 0 CHECK (has_active_grid IN (0, 1)),
    -- Schema v3 (M11c.2): mirrors `has_active_grid` for the key
    -- pipeline. `1` once an `is_active = 1` row lands in
    -- `track_keys` for this fingerprint.
    has_active_key          INTEGER NOT NULL DEFAULT 0 CHECK (has_active_key IN (0, 1)),
    analyzed_at             INTEGER
);
```

Keyed by canonical `fingerprint_id` (not `track_id`) so the cache
survives file moves and dedupe merges. A "prepared" track is one
where `has_lufs AND has_waveform AND has_active_grid AND
has_active_key`; the prepared flag in the browser is computed at
query time from these columns.

### `smart_crates` — user-defined smart crates (v1.x)

```sql
CREATE TABLE IF NOT EXISTS smart_crates (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL UNIQUE,
    -- A SQL fragment usable as a WHERE clause against the canonical
    -- track-query view. Validated at insert time against an allow-list
    -- of columns and operators (v1.x); v1.0 leaves the table empty.
    sql_predicate   TEXT    NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
```

Empty in v1.0 (PRD §8.5.2). The two v1 smart crates ("Recently Played",
"Just Imported") ship as code, not data. The table exists in v1.0 so
the v1.x rule builder lands without a migration.

## Full-text search (FTS5)

Search per PRD §8.5.4. Substring match with `AND` across whitespace-
separated tokens. Operators (`bpm:90-100`, `key:Am`, etc.) are parsed
out of the query string before the FTS5 call and applied as separate
SQL predicates.

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS track_metadata_fts USING fts5(
    artist,
    title,
    album,
    comment,
    -- UNINDEXED columns are not full-text-tokenised; they are stored
    -- as-is for projection in the query result.
    track_metadata_source_id UNINDEXED,
    track_id UNINDEXED,
    source UNINDEXED,
    -- unicode61 with diacritic removal so "Beyoncé" matches "Beyonce"
    -- and vice versa, which is what the DJ types in either form.
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Triggers keep the FTS table in sync with track_metadata_source.
CREATE TRIGGER IF NOT EXISTS trg_metadata_fts_insert
AFTER INSERT ON track_metadata_source BEGIN
    INSERT INTO track_metadata_fts (
        artist, title, album, comment,
        track_metadata_source_id, track_id, source
    ) VALUES (
        new.artist, new.title, new.album, new.comment,
        new.id, new.track_id, new.source
    );
END;

CREATE TRIGGER IF NOT EXISTS trg_metadata_fts_delete
AFTER DELETE ON track_metadata_source BEGIN
    DELETE FROM track_metadata_fts
        WHERE track_metadata_source_id = old.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_metadata_fts_update
AFTER UPDATE ON track_metadata_source BEGIN
    UPDATE track_metadata_fts SET
        artist  = new.artist,
        title   = new.title,
        album   = new.album,
        comment = new.comment,
        source  = new.source
    WHERE track_metadata_source_id = old.id;
END;
```

The FTS table indexes every per-source row (so a track with three
imported sources has three FTS rows). Search returns
`DISTINCT track_id`. The browser then resolves the displayed metadata
through the priority chain.

## Fingerprint parameters

Per PRD §8.7, the Chromaprint algorithm parameters Dub uses are
documented so a third party can re-derive Dub's fingerprints with
any algorithm-2-faithful implementation.

* **Crate**: `rusty-chromaprint` 0.3.x (pure-Rust, MIT/Apache) via
  `dub-fingerprint`. M11b chose this over an FFI wrapper around the
  reference C library (`chromaprint`, LGPL-2.1) for the reasons
  documented in PRD §10.2: license isolation, no C build dep, no
  unsafe FFI surface, simpler distribution. The Chromaprint
  **algorithm** is unchanged; only the implementation crate differs.
* **Algorithm**: Chromaprint algorithm 2 — the same one AcoustID
  uses. `rusty_chromaprint::Configuration::preset_test1()` is the
  invocation that materialises this preset.
* **Sample rate**: 11025 Hz. Multi-channel input is supported
  natively by the algorithm (we pass `channels = 1` or `2`); the
  Chromaprint algorithm internally collapses to mono before its
  chroma analysis.
* **Frame size**: 4096 samples, 2/3 overlap. These are the algorithm-
  2 defaults baked into the preset.
* **Duration window**: full track. Long tracks are not truncated; the
  full fingerprint is what the §8.1 dedupe similarity comparison
  runs against.
* **Storage**: the `chromaprint_blob` column holds the algorithm
  output as little-endian `u32` items, native length (typically
  200–400 items per track), with no header. Length is recoverable
  as `bytes.len() / 4`. Round-trip via
  `Fingerprint::to_blob` / `Fingerprint::from_blob`.

The similarity comparison for dedupe (§8.1) is in
`dub_fingerprint::similarity(&Fingerprint, &Fingerprint) -> f32`:

* Sliding-window alignment over ±30 items (≈ ±3.7 s) so two
  encodings of the same recording with differing lead-in / silence
  trim still register as the same recording.
* At each candidate offset, the overlap-region bitwise Hamming
  distance is normalised to `[0, 1]` as
  `1.0 − (popcount(a XOR b) / (32 × overlap_items))`.
* The window-best score is returned. Empty fingerprints (zero items)
  score `0.0` by definition; overlaps shorter than 8 items also
  return `0.0` because below that the bit count is too small to be
  meaningful and noise produces spurious high scores.
* The threshold for auto-merge is `≥ 0.98` per PRD §8.1.

A third-party reader can compare Dub-stored fingerprints with any
algorithm-2-faithful implementation by deserialising the
`chromaprint_blob` to `u32` items and running the same Hamming
distance computation. Cross-implementation bit-identity is not
guaranteed (the C library and `rusty-chromaprint` differ in the
resampler and in a handful of edge-case bugs the Rust port did not
reproduce), but for our use case — library-internal dedupe — the
fingerprint generator's **self-consistency** is what matters, and
that is preserved.

## File system locations

| What | Where |
|---|---|
| Library database | `~/Library/Application Support/Dub/library.sqlite` |
| WAL companion | `~/Library/Application Support/Dub/library.sqlite-wal` |
| SHM companion | `~/Library/Application Support/Dub/library.sqlite-shm` |
| Waveform sidecars (M10.5j → M11a) | `~/Library/Caches/Dub/waveforms/{fingerprint_hex}.wf` |
| Per-session log | `~/Library/Logs/Dub/session.log` (per PRD §2.2.7) |

The Caches directory is intentionally separate from Application
Support: macOS treats `~/Library/Caches/Dub/` as evictable under disk
pressure, which is correct for the waveform sidecars (regeneratable
from the audio files). The library database is in Application Support
which macOS does not evict, which is correct for the user's
irreplaceable Dub-crate / mix-history / tap-grid data.

## Query examples

### Browser default sort: Last Played

```sql
SELECT
    t.id AS track_id,
    COALESCE(s.title, r.title, k.title, i3.title, f.title)        AS title,
    COALESCE(s.artist, r.artist, k.artist, i3.artist, f.artist)   AS artist,
    g.bpm AS active_bpm,
    g.source AS active_grid_source,
    (SELECT MAX(timestamp_ms) FROM play_history p WHERE p.track_id = t.id)
        AS last_played_ms
FROM tracks t
LEFT JOIN track_metadata_source s  ON s.track_id  = t.id AND s.source  = 'serato'
LEFT JOIN track_metadata_source r  ON r.track_id  = t.id AND r.source  = 'rekordbox'
LEFT JOIN track_metadata_source k  ON k.track_id  = t.id AND k.source  = 'traktor'
LEFT JOIN track_metadata_source i3 ON i3.track_id = t.id AND i3.source = 'id3'
LEFT JOIN track_metadata_source f  ON f.track_id  = t.id AND f.source  = 'filename'
LEFT JOIN track_beatgrids g        ON g.track_id  = t.id AND g.is_active = 1
ORDER BY last_played_ms DESC NULLS LAST;
```

### FTS substring search

```sql
SELECT DISTINCT track_id
FROM track_metadata_fts
WHERE track_metadata_fts MATCH 'donuts dilla';
```

### Played Into (M11d-history; the v1.x side panel reuses it)

```sql
-- Given a just-loaded track, find the N tracks the DJ has most
-- often handed over to from it (deck-header "↝ usually" hint at
-- LIMIT 1). Counts transition_out rows — each handover writes
-- exactly one. Recency breaks count ties.
SELECT to_track_id, COUNT(*) AS n, MAX(timestamp_ms) AS last_ms
FROM play_history
WHERE event_type = 'transition_out'
  AND from_track_id = :loaded_track_id
  AND to_track_id IS NOT NULL
GROUP BY to_track_id
ORDER BY n DESC, last_ms DESC
LIMIT 10;
```

### Session History (M11d-history)

```sql
-- This app run's set list, newest first. The "← from <track>"
-- annotation joins each track's most recent transition_in of the
-- session.
SELECT track_id, MIN(timestamp_ms) AS first_ms
FROM play_history
WHERE session_id = :session_id AND event_type = 'play_start'
GROUP BY track_id
ORDER BY first_ms DESC
LIMIT 200;
```

## What v1.0 does not commit to

* **Forward compatibility with v1.x writers from a v1.0 binary.** A
  v1.0 binary that opens a database written by a v1.x binary detects
  the higher `schema_version` and refuses to write (read-only fallback).
* **A schema migration tool independent of the Dub binary.** The
  migration runner lives inside `dub-library`; third-party readers that
  want forward-compat can implement their own (the schema is open) but
  Dub does not promise a CLI tool until v1.x.
* **Encrypted-at-rest support.** macOS FileVault handles disk-level
  encryption. We do not encrypt the SQLite file; the user's library is
  not a secret from themselves.

## See also

* [PRD §8](PRD.md#8-library) — library subsystem spec
* [PRD §8.7](PRD.md#87-schema-as-public-api) — public-API commitment
* [docs/LIBRARY-FORMATS.md](LIBRARY-FORMATS.md) — source-format notes
  for the M11e Serato importer and M12 Traktor / rekordbox / iTunes
  importers
* [docs/SHIPPED.md](../history/SHIPPED.md) — milestone history
