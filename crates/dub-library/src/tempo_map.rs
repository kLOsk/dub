//! A track's saved tempo map (schema v11): a beat list that is not an
//! even grid.
//!
//! The following beat grid (alpha) puts each beat where a band played
//! it. It used to be worked out on every load and thrown away, so the
//! library could not show it and a lock could not keep it (Daniel,
//! 2026-11-16: "if following beat grid is enabled the grid should be
//! saved to the track … when it's locked the lock should … tell it's
//! moving and accepted").
//!
//! A map remembers the even grid it was followed from (`base_*`). While
//! the track's active even grid is still that one, the map is reused;
//! when the grid changes (a tap on the BPM, a re-import) the map is
//! stale and the band is followed again. A locked track plays its map
//! regardless. `kind` is the general slot: `followed` today, a
//! time-stretched track's map later.

use rusqlite::{params, OptionalExtension};

use crate::db::Library;
use crate::error::{LibraryError, Result};

/// How far a base grid may differ and still be the same one: a rounding
/// of the stored REAL, not a different grid.
const BASE_BPM_TOLERANCE: f64 = 0.001;
const BASE_ANCHOR_TOLERANCE_SECS: f64 = 0.002;

/// A saved tempo map.
#[derive(Debug, Clone, PartialEq)]
pub struct TempoMap {
    /// `"followed"` — the only kind today.
    pub kind: String,
    /// Beat times in seconds from sample 0, ascending.
    pub beats: Vec<f64>,
    /// Index into `beats` of the first downbeat, mod the bar.
    pub bar_phase: u8,
    /// The even grid this map was followed from: its source, tempo and
    /// anchor, as `track_beatgrids` holds them.
    pub base_source: String,
    /// See [`Self::base_source`].
    pub base_bpm: f64,
    /// See [`Self::base_source`].
    pub base_anchor_secs: f64,
    /// The band's tempo range across the map.
    pub bpm_min: f64,
    /// See [`Self::bpm_min`].
    pub bpm_max: f64,
}

impl TempoMap {
    /// A followed map over `beats`, with its tempo range worked out.
    #[must_use]
    pub fn followed(
        beats: Vec<f64>,
        bar_phase: u8,
        base_source: &str,
        base_bpm: f64,
        base_anchor_secs: f64,
    ) -> Self {
        let (bpm_min, bpm_max) = tempo_range(&beats).unwrap_or((base_bpm, base_bpm));
        Self {
            kind: "followed".to_string(),
            beats,
            bar_phase,
            base_source: base_source.to_string(),
            base_bpm,
            base_anchor_secs,
            bpm_min,
            bpm_max,
        }
    }

    /// Whether this map was followed from the even grid `(source, bpm,
    /// anchor)` — i.e. is not stale against it.
    #[must_use]
    pub fn follows_base(&self, source: &str, bpm: f64, anchor_secs: f64) -> bool {
        self.base_source == source
            && (self.base_bpm - bpm).abs() <= BASE_BPM_TOLERANCE
            && (self.base_anchor_secs - anchor_secs).abs() <= BASE_ANCHOR_TOLERANCE_SECS
    }
}

/// The band's tempo range over `beats`: the tempo of each bar (four
/// beats), with the outer tenth at each end dropped so one flam or a
/// held fill does not set the range. `None` with under two bars.
#[must_use]
pub fn tempo_range(beats: &[f64]) -> Option<(f64, f64)> {
    const BAR: usize = 4;
    let mut bars: Vec<f64> = beats
        .windows(BAR + 1)
        .step_by(BAR)
        .filter_map(|w| {
            let span = w[BAR] - w[0];
            (span > 0.0).then(|| 60.0 * BAR as f64 / span)
        })
        .collect();
    if bars.len() < 2 {
        return None;
    }
    bars.sort_by(f64::total_cmp);
    let cut = bars.len() / 10;
    let kept = &bars[cut..bars.len() - cut];
    Some((*kept.first()?, *kept.last()?))
}

fn encode(beats: &[f64]) -> Vec<u8> {
    beats.iter().flat_map(|b| b.to_le_bytes()).collect()
}

fn decode(blob: &[u8]) -> Vec<f64> {
    blob.chunks_exact(8)
        .map(|c| f64::from_le_bytes(c.try_into().unwrap_or([0; 8])))
        .collect()
}

impl Library {
    /// The track's saved tempo map, if it has one.
    pub fn tempo_map(&self, track_id: &str) -> Result<Option<TempoMap>> {
        self.connection()
            .query_row(
                "SELECT kind, beats, bar_phase, base_source, base_bpm, base_anchor_secs, \
                        bpm_min, bpm_max \
                 FROM track_tempo_maps WHERE track_id = ?1",
                params![track_id],
                |r| {
                    let blob: Vec<u8> = r.get(1)?;
                    Ok(TempoMap {
                        kind: r.get(0)?,
                        beats: decode(&blob),
                        bar_phase: u8::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                        base_source: r.get(3)?,
                        base_bpm: r.get(4)?,
                        base_anchor_secs: r.get(5)?,
                        bpm_min: r.get(6)?,
                        bpm_max: r.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("tempo_map", e))
    }

    /// Save (or replace) the track's tempo map. A map with fewer than two
    /// beats, or a non-finite one, is not a map and is skipped.
    pub fn save_tempo_map(&self, track_id: &str, map: &TempoMap) -> Result<()> {
        if map.beats.len() < 2 || map.beats.iter().any(|b| !b.is_finite()) {
            return Ok(());
        }
        self.connection()
            .execute(
                "INSERT INTO track_tempo_maps \
                 (track_id, kind, beats, bar_phase, base_source, base_bpm, base_anchor_secs, \
                  bpm_min, bpm_max, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, strftime('%s','now')) \
                 ON CONFLICT(track_id) DO UPDATE SET \
                     kind = excluded.kind, beats = excluded.beats, \
                     bar_phase = excluded.bar_phase, base_source = excluded.base_source, \
                     base_bpm = excluded.base_bpm, base_anchor_secs = excluded.base_anchor_secs, \
                     bpm_min = excluded.bpm_min, bpm_max = excluded.bpm_max, \
                     created_at = excluded.created_at",
                params![
                    track_id,
                    map.kind,
                    encode(&map.beats),
                    i64::from(map.bar_phase),
                    map.base_source,
                    map.base_bpm,
                    map.base_anchor_secs,
                    map.bpm_min,
                    map.bpm_max,
                ],
            )
            .map_err(|e| LibraryError::sqlite("save_tempo_map", e))?;
        Ok(())
    }

    /// Forget the track's tempo map: its grid is even again.
    pub fn delete_tempo_map(&self, track_id: &str) -> Result<()> {
        self.connection()
            .execute(
                "DELETE FROM track_tempo_maps WHERE track_id = ?1",
                params![track_id],
            )
            .map_err(|e| LibraryError::sqlite("delete_tempo_map", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(lib: &Library, id: &str) {
        lib.connection()
            .execute(
                "INSERT INTO tracks (id, created_at, updated_at) VALUES (?1, 0, 0)",
                params![id],
            )
            .unwrap();
    }

    /// A band drifting from 90 to 96 BPM over 64 beats.
    fn drifting() -> Vec<f64> {
        let mut t = 0.0;
        (0..64)
            .map(|i| {
                let bpm = 90.0 + 6.0 * f64::from(i) / 63.0;
                let at = t;
                t += 60.0 / bpm;
                at
            })
            .collect()
    }

    #[test]
    fn a_saved_map_reads_back_exactly() {
        let lib = Library::open_in_memory().unwrap();
        seed(&lib, "t1");
        assert_eq!(lib.tempo_map("t1").unwrap(), None);
        let map = TempoMap::followed(drifting(), 1, "traktor", 93.0, 0.25);
        lib.save_tempo_map("t1", &map).unwrap();
        assert_eq!(lib.tempo_map("t1").unwrap(), Some(map.clone()));

        let moved = TempoMap::followed(drifting()[1..].to_vec(), 0, "user_tap", 93.0, 0.9);
        lib.save_tempo_map("t1", &moved).unwrap();
        assert_eq!(
            lib.tempo_map("t1").unwrap(),
            Some(moved),
            "one map per track, replaced"
        );

        lib.delete_tempo_map("t1").unwrap();
        assert_eq!(lib.tempo_map("t1").unwrap(), None);
    }

    #[test]
    fn the_range_is_the_bands_and_ignores_one_odd_bar() {
        let (lo, hi) = tempo_range(&drifting()).unwrap();
        assert!(lo > 89.5 && lo < 91.5, "lo {lo}");
        assert!(hi > 94.5 && hi < 96.5, "hi {hi}");
        assert_eq!(tempo_range(&[0.0, 0.5, 1.0]), None);
    }

    #[test]
    fn a_map_knows_the_grid_it_was_followed_from() {
        let map = TempoMap::followed(drifting(), 0, "auto", 93.0, 0.25);
        assert!(map.follows_base("auto", 93.0, 0.25));
        assert!(
            map.follows_base("auto", 93.0004, 0.2515),
            "a stored REAL's rounding"
        );
        assert!(
            !map.follows_base("user_tap", 93.0, 0.25),
            "a tap made a new grid"
        );
        assert!(!map.follows_base("auto", 93.0, 0.5), "the 1 moved");
        assert!(!map.follows_base("auto", 186.0, 0.25), "doubled");
    }

    #[test]
    fn the_browser_row_carries_the_map() {
        let lib = Library::open_in_memory().unwrap();
        seed(&lib, "t1");
        lib.save_tempo_map("t1", &TempoMap::followed(drifting(), 0, "auto", 93.0, 0.0))
            .unwrap();
        let kind: Option<String> = lib
            .connection()
            .query_row(
                "SELECT tm.kind FROM tracks t LEFT JOIN track_tempo_maps tm \
                 ON tm.track_id = t.id WHERE t.id = 't1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind.as_deref(), Some("followed"));
    }

    #[test]
    fn junk_is_not_saved() {
        let lib = Library::open_in_memory().unwrap();
        seed(&lib, "t1");
        lib.save_tempo_map("t1", &TempoMap::followed(vec![1.0], 0, "auto", 90.0, 0.0))
            .unwrap();
        lib.save_tempo_map(
            "t1",
            &TempoMap::followed(vec![0.0, f64::NAN], 0, "auto", 90.0, 0.0),
        )
        .unwrap();
        assert_eq!(lib.tempo_map("t1").unwrap(), None);
    }
}
