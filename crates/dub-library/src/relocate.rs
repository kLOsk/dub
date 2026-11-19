//! A library brought from another Mac still names that Mac's user folder:
//! an Apple Music XML from an account called `dj` says
//! `/Users/dj/Desktop/Musik/…`, while the same files on this Mac sit under
//! `/Users/klos/Desktop/Musik/…` (Daniel, 2026-11-18: 0 of 2,186 tracks found
//! as written, 2,184 with the user folder swapped). Every importer that reads
//! an absolute path asks here first. A path that exists is never touched, and
//! the swap is only taken when the file is really there, so nothing can be
//! pointed at the wrong song.

use std::path::{Component, Path, PathBuf};

const USERS: &str = "/Users";

/// `path` itself when the file is there; otherwise the same path inside this
/// Mac's user folder, when *that* file is there; otherwise `path` unchanged,
/// so the caller reports the path the library actually wrote.
pub(crate) fn relocate(path: &Path) -> PathBuf {
    match dirs::home_dir() {
        Some(home) => relocate_between(path, Path::new(USERS), &home),
        None => path.to_path_buf(),
    }
}

fn relocate_between(path: &Path, users: &Path, home: &Path) -> PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    let Ok(rest) = path.strip_prefix(users) else {
        return path.to_path_buf();
    };
    let mut parts = rest.components();
    if !matches!(parts.next(), Some(Component::Normal(_))) {
        return path.to_path_buf();
    }
    let inside = parts.as_path();
    if inside.as_os_str().is_empty() {
        return path.to_path_buf();
    }
    let candidate = home.join(inside);
    if candidate.is_file() {
        candidate
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mac {
        _dir: tempfile::TempDir,
        users: PathBuf,
        home: PathBuf,
    }

    fn mac() -> Mac {
        let dir = tempfile::tempdir().unwrap();
        let users = dir.path().join("Users");
        let home = users.join("klos");
        std::fs::create_dir_all(home.join("Desktop/Musik")).unwrap();
        std::fs::write(home.join("Desktop/Musik/a.mp3"), b"x").unwrap();
        Mac {
            _dir: dir,
            users,
            home,
        }
    }

    #[test]
    fn another_macs_user_folder_becomes_this_ones() {
        let m = mac();
        let written = m.users.join("dj/Desktop/Musik/a.mp3");
        assert_eq!(
            relocate_between(&written, &m.users, &m.home),
            m.home.join("Desktop/Musik/a.mp3")
        );
    }

    #[test]
    fn a_path_that_exists_is_left_alone() {
        let m = mac();
        let here = m.home.join("Desktop/Musik/a.mp3");
        assert_eq!(relocate_between(&here, &m.users, &m.home), here);
    }

    #[test]
    fn no_file_either_way_keeps_the_written_path() {
        let m = mac();
        let written = m.users.join("dj/Desktop/Musik/gone.mp3");
        assert_eq!(relocate_between(&written, &m.users, &m.home), written);
    }

    #[test]
    fn paths_outside_the_user_folders_are_not_guessed_at() {
        let m = mac();
        let drive = PathBuf::from("/Volumes/Crate/Desktop/Musik/a.mp3");
        assert_eq!(relocate_between(&drive, &m.users, &m.home), drive);
        let bare = m.users.join("dj");
        assert_eq!(relocate_between(&bare, &m.users, &m.home), bare);
    }
}
