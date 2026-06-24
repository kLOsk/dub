//! Default-location discovery for the external DJ libraries Dub imports
//! (M11e / M12b / M12c).
//!
//! A target user already runs Serato / Traktor / iTunes, and each writes its
//! library to a well-known place. When the user enables a source in
//! Preferences, the app scans that location and imports it. This module owns
//! the path conventions (and the messy bits — `~` expansion, Traktor's
//! versioned folder) so the Apple side just toggles + imports what we report.

use std::path::PathBuf;

/// An external library source Dub can import from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Serato (`~/Music/_Serato_`).
    Serato,
    /// Traktor (`~/Documents/Native Instruments/Traktor */collection.nml`).
    Traktor,
    /// iTunes / Apple Music (`~/Music/iTunes/iTunes Library.xml`).
    Itunes,
}

impl SourceKind {
    /// The schema / FFI `source` tag.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Serato => "serato",
            SourceKind::Traktor => "traktor",
            SourceKind::Itunes => "itunes",
        }
    }
}

/// A discovered (or expected) default location for one source.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredSource {
    /// Which app this is.
    pub kind: SourceKind,
    /// The path the importer should be pointed at — the `_Serato_` folder,
    /// the `collection.nml`, or the iTunes `Library.xml`. When `exists` is
    /// false this is the *expected* default (useful for a "not found here"
    /// hint in Preferences).
    pub path: PathBuf,
    /// `true` when the path is actually present on disk right now.
    pub exists: bool,
}

/// Discover each source's default location under the current user's home.
/// Always returns one entry per [`SourceKind`] (with `exists` reflecting
/// reality), so the UI can render every source row whether or not the app is
/// installed.
pub fn discover_default_sources() -> Vec<DiscoveredSource> {
    let home = dirs::home_dir().unwrap_or_default();
    vec![
        discover_serato(&home),
        discover_traktor(&home),
        discover_itunes(&home),
    ]
}

/// Discover one source by kind (used by the importer after the user enables
/// it). `None` only if the home directory can't be resolved.
pub fn discover_source(kind: SourceKind) -> Option<DiscoveredSource> {
    let home = dirs::home_dir()?;
    Some(match kind {
        SourceKind::Serato => discover_serato(&home),
        SourceKind::Traktor => discover_traktor(&home),
        SourceKind::Itunes => discover_itunes(&home),
    })
}

fn discover_serato(home: &std::path::Path) -> DiscoveredSource {
    let dir = home.join("Music").join("_Serato_");
    // `database V2` is the file the importer needs; treat its presence as
    // "Serato is set up here".
    let exists = dir.join("database V2").is_file();
    DiscoveredSource {
        kind: SourceKind::Serato,
        path: dir,
        exists,
    }
}

fn discover_traktor(home: &std::path::Path) -> DiscoveredSource {
    let ni = home.join("Documents").join("Native Instruments");
    // Traktor versions its folder ("Traktor 3.11.1", "Traktor Pro 4", …);
    // pick the newest `Traktor*` dir that actually has a collection.nml.
    let best = std::fs::read_dir(&ni)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("Traktor"))
                && p.join("collection.nml").is_file()
        })
        .max(); // lexicographic max ≈ highest version
    match best {
        Some(dir) => DiscoveredSource {
            kind: SourceKind::Traktor,
            path: dir.join("collection.nml"),
            exists: true,
        },
        None => DiscoveredSource {
            kind: SourceKind::Traktor,
            path: ni.join("Traktor").join("collection.nml"),
            exists: false,
        },
    }
}

fn discover_itunes(home: &std::path::Path) -> DiscoveredSource {
    // The XML lives in `~/Music/iTunes/` under one of two names depending on
    // the iTunes version (`iTunes Music Library.xml` ≤ iTunes 11,
    // `iTunes Library.xml` iTunes 12+), or as an Apple Music "Share Library
    // XML" export. (Apple Music's own `~/Music/Music/Music Library.musiclibrary`
    // bundle is a binary format, not this plist — not importable here.)
    let candidates = [
        home.join("Music").join("iTunes").join("iTunes Library.xml"),
        home.join("Music")
            .join("iTunes")
            .join("iTunes Music Library.xml"),
        home.join("Music").join("Music").join("Library.xml"),
    ];
    for path in &candidates {
        if path.is_file() {
            return DiscoveredSource {
                kind: SourceKind::Itunes,
                path: path.clone(),
                exists: true,
            };
        }
    }
    DiscoveredSource {
        kind: SourceKind::Itunes,
        path: candidates.into_iter().next().unwrap_or_default(),
        exists: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_entry_per_kind() {
        let found = discover_default_sources();
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].kind, SourceKind::Serato);
        assert_eq!(found[1].kind, SourceKind::Traktor);
        assert_eq!(found[2].kind, SourceKind::Itunes);
        // Paths are always populated (default expected location when absent).
        assert!(found.iter().all(|s| !s.path.as_os_str().is_empty()));
    }

    #[test]
    fn source_tags_match_schema() {
        assert_eq!(SourceKind::Serato.as_str(), "serato");
        assert_eq!(SourceKind::Traktor.as_str(), "traktor");
        assert_eq!(SourceKind::Itunes.as_str(), "itunes");
    }
}
