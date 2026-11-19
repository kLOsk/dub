//! Which DJ app's analysis comes first, and which apps are on: Preferences
//! ▸ Library (2026-11-18, Daniel). One order answers whose grid, whose key
//! and whose hot cues a track gets when Serato, rekordbox and Traktor all
//! have an opinion — PRD §8.3's "per-genre priority is user-configurable",
//! for the library as a whole. An app switched off counts for nothing: its
//! grids, keys, cues and names stay stored (switching it back on is
//! instant) but never win, and the library columns leave them out — the
//! data read from inside the files included. Both settings live in the
//! library database (`library_settings`, schema v12), so the app and the
//! CLI agree.
//!
//! The DJ's own work in Dub still outranks every app: a set-the-1 grid, a
//! key set by hand, cues edited in Dub, a locked grid.

use rusqlite::{params, OptionalExtension};

use crate::db::Library;
use crate::error::{LibraryError, Result};

/// The apps whose order the DJ sets.
pub const IMPORT_SOURCES: [&str; 3] = ["serato", "rekordbox", "traktor"];

/// The apps the DJ can switch off: the three, and Apple Music.
pub const SWITCHABLE_SOURCES: [&str; 4] = ["serato", "rekordbox", "traktor", "itunes"];

/// PRD §8.3's default: Serato dominates pre-analysed hip-hop and reggae.
pub const DEFAULT_IMPORT_ORDER: [&str; 3] = IMPORT_SOURCES;

const ORDER_KEY: &str = "import_order";
/// Read by the track-row joins too (`db::TRACK_ROW_JOINS`).
pub(crate) const OFF_KEY: &str = "disabled_sources";

/// The order and the switched-off apps, read once per operation.
pub(crate) struct SourcePrefs {
    pub(crate) order: [&'static str; 3],
    off: Vec<&'static str>,
}

impl SourcePrefs {
    pub(crate) fn is_off(&self, source: &str) -> bool {
        self.off.contains(&source)
    }

    /// Where `source` stands among the apps: 2 for the first, 0 for the
    /// last, `None` for anything else.
    fn app_rank(&self, source: &str) -> Option<u8> {
        self.order
            .iter()
            .position(|o| *o == source)
            .and_then(|p| u8::try_from(2 - p).ok())
    }

    /// Beat-grid source priority for the single active slot, highest
    /// first; `None` for an app switched off, which never takes it. The
    /// three apps rank in the DJ's order (PRD §8.3) over `auto`; `user_tap`
    /// (the DJ's deliberate Dub correction) is preserved above imports
    /// across re-import (PRD §8.1), so it sits at the top.
    pub(crate) fn grid_rank(&self, source: &str) -> Option<u8> {
        if self.is_off(source) {
            return None;
        }
        Some(match source {
            "user_tap" => 5,
            "itunes" => 1,
            s => self.app_rank(s).map_or(0, |r| 2 + r), // auto → 0
        })
    }

    /// Key-notation source priority, the same way. `mixedinkey` (a
    /// dedicated key-detection authority DJs trust) and a `user` override
    /// rank above the apps. The file's own tag ranks above `auto`: a `TKEY`
    /// was written by whatever keyed the DJ's collection, and §8.3 puts
    /// every import ahead of Dub's estimate.
    pub(crate) fn key_rank(&self, source: &str) -> Option<u8> {
        if self.is_off(source) {
            return None;
        }
        Some(match source {
            "user" => 7,
            "mixedinkey" => 6,
            "itunes" => 2,
            "id3" => 1,
            s => self.app_rank(s).map_or(0, |r| 3 + r), // auto → 0
        })
    }

    /// The apps that are on, in order: whose hot cues fill the pads.
    pub(crate) fn cue_order(&self) -> Vec<&'static str> {
        self.order.into_iter().filter(|s| !self.is_off(s)).collect()
    }
}

impl Library {
    /// The order, first app first.
    pub fn import_order(&self) -> Result<[&'static str; 3]> {
        Ok(self
            .setting(ORDER_KEY)?
            .and_then(|s| parse_order(&s.split(',').collect::<Vec<_>>()))
            .unwrap_or(DEFAULT_IMPORT_ORDER))
    }

    /// The apps switched off.
    pub fn disabled_sources(&self) -> Result<Vec<&'static str>> {
        Ok(self
            .setting(OFF_KEY)?
            .map(|s| parse_off(&s.split(',').collect::<Vec<_>>()).unwrap_or_default())
            .unwrap_or_default())
    }

    pub(crate) fn source_prefs(&self) -> Result<SourcePrefs> {
        Ok(SourcePrefs {
            order: self.import_order()?,
            off: self.disabled_sources()?,
        })
    }

    /// Set the order and re-pick every track's grid and key by it. A track
    /// the DJ locked, set the 1 on or keyed by hand keeps what it has; a
    /// grid its app had locked brings that lock with it when it takes over.
    /// An order that isn't the three apps once each is refused.
    pub fn set_import_order(&self, order: &[String]) -> Result<()> {
        let names: Vec<&str> = order.iter().map(String::as_str).collect();
        let Some(order) = parse_order(&names) else {
            return Err(LibraryError::InvalidArgument(format!(
                "import order must name serato, rekordbox and traktor once each; got {names:?}"
            )));
        };
        self.store_and_repick(ORDER_KEY, &order.join(","))
    }

    /// Switch apps off (and every other one on), then re-pick. A track
    /// playing a switched-off app's grid moves to the next one — even a
    /// locked one, whose lock then follows the new grid's own.
    pub fn set_disabled_sources(&self, off: &[String]) -> Result<()> {
        let names: Vec<&str> = off.iter().map(String::as_str).collect();
        let Some(off) = parse_off(&names) else {
            return Err(LibraryError::InvalidArgument(format!(
                "only serato, rekordbox, traktor and itunes can be switched off; got {names:?}"
            )));
        };
        self.store_and_repick(OFF_KEY, &off.join(","))
    }

    pub(crate) fn setting(&self, key: &str) -> Result<Option<String>> {
        self.connection()
            .query_row(
                "SELECT value FROM library_settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("library_setting", e))
    }

    /// Store a setting that changes nothing already in the library.
    pub(crate) fn store_setting(&self, key: &str, value: &str) -> Result<()> {
        self.connection()
            .execute(
                "INSERT INTO library_settings (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map(|_| ())
            .map_err(|e| LibraryError::sqlite("library_setting_store", e))
    }

    fn store_and_repick(&self, key: &str, value: &str) -> Result<()> {
        let tx = self
            .connection()
            .unchecked_transaction()
            .map_err(|e| LibraryError::sqlite("library_setting_begin", e))?;
        tx.execute(
            "INSERT INTO library_settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| LibraryError::sqlite("library_setting_store", e))?;
        let prefs = self.source_prefs()?;
        self.repick(&prefs, Pick::Grid)?;
        self.repick(&prefs, Pick::Key)?;
        tx.commit()
            .map_err(|e| LibraryError::sqlite("library_setting_commit", e))
    }

    /// Make every track's best-ranked row the active one. Skipped: a grid
    /// the DJ set the 1 on (an inactive set-the-1 row, which a re-analysis
    /// retired, never comes back), and a locked grid — unless its app is
    /// switched off.
    fn repick(&self, prefs: &SourcePrefs, pick: Pick) -> Result<()> {
        let sql = match pick {
            Pick::Grid => {
                "SELECT r.track_id, r.source, r.is_active, r.source_locked, t.grid_locked \
                 FROM track_beatgrids r JOIN tracks t ON t.id = r.track_id \
                 WHERE r.source != 'user_tap' OR r.is_active = 1 \
                 ORDER BY r.track_id"
            }
            Pick::Key => {
                "SELECT track_id, source, is_active, 0, 0 FROM track_keys ORDER BY track_id"
            }
        };
        let rows: Vec<(String, String, bool, bool, bool)> = {
            let conn = self.connection();
            let mut stmt = conn
                .prepare(sql)
                .map_err(|e| LibraryError::sqlite("repick_select", e))?;
            let mapped = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get::<_, i64>(2)? != 0,
                        r.get::<_, i64>(3)? != 0,
                        r.get::<_, i64>(4)? != 0,
                    ))
                })
                .map_err(|e| LibraryError::sqlite("repick_query", e))?;
            mapped
                .collect::<std::result::Result<_, _>>()
                .map_err(|e| LibraryError::sqlite("repick_row", e))?
        };
        let rank = |s: &str| match pick {
            Pick::Grid => prefs.grid_rank(s),
            Pick::Key => prefs.key_rank(s),
        };
        let table = match pick {
            Pick::Grid => "track_beatgrids",
            Pick::Key => "track_keys",
        };
        for group in rows.chunk_by(|a, b| a.0 == b.0) {
            let track = &group[0].0;
            let active = group.iter().find(|r| r.2);
            let track_locked = group[0].4;
            let active_off = active.is_some_and(|r| prefs.is_off(&r.1));
            if track_locked && !active_off {
                continue;
            }
            let best = group
                .iter()
                .filter_map(|r| rank(&r.1).map(|k| (k, r)))
                .max_by_key(|(k, _)| *k)
                .map(|(_, r)| r);
            if best.map(|r| &r.1) == active.map(|r| &r.1) {
                continue;
            }
            let conn = self.connection();
            conn.execute(
                &format!("UPDATE {table} SET is_active = 0 WHERE track_id = ?1"),
                params![track],
            )
            .map_err(|e| LibraryError::sqlite("repick_clear", e))?;
            if let Some(best) = best {
                conn.execute(
                    &format!(
                        "UPDATE {table} SET is_active = 1 WHERE track_id = ?1 AND source = ?2"
                    ),
                    params![track, best.1],
                )
                .map_err(|e| LibraryError::sqlite("repick_set", e))?;
            }
            if matches!(pick, Pick::Grid) {
                let lock = best.is_some_and(|b| b.3);
                if lock || track_locked {
                    self.set_grid_locked(track, lock)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Pick {
    Grid,
    Key,
}

fn parse_order(names: &[&str]) -> Option<[&'static str; 3]> {
    if names.len() != 3 {
        return None;
    }
    let mut out = [""; 3];
    for (slot, name) in out.iter_mut().zip(names) {
        *slot = IMPORT_SOURCES.into_iter().find(|s| s == name)?;
    }
    let distinct = out[0] != out[1] && out[1] != out[2] && out[0] != out[2];
    distinct.then_some(out)
}

fn parse_off(names: &[&str]) -> Option<Vec<&'static str>> {
    let mut out = Vec::new();
    for name in names.iter().filter(|n| !n.is_empty()) {
        let s = SWITCHABLE_SOURCES.into_iter().find(|s| s == name)?;
        if !out.contains(&s) {
            out.push(s);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(lib: &Library, id: &str) {
        lib.insert_track(id, None, None, None).unwrap();
    }

    fn active_grid(lib: &Library, id: &str) -> String {
        lib.connection()
            .query_row(
                "SELECT source FROM track_beatgrids WHERE track_id = ?1 AND is_active = 1",
                params![id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn active_key(lib: &Library, id: &str) -> String {
        lib.connection()
            .query_row(
                "SELECT source FROM track_keys WHERE track_id = ?1 AND is_active = 1",
                params![id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn order(names: [&str; 3]) -> Vec<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn defaults_to_serato_first_and_refuses_a_bad_order() {
        let lib = Library::open_in_memory().unwrap();
        assert_eq!(lib.import_order().unwrap(), DEFAULT_IMPORT_ORDER);
        assert!(lib
            .set_import_order(&order(["serato", "serato", "traktor"]))
            .is_err());
        assert!(lib
            .set_import_order(&order(["serato", "itunes", "traktor"]))
            .is_err());
        assert!(lib.set_import_order(&["serato".to_string()]).is_err());
        assert_eq!(lib.import_order().unwrap(), DEFAULT_IMPORT_ORDER);
    }

    #[test]
    fn traktor_first_takes_grids_keys_and_its_locks_over() {
        let lib = Library::open_in_memory().unwrap();
        track(&lib, "both");
        lib.upsert_imported_beatgrid_locked("both", "serato", 0.5, 128.0, false)
            .unwrap();
        lib.upsert_imported_beatgrid_locked("both", "traktor", 0.5, 128.0, true)
            .unwrap();
        lib.upsert_imported_key("both", "serato", "8A", None)
            .unwrap();
        lib.upsert_imported_key("both", "traktor", "9A", None)
            .unwrap();
        assert_eq!(active_grid(&lib, "both"), "serato");
        assert!(!lib.is_grid_locked("both").unwrap());

        lib.set_import_order(&order(["traktor", "serato", "rekordbox"]))
            .unwrap();
        assert_eq!(
            lib.import_order().unwrap(),
            ["traktor", "serato", "rekordbox"]
        );
        assert_eq!(active_grid(&lib, "both"), "traktor");
        assert_eq!(active_key(&lib, "both"), "traktor");
        assert!(
            lib.is_grid_locked("both").unwrap(),
            "Traktor's lock comes along"
        );

        // New imports follow the order too.
        track(&lib, "new");
        lib.upsert_imported_beatgrid_locked("new", "traktor", 0.1, 90.0, false)
            .unwrap();
        lib.upsert_imported_beatgrid_locked("new", "serato", 0.1, 90.0, false)
            .unwrap();
        assert_eq!(active_grid(&lib, "new"), "traktor");
    }

    #[test]
    fn a_switched_off_app_counts_for_nothing_until_it_is_back_on() {
        let lib = Library::open_in_memory().unwrap();
        track(&lib, "t");
        lib.upsert_imported_beatgrid_locked("t", "serato", 0.5, 128.0, true)
            .unwrap();
        lib.upsert_imported_beatgrid_locked("t", "traktor", 0.4, 128.0, false)
            .unwrap();
        lib.upsert_imported_key("t", "serato", "8A", None).unwrap();
        lib.upsert_imported_key("t", "traktor", "9A", None).unwrap();
        lib.upsert_metadata_source(
            "t",
            "serato",
            None,
            Some("Serato's title"),
            None,
            None,
            Some("serato note"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(active_grid(&lib, "t"), "serato");
        assert!(lib.is_grid_locked("t").unwrap(), "Serato's lock");

        lib.set_disabled_sources(&["serato".to_string()]).unwrap();
        assert_eq!(lib.disabled_sources().unwrap(), vec!["serato"]);
        assert_eq!(active_grid(&lib, "t"), "traktor");
        assert_eq!(active_key(&lib, "t"), "traktor");
        assert!(
            !lib.is_grid_locked("t").unwrap(),
            "the lock was Serato's grid's; Traktor's isn't locked"
        );
        let row = lib.track_row_by_id("t").unwrap().unwrap();
        assert_eq!(row.title, None, "Serato's names drop out");
        assert_eq!(row.comment, None);

        // A Serato grid read while it's off stays out of the slot.
        track(&lib, "later");
        lib.upsert_imported_beatgrid_locked("later", "serato", 0.1, 90.0, false)
            .unwrap();
        lib.connection()
            .execute(
                "INSERT INTO track_beatgrids (track_id, source, anchor_secs, bpm, is_active, captured_at) \
                 VALUES ('later', 'auto', 0.1, 91.0, 1, 0)",
                [],
            )
            .unwrap();
        assert_eq!(active_grid(&lib, "later"), "auto");

        lib.set_disabled_sources(&[]).unwrap();
        assert_eq!(active_grid(&lib, "t"), "serato", "back on: back in front");
        assert!(lib.is_grid_locked("t").unwrap(), "with its lock");
        assert_eq!(active_grid(&lib, "later"), "serato");
        let row = lib.track_row_by_id("t").unwrap().unwrap();
        assert_eq!(row.title.as_deref(), Some("Serato's title"));
        assert!(lib.set_disabled_sources(&["auto".to_string()]).is_err());
    }

    #[test]
    fn the_djs_own_work_stays_put() {
        let lib = Library::open_in_memory().unwrap();
        track(&lib, "tapped");
        lib.upsert_imported_beatgrid_locked("tapped", "serato", 0.5, 128.0, false)
            .unwrap();
        lib.upsert_imported_beatgrid_locked("tapped", "traktor", 0.5, 128.0, false)
            .unwrap();
        lib.upsert_user_tap_beatgrid("tapped", 0.2, 127.5, 0)
            .unwrap();
        track(&lib, "locked");
        lib.upsert_imported_beatgrid_locked("locked", "serato", 0.5, 128.0, false)
            .unwrap();
        lib.upsert_imported_beatgrid_locked("locked", "traktor", 0.5, 128.0, false)
            .unwrap();
        lib.set_grid_locked("locked", true).unwrap();

        lib.set_import_order(&order(["traktor", "rekordbox", "serato"]))
            .unwrap();
        assert_eq!(active_grid(&lib, "tapped"), "user_tap");
        assert_eq!(active_grid(&lib, "locked"), "serato");
    }
}
