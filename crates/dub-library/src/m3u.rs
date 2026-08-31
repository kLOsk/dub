//! M3U8 playlist export (M11f).
//!
//! The universal format, and deliberately the lossy one: a playlist is
//! file paths and nothing else. No grid, no cues, no key, no colour —
//! anything Dub knows beyond "these files, in this order" is dropped.
//! That is not a shortcoming to fix, it is what M3U is; the rekordbox
//! XML exporter is where fidelity lives (PRD §8.6), and this is for
//! everything that only speaks paths.
//!
//! **M3U8, not M3U** — the `8` is UTF-8, which a record collection
//! needs. A plain `.m3u` is nominally Latin-1 and mangles the first
//! non-ASCII artist name it meets.

use std::io::Write;

/// One playlist entry.
#[derive(Debug, Clone, PartialEq)]
pub struct M3uEntry {
    /// Absolute path to the audio file.
    pub path: std::path::PathBuf,
    /// Display artist, if known.
    pub artist: Option<String>,
    /// Display title, if known.
    pub title: Option<String>,
    /// Length in seconds; `-1` is written when unknown, which is the
    /// convention players expect.
    pub duration_secs: Option<f64>,
}

/// Write an extended M3U8 playlist.
///
/// Absolute paths, because a Dub library spans volumes and a relative
/// path would only be meaningful from one directory.
///
/// # Errors
///
/// Propagates any write failure.
pub fn write_m3u8<W: Write>(entries: &[M3uEntry], mut sink: W) -> std::io::Result<()> {
    writeln!(sink, "#EXTM3U")?;
    for e in entries {
        let secs = e.duration_secs.filter(|s| s.is_finite() && *s > 0.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let whole = secs.map_or(-1_i64, |s| s.round() as i64);
        writeln!(sink, "#EXTINF:{whole},{}", display_name(e))?;
        writeln!(sink, "{}", e.path.display())?;
    }
    Ok(())
}

/// `Artist - Title`, degrading to whichever half exists, and finally to
/// the file name — a playlist entry with a blank label is useless in a
/// player's list even though the path still resolves.
fn display_name(e: &M3uEntry) -> String {
    match (e.artist.as_deref(), e.title.as_deref()) {
        (Some(a), Some(t)) => format!("{a} - {t}"),
        (None, Some(t)) => t.to_string(),
        (Some(a), None) => a.to_string(),
        (None, None) => e
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unknown".to_string()),
    }
}

/// Build entries from library rows, skipping tracks with no file.
///
/// # Errors
///
/// Propagates any library read failure.
pub fn entries_from_tracks(
    library: &crate::Library,
    rows: &[crate::db::TrackRow],
) -> crate::error::Result<Vec<M3uEntry>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        // A path is the entire content of an M3U line; a row we cannot
        // point at has nothing to contribute.
        let Some(path) = library.track_path(&row.id)? else {
            continue;
        };
        out.push(M3uEntry {
            path,
            artist: row.artist.clone(),
            title: row.title.clone(),
            duration_secs: (row.duration_ms > 0).then(|| f64::from(row.duration_ms) / 1000.0),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn entry(path: &str, artist: Option<&str>, title: Option<&str>, secs: Option<f64>) -> M3uEntry {
        M3uEntry {
            path: Path::new(path).to_path_buf(),
            artist: artist.map(str::to_string),
            title: title.map(str::to_string),
            duration_secs: secs,
        }
    }

    fn render(entries: &[M3uEntry]) -> String {
        let mut buf = Vec::new();
        write_m3u8(entries, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn writes_the_extended_header_and_one_pair_of_lines_per_track() {
        let out = render(&[entry(
            "/Users/dj/a.flac",
            Some("Sound Dimension"),
            Some("Real Rock"),
            Some(212.4),
        )]);
        assert_eq!(
            out,
            "#EXTM3U\n#EXTINF:212,Sound Dimension - Real Rock\n/Users/dj/a.flac\n"
        );
    }

    /// `-1` is the convention for "length unknown"; writing `0` would
    /// make players show a zero-length track.
    #[test]
    fn an_unknown_duration_is_minus_one() {
        let out = render(&[entry("/a.flac", Some("X"), Some("Y"), None)]);
        assert!(out.contains("#EXTINF:-1,X - Y"), "{out}");
    }

    #[test]
    fn a_nameless_track_falls_back_to_its_file_name() {
        let out = render(&[entry("/Users/dj/Untitled Take 3.flac", None, None, None)]);
        assert!(out.contains("#EXTINF:-1,Untitled Take 3"), "{out}");
    }

    #[test]
    fn half_named_tracks_use_the_half_they_have() {
        let out = render(&[
            entry("/a.flac", None, Some("Just A Title"), None),
            entry("/b.flac", Some("Just An Artist"), None, None),
        ]);
        assert!(out.contains(",Just A Title\n"), "{out}");
        assert!(out.contains(",Just An Artist\n"), "{out}");
    }

    /// The reason the format is M3U8 rather than M3U.
    #[test]
    fn unicode_survives() {
        let out = render(&[entry(
            "/Users/dj/Café/ソウル.flac",
            Some("MFSB"),
            Some("ソウル･トレインのテーマ"),
            Some(180.0),
        )]);
        assert!(out.contains("ソウル･トレインのテーマ"), "{out}");
        assert!(out.contains("/Users/dj/Café/ソウル.flac"), "{out}");
    }

    #[test]
    fn an_empty_playlist_is_still_a_valid_file() {
        assert_eq!(render(&[]), "#EXTM3U\n");
    }
}
