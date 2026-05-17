//! Volume UUID discovery (PRD §8.2 path-by-volume-UUID).
//!
//! macOS volumes expose a stable UUID via the `volumeUUIDStringKey`
//! `NSURL` resource value, which underneath is `getattrlist(2)` with
//! `ATTR_VOL_UUID`. We call `getattrlist` directly to avoid pulling
//! in the entire Foundation bridge for one syscall.
//!
//! On non-macOS targets we surface
//! `LibraryError::VolumeUuidUnavailable` so the caller can degrade
//! cleanly; Dub is macOS-only in v1 (PRD §1) but the build
//! configuration is portable to keep the workspace `cargo check`-
//! clean on other developers' machines.

use std::path::Path;

#[cfg(not(target_os = "macos"))]
use crate::error::LibraryError;
use crate::error::Result;

/// Discovered volume identity at a moment in time. The `volume_uuid`
/// is the stable identifier; `mount_point` / `display_name` are the
/// **current** values, persisted into `volumes.last_known_mount_point`
/// and `volumes.display_name` for the missing-files / relocate
/// affordances (PRD §8.5.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredVolume {
    /// The UUID exposed by the filesystem. RFC 4122 canonical form
    /// on macOS (lowercase, hyphenated, 36 characters), per
    /// `man getattrlist`.
    pub volume_uuid: String,
    /// The mount-point path at the time of discovery.
    pub mount_point: std::path::PathBuf,
    /// User-facing volume name (`f_mntfromname` minus `/Volumes/`
    /// prefix where present, falling back to the mount-point's
    /// last component otherwise).
    pub display_name: String,
    /// True for the boot volume / internal drive. Drives the
    /// "external drive ejected = expected, internal drive missing
    /// = problem" UI discrimination in PRD §8.5.5.
    pub is_internal: bool,
}

impl DiscoveredVolume {
    /// Compute the path relative to this volume's mount point.
    /// Returns `None` if `path` is not under `mount_point`, which
    /// the caller (importer / FFI relocate path) treats as a
    /// registration failure. Used by both the importer and the
    /// M11d.4 Relocate panel to derive the `relative_path` column
    /// from a user-supplied absolute path.
    pub fn relative_to(&self, path: &Path) -> Option<String> {
        let stripped = path.strip_prefix(&self.mount_point).ok()?;
        Some(stripped.to_string_lossy().to_string())
    }
}

/// Discover the volume containing the given path. Returns a
/// `DiscoveredVolume` describing the volume's UUID and current
/// mount-point.
///
/// On non-macOS targets always returns
/// `LibraryError::VolumeUuidUnavailable` (Dub is macOS-only in v1;
/// PRD §1). Callers running on Linux for CI / dev convenience
/// should handle this error and skip file registration.
pub fn discover_for_path(path: &Path) -> Result<DiscoveredVolume> {
    #[cfg(target_os = "macos")]
    {
        macos::discover_for_path(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err(LibraryError::VolumeUuidUnavailable {
            path: path.to_path_buf(),
            reason: "volume UUID discovery is implemented for macOS only in v1",
        })
    }
}

#[cfg(target_os = "macos")]
mod macos {
    //! macOS-specific `getattrlist(2)` plumbing. The attribute list
    //! we request:
    //!
    //! ```c
    //! struct attrlist al = {
    //!     .bitmapcount = ATTR_BIT_MAP_COUNT,
    //!     .volattr = ATTR_VOL_INFO | ATTR_VOL_UUID,
    //! };
    //! ```
    //!
    //! `ATTR_VOL_INFO` is the mandatory leading bit for any volume
    //! attribute request; `ATTR_VOL_UUID` asks for a `uuid_t` (16
    //! raw bytes) in the response packet. We pair that with a
    //! `statfs(2)` call to read the mount-point + filesystem source
    //! for display-name purposes — `statfs` is one syscall, the
    //! same cost as querying `f_mntonname` via `getattrlist`.
    //!
    //! Note: the path must be queryable; on a network volume that
    //! doesn't expose UUIDs (rare; SMB shares without server
    //! support, some AFP edge cases), `getattrlist` returns a
    //! UUID of all zeros which we treat as "no UUID available"
    //! and surface as `VolumeUuidUnavailable`.

    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use crate::error::{LibraryError, Result};

    use super::DiscoveredVolume;

    const ATTR_BIT_MAP_COUNT: u16 = 5;
    const ATTR_VOL_INFO: u32 = 0x8000_0000;
    const ATTR_VOL_UUID: u32 = 0x0000_4000;

    // `getattrlist` writes a response packet whose first 4 bytes
    // are the packet length, followed by the requested attributes
    // in the order declared in `attrlist`. For our request that's
    // `[u32 length][uuid_t (16 bytes)]`.
    #[repr(C)]
    struct AttrList {
        bitmapcount: u16,
        reserved: u16,
        commonattr: u32,
        volattr: u32,
        dirattr: u32,
        fileattr: u32,
        forkattr: u32,
    }

    #[repr(C, packed)]
    struct UuidResponse {
        length: u32,
        uuid: [u8; 16],
    }

    extern "C" {
        fn getattrlist(
            path: *const libc::c_char,
            attrlist: *const AttrList,
            attribute_buffer: *mut libc::c_void,
            attribute_buffer_size: libc::size_t,
            options: u64,
        ) -> libc::c_int;
    }

    pub fn discover_for_path(path: &Path) -> Result<DiscoveredVolume> {
        let cstr = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            LibraryError::VolumeUuidUnavailable {
                path: path.to_path_buf(),
                reason: "path contains an interior NUL byte",
            }
        })?;

        let attrlist = AttrList {
            bitmapcount: ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: 0,
            volattr: ATTR_VOL_INFO | ATTR_VOL_UUID,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        };

        let mut response = UuidResponse {
            length: 0,
            uuid: [0; 16],
        };

        // `0` for options = follow symlinks; matches the semantics
        // we want (resolve through `/Users/klos/.../symlink`).
        let rc = unsafe {
            getattrlist(
                cstr.as_ptr(),
                &attrlist,
                &mut response as *mut UuidResponse as *mut libc::c_void,
                std::mem::size_of::<UuidResponse>(),
                0,
            )
        };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            return Err(LibraryError::Io {
                path: path.to_path_buf(),
                source: err,
            });
        }
        if response.uuid == [0; 16] {
            return Err(LibraryError::VolumeUuidUnavailable {
                path: path.to_path_buf(),
                reason: "filesystem returned a zero UUID (likely network volume \
                         without UUID support)",
            });
        }

        let raw_mount_point = statfs_mount_point(path)?;
        // macOS APFS firmlink reality: the boot volume's data
        // partition mounts at `/System/Volumes/Data`, but
        // user-visible paths like `/Users/...`, `/var/...`,
        // `/tmp/...` are firmlinked into `/` and don't include the
        // `/System/Volumes/Data` prefix. Treating that mount-point
        // string literally would break every path-relative-to-
        // volume computation. Normalise to `/` (the user-visible
        // boot-volume root) so the rest of the system Just Works.
        let (mount_point, is_internal) = if raw_mount_point == Path::new("/System/Volumes/Data")
            || raw_mount_point == Path::new("/")
        {
            (PathBuf::from("/"), true)
        } else {
            (raw_mount_point, false)
        };
        let display_name = derive_display_name(&mount_point);

        Ok(DiscoveredVolume {
            volume_uuid: uuid_bytes_to_string(response.uuid),
            mount_point,
            display_name,
            is_internal,
        })
    }

    /// Convert 16 raw UUID bytes into the canonical RFC 4122
    /// hyphenated lowercase string (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`).
    /// We deliberately do not pull in the `uuid` crate here just
    /// for formatting — this is a 30-line write-out and stays in
    /// lockstep with `man uuid` regardless of crate revisions.
    fn uuid_bytes_to_string(bytes: [u8; 16]) -> String {
        let mut s = String::with_capacity(36);
        for (i, byte) in bytes.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                s.push('-');
            }
            // SAFETY: hex of a u8 is always two ASCII chars.
            s.push_str(&format!("{byte:02x}"));
        }
        s
    }

    /// Reads the mount-point path that owns `path` via `statfs(2)`.
    fn statfs_mount_point(path: &Path) -> Result<PathBuf> {
        let cstr = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            LibraryError::VolumeUuidUnavailable {
                path: path.to_path_buf(),
                reason: "path contains an interior NUL byte",
            }
        })?;
        let mut info: libc::statfs = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::statfs(cstr.as_ptr(), &mut info) };
        if rc != 0 {
            return Err(LibraryError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::last_os_error(),
            });
        }
        // `f_mntonname` is a fixed-size `[c_char; MNAMELEN]` (typically
        // 1024 on macOS); read up to the NUL.
        let mnt = unsafe { std::ffi::CStr::from_ptr(info.f_mntonname.as_ptr()) };
        let bytes = mnt.to_bytes();
        // `to_path_buf` via `OsString::from_vec` keeps non-UTF-8
        // mount points working (rare but possible with FAT
        // mount points on USB sticks).
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
    }

    /// Derives a human display name for the volume. For
    /// `/Volumes/Touring SSD` we return `"Touring SSD"`; for `/`
    /// we return `"Macintosh HD"` matching Finder's convention.
    fn derive_display_name(mount_point: &Path) -> String {
        if mount_point == Path::new("/") {
            return "Macintosh HD".to_string();
        }
        mount_point
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| mount_point.to_string_lossy().to_string())
    }

    use std::os::unix::ffi::OsStringExt;

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn home_dir_resolves_to_some_uuid() {
            // The user's home directory must live on some volume
            // with a UUID. We don't assert which one (the test must
            // pass on every developer's machine) but we do require
            // the round-trip succeeds and the UUID is 36 chars.
            let home = dirs::home_dir().expect("home dir on macOS");
            let volume = discover_for_path(&home).expect("UUID resolvable for ~");
            assert_eq!(volume.volume_uuid.len(), 36, "RFC 4122 form");
            assert!(
                !volume.display_name.is_empty(),
                "volume display name must be non-empty"
            );
        }

        #[test]
        fn root_path_resolves_internal() {
            // `/` is the boot volume by definition on macOS.
            let v = discover_for_path(Path::new("/")).expect("UUID resolvable for /");
            assert!(v.is_internal, "/ must be marked internal");
            assert_eq!(v.mount_point, Path::new("/"));
            assert_eq!(v.display_name, "Macintosh HD");
        }

        #[test]
        fn uuid_round_trip_formatting() {
            // 0123456789abcdef-style bytes → canonical hex/hyphen string.
            let bytes = [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60,
                0x70, 0x80,
            ];
            assert_eq!(
                uuid_bytes_to_string(bytes),
                "01234567-89ab-cdef-1020-304050607080"
            );
        }
    }
}
