//! Hot cues — per-track jump markers the DJ sets by ear.
//!
//! Press an empty cue pad to drop a marker at the playhead; press it
//! again to jump back. This is the persistence layer behind that
//! gesture, so cues survive a library reload. Stored in `track_cues`
//! under `source = 'user'` (user-authored, vs the imported
//! `serato/traktor/rekordbox/itunes` rows) and `kind = 'hot_cue'`.
//!
//! A cue carries an optional `name` and `color` alongside its position.
//! Both columns already existed — the importers have written them since
//! M11e, because Serato and rekordbox cues carry them — and only the
//! user-authored path was dropping them on the floor. A cue the DJ set by
//! ear is now the same shape as one that arrived from another library,
//! which is what lets the browser and the pads render either identically.

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
    /// What the DJ called it — `INTRO`, `FIRST VERSE`. `None` for an
    /// unnamed cue, which renders as the pad number alone.
    pub name: Option<String>,
    /// Colour-label token, from the same palette the browser's row
    /// colours use (`"red"`, `"aqua"`, …). `None` renders in the pad's
    /// own accent rather than picking a colour on the DJ's behalf.
    pub color: Option<String>,
}

/// User-authored cue source. The imported sources keep their own rows;
/// the same `(track_id, cue_index)` can therefore exist as both an
/// imported and a user cue without colliding (the UNIQUE key includes
/// `source`).
const USER_SOURCE: &str = "user";

/// The pad slots. Importers park unslotted memory cues above this range.
const PAD_COUNT: i64 = 8;

/// The cues a deck's pads show for a track, and where they came from.
#[derive(Debug, Clone, PartialEq)]
pub struct PadCues {
    /// `"user"` for cues set (or taken over) in Dub, else the imported
    /// source they come from; `None` when the track has none.
    pub source: Option<String>,
    /// The cues on pads 0–7, ascending by pad.
    pub cues: Vec<HotCue>,
}

impl Library {
    /// Set (or move) hot cue `cue_index` for `track_id` to
    /// `position_secs`. No-op on a non-finite / negative position.
    ///
    /// Deliberately leaves `name` and `color` alone on an update: dropping
    /// a cue again to nudge its position is the common gesture, and it
    /// must not silently erase the label the DJ typed. Use
    /// [`Self::set_hot_cue_label`] to change those.
    pub fn set_hot_cue(&self, track_id: &str, cue_index: u8, position_secs: f64) -> Result<()> {
        if !position_secs.is_finite() || position_secs < 0.0 {
            return Ok(());
        }
        self.adopt_imported_cues(track_id)?;
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

    /// Name and/or colour an existing user cue. `None` clears that field.
    ///
    /// No-op when the cue does not exist — naming a pad you have not set
    /// would otherwise create a cue at position 0, which is a marker the
    /// DJ never placed.
    pub fn set_hot_cue_label(
        &self,
        track_id: &str,
        cue_index: u8,
        name: Option<&str>,
        color: Option<&str>,
    ) -> Result<()> {
        self.adopt_imported_cues(track_id)?;
        self.connection()
            .execute(
                "UPDATE track_cues SET name = ?4, color = ?5 \
                 WHERE track_id = ?1 AND source = ?2 AND cue_index = ?3 \
                   AND kind = 'hot_cue'",
                params![track_id, USER_SOURCE, i64::from(cue_index), name, color],
            )
            .map_err(|e| LibraryError::sqlite("set_hot_cue_label", e))?;
        Ok(())
    }

    /// Clear hot cue `cue_index` for `track_id`. Idempotent.
    pub fn delete_hot_cue(&self, track_id: &str, cue_index: u8) -> Result<()> {
        self.adopt_imported_cues(track_id)?;
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
                "SELECT cue_index, position_secs, name, color FROM track_cues \
                 WHERE track_id = ?1 AND source = ?2 AND kind = 'hot_cue' \
                 ORDER BY cue_index",
            )
            .map_err(|e| LibraryError::sqlite("hot_cues_prepare", e))?;
        let rows = stmt
            .query_map(params![track_id, USER_SOURCE], |r| {
                Ok(HotCue {
                    cue_index: u8::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                    position_secs: r.get(1)?,
                    name: r.get(2)?,
                    color: r.get(3)?,
                })
            })
            .map_err(|e| LibraryError::sqlite("hot_cues_query", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| LibraryError::sqlite("hot_cues_row", e))?);
        }
        Ok(out)
    }

    /// Where a track's pad cues come from: `"user"` once the DJ has set or
    /// taken over any cue in Dub, else the highest imported source in
    /// the DJ's import order (`Library::import_order`, the order their grids
    /// rank in too) with a cue on a pad, else `None`.
    pub fn pad_cue_source(&self, track_id: &str) -> Result<Option<String>> {
        let conn = self.connection();
        // Any cue set in Dub, or a track taken over (schema v10) even if
        // the DJ has since cleared every cue.
        let user: i64 = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM track_cues WHERE track_id = ?1 AND source = ?2) \
                      + (SELECT COUNT(*) FROM track_cue_takeover WHERE track_id = ?1)",
                params![track_id, USER_SOURCE],
                |r| r.get(0),
            )
            .map_err(|e| LibraryError::sqlite("pad_cue_source_user", e))?;
        if user > 0 {
            return Ok(Some(USER_SOURCE.to_string()));
        }
        for source in self.source_prefs()?.cue_order() {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM track_cues WHERE track_id = ?1 AND source = ?2 \
                     AND kind = 'hot_cue' AND cue_index BETWEEN 0 AND ?3",
                    params![track_id, source, PAD_COUNT - 1],
                    |r| r.get(0),
                )
                .map_err(|e| LibraryError::sqlite("pad_cue_source_import", e))?;
            if n > 0 {
                return Ok(Some(source.to_string()));
            }
        }
        Ok(None)
    }

    /// What the deck's pads show for `track_id`: the DJ's own cues, or —
    /// on a track whose cues were never touched in Dub — the hot cues of
    /// the imported source that ranks highest ([`Self::pad_cue_source`]).
    /// Daniel, 2026-11-16: Traktor's (and Serato's, rekordbox's) hot cues
    /// on the pads.
    pub fn pad_cues(&self, track_id: &str) -> Result<PadCues> {
        let Some(source) = self.pad_cue_source(track_id)? else {
            return Ok(PadCues {
                source: None,
                cues: Vec::new(),
            });
        };
        let conn = self.connection();
        let mut stmt = conn
            .prepare(
                "SELECT cue_index, position_secs, name, color FROM track_cues \
                 WHERE track_id = ?1 AND source = ?2 AND kind = 'hot_cue' \
                   AND cue_index BETWEEN 0 AND ?3 \
                 ORDER BY cue_index",
            )
            .map_err(|e| LibraryError::sqlite("pad_cues_prepare", e))?;
        let rows = stmt
            .query_map(params![track_id, source, PAD_COUNT - 1], |r| {
                Ok(HotCue {
                    cue_index: u8::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                    position_secs: r.get(1)?,
                    name: r.get(2)?,
                    color: r.get(3)?,
                })
            })
            .map_err(|e| LibraryError::sqlite("pad_cues_query", e))?;
        let mut cues = Vec::new();
        for row in rows {
            cues.push(row.map_err(|e| LibraryError::sqlite("pad_cues_row", e))?);
        }
        Ok(PadCues {
            source: Some(source),
            cues,
        })
    }

    /// Before the DJ's first cue edit on a track whose pads show an
    /// import's cues: copy that source's cues — hot and memory — to
    /// `user` rows and record the track in `track_cue_takeover`, so the
    /// edit applies to what the DJ is looking at and the import never
    /// comes back over it (a cleared cue stays cleared; the source app is
    /// never written to). A no-op once the track is the DJ's, or when it
    /// has no imported cues.
    fn adopt_imported_cues(&self, track_id: &str) -> Result<()> {
        let Some(source) = self.pad_cue_source(track_id)? else {
            return Ok(());
        };
        if source == USER_SOURCE {
            return Ok(());
        }
        let conn = self.connection();
        conn.execute(
            "INSERT INTO track_cues \
             (track_id, source, cue_index, position_secs, name, color, kind, imported_at) \
             SELECT track_id, ?2, cue_index, position_secs, name, color, kind, \
                    strftime('%s','now') \
             FROM track_cues WHERE track_id = ?1 AND source = ?3 \
               AND kind IN ('hot_cue', 'memory')",
            params![track_id, USER_SOURCE, source],
        )
        .map_err(|e| LibraryError::sqlite("adopt_imported_cues_copy", e))?;
        conn.execute(
            "INSERT OR IGNORE INTO track_cue_takeover (track_id, taken_at) \
             VALUES (?1, strftime('%s','now'))",
            params![track_id],
        )
        .map_err(|e| LibraryError::sqlite("adopt_imported_cues_mark", e))?;
        Ok(())
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

    fn import(lib: &Library, track: &str, source: &str, index: u8, secs: f64, name: &str) {
        lib.upsert_imported_cue(track, source, index, secs, Some(name), None, "hot_cue")
            .unwrap();
    }

    fn pads(lib: &Library, track: &str) -> Vec<(u8, f64)> {
        lib.pad_cues(track)
            .unwrap()
            .cues
            .iter()
            .map(|c| (c.cue_index, c.position_secs))
            .collect()
    }

    /// Daniel, 2026-11-16: Traktor's hot cues on the pads — and Serato's
    /// and rekordbox's. With several, the beat-grid order decides.
    #[test]
    fn pads_show_the_highest_ranked_imports_hot_cues() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        assert_eq!(lib.pad_cues("t1").unwrap().source, None);

        import(&lib, "t1", "traktor", 0, 10.0, "Drop");
        import(&lib, "t1", "traktor", 3, 40.0, "Break");
        let p = lib.pad_cues("t1").unwrap();
        assert_eq!(p.source.as_deref(), Some("traktor"));
        assert_eq!(pads(&lib, "t1"), vec![(0, 10.0), (3, 40.0)]);
        assert_eq!(p.cues[0].name.as_deref(), Some("Drop"));

        import(&lib, "t1", "rekordbox", 1, 20.0, "Verse");
        assert_eq!(
            lib.pad_cue_source("t1").unwrap().as_deref(),
            Some("rekordbox")
        );
        import(&lib, "t1", "serato", 2, 30.0, "Hook");
        assert_eq!(lib.pad_cue_source("t1").unwrap().as_deref(), Some("serato"));
        assert_eq!(
            pads(&lib, "t1"),
            vec![(2, 30.0)],
            "one source's set, never a merge"
        );
    }

    /// Memory cues and anything past pad 8 never fill a pad, and a source
    /// with only those does not claim the pads.
    #[test]
    fn only_pad_slots_reach_the_pads() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        lib.upsert_imported_cue("t1", "serato", 9, 5.0, None, None, "memory")
            .unwrap();
        lib.upsert_imported_cue("t1", "serato", 8, 6.0, None, None, "hot_cue")
            .unwrap();
        import(&lib, "t1", "traktor", 7, 70.0, "Outro");
        assert_eq!(
            lib.pad_cue_source("t1").unwrap().as_deref(),
            Some("traktor")
        );
        assert_eq!(pads(&lib, "t1"), vec![(7, 70.0)]);
    }

    /// The first edit takes the shown set over: the edit lands on what the
    /// DJ sees, the rest of the set comes along, and from then on the
    /// pads are the DJ's — a re-import can no longer change them.
    #[test]
    fn the_first_edit_takes_the_imported_set_over() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        import(&lib, "t1", "traktor", 0, 10.0, "Drop");
        import(&lib, "t1", "traktor", 3, 40.0, "Break");
        lib.upsert_imported_cue("t1", "traktor", 20, 55.0, Some("mem"), None, "memory")
            .unwrap();

        lib.set_hot_cue("t1", 5, 60.0).unwrap();
        let p = lib.pad_cues("t1").unwrap();
        assert_eq!(p.source.as_deref(), Some("user"));
        assert_eq!(pads(&lib, "t1"), vec![(0, 10.0), (3, 40.0), (5, 60.0)]);
        assert_eq!(p.cues[1].name.as_deref(), Some("Break"), "names come along");

        // Traktor moves its drop; Dub's pads keep the DJ's.
        lib.clear_imported_cues("t1", "traktor").unwrap();
        import(&lib, "t1", "traktor", 0, 12.0, "Drop");
        assert_eq!(pads(&lib, "t1")[0], (0, 10.0));

        // The memory cue came along too, for the export.
        let user_memory = lib
            .all_cues("t1")
            .unwrap()
            .into_iter()
            .filter(|c| c.source == "user" && c.kind == "memory")
            .count();
        assert_eq!(user_memory, 1);
    }

    /// Clearing an imported cue clears it — every one of them, even — and
    /// the import never comes back over the empty pads.
    #[test]
    fn a_cleared_imported_cue_stays_cleared() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        import(&lib, "t1", "rekordbox", 0, 10.0, "A");
        import(&lib, "t1", "rekordbox", 1, 20.0, "B");

        lib.delete_hot_cue("t1", 0).unwrap();
        assert_eq!(pads(&lib, "t1"), vec![(1, 20.0)]);
        lib.delete_hot_cue("t1", 1).unwrap();
        let p = lib.pad_cues("t1").unwrap();
        assert!(p.cues.is_empty());
        assert_eq!(p.source.as_deref(), Some("user"));
    }

    /// Renaming or recolouring an imported cue takes the set over too, and
    /// the label lands on the copied cue.
    #[test]
    fn labelling_an_imported_cue_takes_the_set_over() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        import(&lib, "t1", "serato", 2, 30.0, "Hook");
        lib.set_hot_cue_label("t1", 2, Some("CHORUS"), Some("red"))
            .unwrap();
        let p = lib.pad_cues("t1").unwrap();
        assert_eq!(p.source.as_deref(), Some("user"));
        assert_eq!(p.cues[0].name.as_deref(), Some("CHORUS"));
        assert_eq!(p.cues[0].color.as_deref(), Some("red"));
        // The import itself is untouched.
        let serato: Vec<_> = lib
            .all_cues("t1")
            .unwrap()
            .into_iter()
            .filter(|c| c.source == "serato")
            .collect();
        assert_eq!(serato[0].name.as_deref(), Some("Hook"));
    }

    /// A track with no imports behaves as before: a cue set in Dub is a
    /// user cue, and clearing the last one leaves no take-over behind.
    #[test]
    fn a_track_without_imports_is_untouched_by_take_over() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        lib.set_hot_cue("t1", 0, 1.0).unwrap();
        lib.delete_hot_cue("t1", 0).unwrap();
        assert_eq!(lib.pad_cue_source("t1").unwrap(), None);
        // An import arriving later fills the pads.
        import(&lib, "t1", "traktor", 0, 10.0, "Drop");
        assert_eq!(pads(&lib, "t1"), vec![(0, 10.0)]);
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

    #[test]
    fn a_user_cue_carries_name_and_colour() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        lib.set_hot_cue("t1", 0, 12.5).unwrap();
        lib.set_hot_cue_label("t1", 0, Some("INTRO"), Some("aqua"))
            .unwrap();

        let cues = lib.hot_cues("t1").unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].name.as_deref(), Some("INTRO"));
        assert_eq!(cues[0].color.as_deref(), Some("aqua"));
    }

    /// Re-dropping a cue to nudge its position is the common gesture. It
    /// must move the marker and leave the label alone — silently erasing
    /// a name the DJ typed because they adjusted the position by 40 ms
    /// would be worse than not having names at all.
    #[test]
    fn moving_a_cue_keeps_its_label() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        lib.set_hot_cue("t1", 2, 10.0).unwrap();
        lib.set_hot_cue_label("t1", 2, Some("DROP"), Some("red"))
            .unwrap();

        lib.set_hot_cue("t1", 2, 10.04).unwrap();

        let cue = &lib.hot_cues("t1").unwrap()[0];
        assert!((cue.position_secs - 10.04).abs() < 1e-9, "position moved");
        assert_eq!(cue.name.as_deref(), Some("DROP"), "label survived");
        assert_eq!(cue.color.as_deref(), Some("red"));
    }

    /// Naming a pad that holds no cue must not conjure one at 0.0 — that
    /// is a marker the DJ never placed, and it would jump the deck to the
    /// top of the track on the next press.
    #[test]
    fn labelling_an_empty_pad_creates_nothing() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        lib.set_hot_cue_label("t1", 1, Some("GHOST"), None).unwrap();
        assert!(lib.hot_cues("t1").unwrap().is_empty());
    }

    #[test]
    fn a_label_can_be_cleared() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        lib.set_hot_cue("t1", 0, 5.0).unwrap();
        lib.set_hot_cue_label("t1", 0, Some("TEMP"), Some("blue"))
            .unwrap();
        lib.set_hot_cue_label("t1", 0, None, None).unwrap();

        let cue = &lib.hot_cues("t1").unwrap()[0];
        assert!(cue.name.is_none());
        assert!(cue.color.is_none());
    }

    /// An unnamed cue is the default and must stay legal — most cues are
    /// dropped by ear mid-listen and never named.
    #[test]
    fn an_unnamed_cue_reads_back_as_none() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "t1");
        lib.set_hot_cue("t1", 3, 88.0).unwrap();
        let cue = &lib.hot_cues("t1").unwrap()[0];
        assert!(cue.name.is_none());
        assert!(cue.color.is_none());
    }
}

/// One stored cue, whatever source wrote it (M11f export).
///
/// [`Library::hot_cues`] deliberately returns only the DJ's own cues,
/// because that is what the deck plays. Export needs the other kind
/// too: the whole point of storing imported cues from v1.0 day one
/// (PRD §8.6) is that a DJ who imports a Serato library and exports to
/// rekordbox gets their hot cues back on the other side.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredCue {
    /// `serato` / `traktor` / `rekordbox` / `itunes` / `user`.
    pub source: String,
    /// Pad slot for a hot cue; memory cues are indexed above the pads.
    pub cue_index: i64,
    /// Seconds from track start.
    pub position_secs: f64,
    /// Label, if the source carried one.
    pub name: Option<String>,
    /// `#RRGGBB`, if the source carried one.
    pub color: Option<String>,
    /// `hot_cue` / `memory` / `load` / `loop_in` / `loop_out`.
    pub kind: String,
}

/// One stored loop (M11f export). Loops had no read path at all before
/// export needed one — they were write-only since M11e.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredLoop {
    /// Source that wrote it.
    pub source: String,
    /// Pad slot.
    pub loop_index: i64,
    /// Loop in-point, seconds.
    pub in_secs: f64,
    /// Loop out-point, seconds.
    pub out_secs: f64,
    /// Label, if any.
    pub name: Option<String>,
    /// `#RRGGBB`, if any.
    pub color: Option<String>,
}

impl Library {
    /// Every cue on a track, from every source, ordered by source then
    /// index.
    ///
    /// # Errors
    ///
    /// [`LibraryError::Sqlite`] if the query fails.
    pub fn all_cues(&self, track_id: &str) -> Result<Vec<StoredCue>> {
        let conn = self.connection();
        let mut stmt = conn
            .prepare(
                "SELECT source, cue_index, position_secs, name, color, kind \
                 FROM track_cues WHERE track_id = ?1 ORDER BY source, cue_index",
            )
            .map_err(|e| LibraryError::sqlite("all_cues_prepare", e))?;
        let rows = stmt
            .query_map(params![track_id], |r| {
                Ok(StoredCue {
                    source: r.get(0)?,
                    cue_index: r.get(1)?,
                    position_secs: r.get(2)?,
                    name: r.get(3)?,
                    color: r.get(4)?,
                    kind: r.get(5)?,
                })
            })
            .map_err(|e| LibraryError::sqlite("all_cues_query", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| LibraryError::sqlite("all_cues_row", e))?);
        }
        Ok(out)
    }

    /// Every stored loop on a track, from every source.
    ///
    /// # Errors
    ///
    /// [`LibraryError::Sqlite`] if the query fails.
    pub fn all_loops(&self, track_id: &str) -> Result<Vec<StoredLoop>> {
        let conn = self.connection();
        let mut stmt = conn
            .prepare(
                "SELECT source, loop_index, in_secs, out_secs, name, color \
                 FROM track_loops WHERE track_id = ?1 ORDER BY source, loop_index",
            )
            .map_err(|e| LibraryError::sqlite("all_loops_prepare", e))?;
        let rows = stmt
            .query_map(params![track_id], |r| {
                Ok(StoredLoop {
                    source: r.get(0)?,
                    loop_index: r.get(1)?,
                    in_secs: r.get(2)?,
                    out_secs: r.get(3)?,
                    name: r.get(4)?,
                    color: r.get(5)?,
                })
            })
            .map_err(|e| LibraryError::sqlite("all_loops_query", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| LibraryError::sqlite("all_loops_row", e))?);
        }
        Ok(out)
    }

    /// Absolute path of a track's most-recently-confirmed file.
    ///
    /// Joins `track_files` to its volume's last known mount point,
    /// which is how every other path-consuming call site resolves one.
    /// `None` when the track has no file row.
    ///
    /// # Errors
    ///
    /// [`LibraryError::Sqlite`] if the query fails.
    pub fn track_path(&self, track_id: &str) -> Result<Option<std::path::PathBuf>> {
        let conn = self.connection();
        let found: rusqlite::Result<(String, String)> = conn.query_row(
            "SELECT v.last_known_mount_point, tf.relative_path \
             FROM track_files tf JOIN volumes v ON v.volume_uuid = tf.volume_uuid \
             WHERE tf.track_id = ?1 ORDER BY tf.last_seen_at DESC LIMIT 1",
            params![track_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        );
        match found {
            Ok((mount, rel)) => Ok(Some(std::path::Path::new(&mount).join(rel))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(LibraryError::sqlite("track_path", e)),
        }
    }
}
