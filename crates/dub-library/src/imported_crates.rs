//! Read-only mirror of an external library's crate / playlist tree
//! (`imported_crates` / `imported_crate_tracks`). Populated by the source
//! importers — Serato `*.crate` subcrates, Traktor `<PLAYLIST>` trees — and
//! never edited by the user (user-authored crates live in the separate
//! `crates` / `crate_tracks` tables; PRD §8.5.1). Re-import is
//! truncate-and-rewrite per source via [`Library::clear_imported_crates`].

use rusqlite::params;

use crate::db::Library;
use crate::error::{LibraryError, Result};

impl Library {
    /// Insert an imported crate/playlist node and return its row id.
    /// `parent` is `None` for a top-level node, else its parent's id
    /// (nested folders). Always inserts — re-import callers
    /// [`Self::clear_imported_crates`] the source first, so there is no
    /// upsert here.
    pub fn create_imported_crate(
        &self,
        source: &str,
        name: &str,
        parent: Option<i64>,
    ) -> Result<i64> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO imported_crates (source, name, parent_imported_crate_id, imported_at) \
             VALUES (?1, ?2, ?3, strftime('%s','now'))",
            params![source, name, parent],
        )
        .map_err(|e| LibraryError::sqlite("create_imported_crate", e))?;
        Ok(conn.last_insert_rowid())
    }

    /// Add a track to an imported crate at `ordinal` (the source's playlist
    /// order). Idempotent: a track already present is ignored (the
    /// `(imported_crate_id, track_id)` primary key), so a track listed twice
    /// in a playlist keeps its first position.
    pub fn add_track_to_imported_crate(
        &self,
        imported_crate_id: i64,
        track_id: &str,
        ordinal: i64,
    ) -> Result<()> {
        self.connection()
            .execute(
                "INSERT OR IGNORE INTO imported_crate_tracks \
                 (imported_crate_id, track_id, ordinal) VALUES (?1, ?2, ?3)",
                params![imported_crate_id, track_id, ordinal],
            )
            .map_err(|e| LibraryError::sqlite("add_track_to_imported_crate", e))?;
        Ok(())
    }

    /// Delete every imported crate for `source` (and, via
    /// `ON DELETE CASCADE`, its nested children and membership). Called at
    /// the start of a re-import so the mirror is rebuilt clean. Never
    /// touches the user `crates` tables.
    pub fn clear_imported_crates(&self, source: &str) -> Result<()> {
        self.connection()
            .execute(
                "DELETE FROM imported_crates WHERE source = ?1",
                params![source],
            )
            .map_err(|e| LibraryError::sqlite("clear_imported_crates", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Library;
    use rusqlite::params;

    fn seed_track(lib: &Library, id: &str) {
        lib.connection()
            .execute(
                "INSERT INTO tracks (id, created_at, updated_at) VALUES (?1, 0, 0)",
                params![id],
            )
            .unwrap();
    }

    fn crate_count(lib: &Library, source: &str) -> i64 {
        lib.connection()
            .query_row(
                "SELECT COUNT(*) FROM imported_crates WHERE source = ?1",
                params![source],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn member_count(lib: &Library) -> i64 {
        lib.connection()
            .query_row("SELECT COUNT(*) FROM imported_crate_tracks", [], |r| {
                r.get(0)
            })
            .unwrap()
    }

    #[test]
    fn nested_crates_members_and_truncate_rewrite() {
        let lib = Library::open_in_memory().unwrap();
        seed_track(&lib, "a");
        seed_track(&lib, "b");

        // Tree: "Hip Hop" > "90s", with two tracks in the child.
        let parent = lib
            .create_imported_crate("serato", "Hip Hop", None)
            .unwrap();
        let child = lib
            .create_imported_crate("serato", "90s", Some(parent))
            .unwrap();
        lib.add_track_to_imported_crate(child, "a", 0).unwrap();
        lib.add_track_to_imported_crate(child, "b", 1).unwrap();
        // A track listed twice keeps its first slot (PK ignore).
        lib.add_track_to_imported_crate(child, "a", 2).unwrap();

        assert_eq!(crate_count(&lib, "serato"), 2);
        assert_eq!(member_count(&lib), 2);

        // A different source coexists untouched.
        lib.create_imported_crate("traktor", "Dnb", None).unwrap();
        assert_eq!(crate_count(&lib, "traktor"), 1);

        // Re-import: clearing serato truncates its whole subtree and
        // cascades the membership away; traktor is left alone.
        lib.clear_imported_crates("serato").unwrap();
        assert_eq!(crate_count(&lib, "serato"), 0);
        assert_eq!(member_count(&lib), 0);
        assert_eq!(crate_count(&lib, "traktor"), 1);
    }
}
