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
}
