//! Hot cues — per-track jump markers the DJ sets by ear.
//!
//! Press an empty cue pad to drop a marker at the playhead; press it
//! again to jump back. This is the persistence layer behind that
//! gesture, so cues survive a library reload. Stored in `track_cues`
//! under `source = 'user'` (user-authored, vs the imported
//! `serato/traktor/rekordbox/itunes` rows) and `kind = 'hot_cue'`.
//! Position-only in v1 — the `name` / `color` columns are left NULL.

use rusqlite::params;

use crate::db::Library;
use crate::error::{LibraryError, Result};

/// A persisted hot cue: which pad (`cue_index`, 0-based) and where on
/// the track it points (`position_secs`, from sample 0).
#[derive(Debug, Clone, PartialEq)]
pub struct HotCue {
    /// Which pad this cue belongs to (0-based).
    pub cue_index: u8,
    /// Track position the cue points to, in seconds from sample 0.
    pub position_secs: f64,
}

/// User-authored cue source. The imported sources keep their own rows;
/// the same `(track_id, cue_index)` can therefore exist as both an
/// imported and a user cue without colliding (the UNIQUE key includes
/// `source`).
const USER_SOURCE: &str = "user";

impl Library {
    /// Set (or move) hot cue `cue_index` for `track_id` to
    /// `position_secs`. No-op on a non-finite / negative position.
    pub fn set_hot_cue(&self, track_id: &str, cue_index: u8, position_secs: f64) -> Result<()> {
        if !position_secs.is_finite() || position_secs < 0.0 {
            return Ok(());
        }
        self.connection()
            .execute(
                "INSERT INTO track_cues \
                 (track_id, source, cue_index, position_secs, kind, imported_at) \
                 VALUES (?1, ?2, ?3, ?4, 'hot_cue', strftime('%s','now')) \
                 ON CONFLICT(track_id, source, cue_index) DO UPDATE SET \
                     position_secs = excluded.position_secs",
                params![track_id, USER_SOURCE, i64::from(cue_index), position_secs],
            )
            .map_err(|e| LibraryError::sqlite("set_hot_cue", e))?;
        Ok(())
    }

    /// Clear hot cue `cue_index` for `track_id`. Idempotent.
    pub fn delete_hot_cue(&self, track_id: &str, cue_index: u8) -> Result<()> {
        self.connection()
            .execute(
                "DELETE FROM track_cues \
                 WHERE track_id = ?1 AND source = ?2 AND cue_index = ?3",
                params![track_id, USER_SOURCE, i64::from(cue_index)],
            )
            .map_err(|e| LibraryError::sqlite("delete_hot_cue", e))?;
        Ok(())
    }

    /// All user hot cues for `track_id`, ascending by pad index.
    pub fn hot_cues(&self, track_id: &str) -> Result<Vec<HotCue>> {
        let conn = self.connection();
        let mut stmt = conn
            .prepare(
                "SELECT cue_index, position_secs FROM track_cues \
                 WHERE track_id = ?1 AND source = ?2 AND kind = 'hot_cue' \
                 ORDER BY cue_index",
            )
            .map_err(|e| LibraryError::sqlite("hot_cues_prepare", e))?;
        let rows = stmt
            .query_map(params![track_id, USER_SOURCE], |r| {
                Ok(HotCue {
                    cue_index: u8::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                    position_secs: r.get(1)?,
                })
            })
            .map_err(|e| LibraryError::sqlite("hot_cues_query", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| LibraryError::sqlite("hot_cues_row", e))?);
        }
        Ok(out)
    }

    /// Upsert an imported cue/marker row (`serato` / `traktor` /
    /// `rekordbox` / `itunes`), idempotent on `(track_id, source, cue_index)`.
    /// Unlike the user-authored [`Self::set_hot_cue`], this carries the
    /// source, name, color, and `kind` verbatim from the external library
    /// (the import-round-trip sink, PRD §8.6). A user cue and an imported
    /// cue at the same index coexist (the UNIQUE key includes `source`).
    /// Non-finite / negative positions are skipped (graceful).
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_imported_cue(
        &self,
        track_id: &str,
        source: &str,
        cue_index: u8,
        position_secs: f64,
        name: Option<&str>,
        color: Option<&str>,
        kind: &str,
    ) -> Result<()> {
        if !position_secs.is_finite() || position_secs < 0.0 {
            return Ok(());
        }
        self.connection()
            .execute(
                "INSERT INTO track_cues \
                 (track_id, source, cue_index, position_secs, name, color, kind, imported_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%s','now')) \
                 ON CONFLICT(track_id, source, cue_index) DO UPDATE SET \
                     position_secs = excluded.position_secs, \
                     name          = excluded.name, \
                     color         = excluded.color, \
                     kind          = excluded.kind",
                params![
                    track_id,
                    source,
                    i64::from(cue_index),
                    position_secs,
                    name,
                    color,
                    kind,
                ],
            )
            .map_err(|e| LibraryError::sqlite("upsert_imported_cue", e))?;
        Ok(())
    }

    /// Upsert an imported loop row, idempotent on
    /// `(track_id, source, loop_index)`. `track_loops` has no other writer
    /// (saved-loop slots are v1.x); this is the import-round-trip sink
    /// (PRD §8.6). Skipped when the region is degenerate (out ≤ in) or
    /// non-finite (graceful). `is_locked` defaults off.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_imported_loop(
        &self,
        track_id: &str,
        source: &str,
        loop_index: u8,
        in_secs: f64,
        out_secs: f64,
        name: Option<&str>,
        color: Option<&str>,
    ) -> Result<()> {
        if !in_secs.is_finite() || !out_secs.is_finite() || out_secs <= in_secs || in_secs < 0.0 {
            return Ok(());
        }
        self.connection()
            .execute(
                "INSERT INTO track_loops \
                 (track_id, source, loop_index, in_secs, out_secs, name, color, is_locked, imported_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, strftime('%s','now')) \
                 ON CONFLICT(track_id, source, loop_index) DO UPDATE SET \
                     in_secs  = excluded.in_secs, \
                     out_secs = excluded.out_secs, \
                     name     = excluded.name, \
                     color    = excluded.color",
                params![
                    track_id,
                    source,
                    i64::from(loop_index),
                    in_secs,
                    out_secs,
                    name,
                    color,
                ],
            )
            .map_err(|e| LibraryError::sqlite("upsert_imported_loop", e))?;
        Ok(())
    }

    /// Delete every imported cue for `(track_id, source)`. An importer calls
    /// this before rewriting a track's cues so a re-import is
    /// truncate-and-rewrite — a cue the source dropped leaves no stale row,
    /// and positional cue indices never accumulate. Never touches
    /// `source = 'user'` rows (those are the DJ's own, not import-owned).
    pub fn clear_imported_cues(&self, track_id: &str, source: &str) -> Result<()> {
        self.connection()
            .execute(
                "DELETE FROM track_cues WHERE track_id = ?1 AND source = ?2",
                params![track_id, source],
            )
            .map_err(|e| LibraryError::sqlite("clear_imported_cues", e))?;
        Ok(())
    }

    /// Delete every imported loop for `(track_id, source)`. Sibling of
    /// [`Self::clear_imported_cues`] for the `track_loops` table.
    pub fn clear_imported_loops(&self, track_id: &str, source: &str) -> Result<()> {
        self.connection()
            .execute(
                "DELETE FROM track_loops WHERE track_id = ?1 AND source = ?2",
                params![track_id, source],
            )
            .map_err(|e| LibraryError::sqlite("clear_imported_loops", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Library;

    fn seed_track(lib: &Library, id: &str) {
        lib.connection()
            .execute(
                "INSERT INTO tracks (id, created_at, updated_at) VALUES (?1, 0, 0)",
                rusqlite::params![id],
            )
            .unwrap();
    }

    #[test]
    fn hot_cues_set_move_read_delete_roundtrip() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");

        // Set two cues out of order; read back ascending.
        lib.set_hot_cue("t1", 2, 3.0).unwrap();
        lib.set_hot_cue("t1", 0, 1.5).unwrap();
        let cues = lib.hot_cues("t1").unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].cue_index, 0);
        assert!((cues[0].position_secs - 1.5).abs() < 1e-9);
        assert_eq!(cues[1].cue_index, 2);

        // Re-setting the same slot MOVES it (no duplicate row).
        lib.set_hot_cue("t1", 0, 2.5).unwrap();
        let cues = lib.hot_cues("t1").unwrap();
        assert_eq!(cues.len(), 2);
        assert!((cues[0].position_secs - 2.5).abs() < 1e-9);

        // Delete is idempotent and removes only that slot.
        lib.delete_hot_cue("t1", 0).unwrap();
        lib.delete_hot_cue("t1", 0).unwrap();
        let cues = lib.hot_cues("t1").unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].cue_index, 2);

        // Non-finite / negative positions are no-ops, not errors.
        lib.set_hot_cue("t1", 1, f64::NAN).unwrap();
        lib.set_hot_cue("t1", 1, -1.0).unwrap();
        assert_eq!(lib.hot_cues("t1").unwrap().len(), 1);
    }

    #[test]
    fn imported_cue_roundtrip_refresh_and_user_coexist() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        lib.upsert_imported_cue(
            "t1",
            "serato",
            0,
            12.5,
            Some("Intro"),
            Some("#FF0000"),
            "hot_cue",
        )
        .unwrap();
        // A user cue at the same index coexists (UNIQUE key includes source).
        lib.set_hot_cue("t1", 0, 3.0).unwrap();
        assert_eq!(lib.hot_cues("t1").unwrap().len(), 1);

        let (pos, name, kind): (f64, Option<String>, String) = lib
            .connection()
            .query_row(
                "SELECT position_secs, name, kind FROM track_cues \
                 WHERE track_id = 't1' AND source = 'serato' AND cue_index = 0",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert!((pos - 12.5).abs() < 1e-9);
        assert_eq!(name.as_deref(), Some("Intro"));
        assert_eq!(kind, "hot_cue");

        // Re-import moves it (idempotent — one row), updates name/kind.
        lib.upsert_imported_cue("t1", "serato", 0, 20.0, None, None, "memory")
            .unwrap();
        let n: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM track_cues WHERE track_id = 't1' AND source = 'serato'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn imported_loop_roundtrip_and_skips_degenerate() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        lib.upsert_imported_loop("t1", "traktor", 0, 4.0, 8.0, Some("A"), None)
            .unwrap();
        lib.upsert_imported_loop("t1", "traktor", 1, 8.0, 4.0, None, None)
            .unwrap(); // out <= in -> skipped
        lib.upsert_imported_loop("t1", "traktor", 2, f64::NAN, 8.0, None, None)
            .unwrap(); // non-finite -> skipped
        let n: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM track_loops WHERE track_id = 't1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        let (i, o): (f64, f64) = lib
            .connection()
            .query_row(
                "SELECT in_secs, out_secs FROM track_loops \
                 WHERE track_id = 't1' AND loop_index = 0",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!((i - 4.0).abs() < 1e-9 && (o - 8.0).abs() < 1e-9);
    }
}
