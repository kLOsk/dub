//! Known-device routing profiles (M5.5.2).
//!
//! Maps recognised audio interfaces to their canonical per-deck output
//! channel layout. Since the audio device classifier landed
//! (`dub_audio::devices`), the routing table is no longer hardcoded here —
//! it lives in `crates/dub-audio/devices.toml` and is parsed once at
//! startup by `DeviceRegistry::embedded`. This module is now a thin
//! CLI-facing facade over that registry.
//!
//! ## Why a table, not a formula
//!
//! Earlier drafts assumed a simple `2N+1, 2N+2` formula (deck 0 →
//! ch 1+2, deck 1 → ch 3+4). That's wrong for the SL3, which wires
//! aux to ch 1+2 and decks to ch 3+4 / 5+6 internally. A formula
//! that's wrong for our reference device is worse than no formula —
//! the CLI would silently send deck audio to the wrong physical
//! pairs. So we keep an explicit table and require unknown devices
//! to opt in via manual flags or a registry PR.
//!
//! ## Adding a new device
//!
//! Edit `crates/dub-audio/devices.toml`. The file documents its own
//! schema and the steps to validate a new entry.

use dub_audio::{DeviceRegistry, KnownInterface};

/// Match a device name against the embedded registry and return the
/// first profile whose `name_pattern` is a case-insensitive substring
/// of `device_name`. Returns `None` for unknown devices.
///
/// Wraps [`DeviceRegistry::match_interface`] so the CLI keeps its
/// previous call-shape.
#[must_use]
pub fn match_device(device_name: &str) -> Option<&'static KnownInterface> {
    DeviceRegistry::embedded().match_interface(device_name)
}

/// Look up a profile by `name_pattern` (case-insensitive exact match,
/// used by the `--device-profile <name>` CLI override). Returns `None`
/// if no profile has that exact pattern.
///
/// Wraps [`DeviceRegistry::interface_by_pattern`].
#[must_use]
pub fn profile_by_pattern(pattern: &str) -> Option<&'static KnownInterface> {
    DeviceRegistry::embedded().interface_by_pattern(pattern)
}

/// Slice of every interface in the embedded registry. Equivalent to
/// the pre-registry `KNOWN_DEVICES` static; consumers that need to
/// iterate (e.g. the `--device-profile` error message) use this.
#[must_use]
pub fn known_devices() -> &'static [KnownInterface] {
    &DeviceRegistry::embedded().interfaces
}

/// Convert a 1-based CLI channel argument to a 0-based first-channel
/// for [`dub_engine::OutputRouting`]. `1` → `0`, `3` → `2`, etc.
/// Returns `None` if the input is `0` (1-based has no channel 0) or
/// if the channel is so large that subtracting 1 would overflow.
///
/// Channel pairs are encoded as the *first* (lower-numbered) channel
/// of the pair: passing `3` here puts deck audio on physical
/// channels 3+4 (1-based) = 2+3 (0-based, what the engine wants).
#[must_use]
pub fn one_based_to_zero_based(ch_1_based: u32) -> Option<u32> {
    ch_1_based.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sl3_matches_with_full_name() {
        let d = match_device("SL 3").expect("SL 3 should match");
        assert_eq!(d.display_name, "Serato SL 3");
        assert_eq!(d.output_channels, 6);
        // 1-based in the registry — convert at the engine boundary.
        assert_eq!(d.deck_a_zero_based(), 2);
        assert_eq!(d.deck_b_zero_based(), 4);
        assert!(d.verified);
    }

    #[test]
    fn sl3_matches_lowercase() {
        // CoreAudio sometimes returns "Rane SL 3" or "Scratch Live SL 3"
        // depending on driver version. Substring + case-insensitive
        // means all of these match the same profile.
        let cases = ["sl 3", "Rane SL 3", "Scratch Live SL 3", "SL 3 (4-Out)"];
        for case in cases {
            assert!(match_device(case).is_some(), "{case} did not match SL 3");
        }
    }

    #[test]
    fn audio6_matches_and_is_unverified() {
        let d = match_device("Traktor Audio 6").expect("should match");
        assert_eq!(d.display_name, "Traktor Audio 6");
        assert!(
            !d.verified,
            "Audio 6 routing is a best-effort guess; should be marked unverified \
             until validated against real hardware"
        );
        assert_eq!(d.deck_a_zero_based(), 0);
        assert_eq!(d.deck_b_zero_based(), 2);
    }

    #[test]
    fn unknown_device_returns_none() {
        assert!(match_device("MacBook Pro Speakers").is_none());
        assert!(match_device("AirPods Pro").is_none());
        assert!(match_device("Some Random USB DAC").is_none());
        assert!(match_device("").is_none());
    }

    #[test]
    fn profile_by_pattern_exact_match() {
        assert_eq!(
            profile_by_pattern("SL 3").unwrap().display_name,
            "Serato SL 3"
        );
        assert_eq!(
            profile_by_pattern("audio 6").unwrap().display_name,
            "Traktor Audio 6"
        );
        assert!(profile_by_pattern("xxxxx").is_none());
    }

    #[test]
    fn one_based_zero_based_conversion() {
        assert_eq!(one_based_to_zero_based(1), Some(0));
        assert_eq!(one_based_to_zero_based(3), Some(2));
        assert_eq!(one_based_to_zero_based(5), Some(4));
        assert_eq!(one_based_to_zero_based(0), None);
    }

    #[test]
    fn deck_routing_pairs_are_disjoint() {
        // A profile that puts deck A and deck B on overlapping
        // channels is almost always a bug. Pin the property: every
        // entry has deck A and deck B on non-overlapping pairs.
        for d in known_devices() {
            let a = d.deck_a_zero_based();
            let b = d.deck_b_zero_based();
            let a_pair = a..a + 2;
            let b_pair = b..b + 2;
            assert!(
                a_pair.end <= b_pair.start || b_pair.end <= a_pair.start,
                "device {} has deck A on ch {}..{} and deck B on ch {}..{} — pairs overlap",
                d.display_name,
                a_pair.start,
                a_pair.end,
                b_pair.start,
                b_pair.end,
            );
        }
    }

    #[test]
    fn deck_pairs_fit_in_output_channels() {
        for d in known_devices() {
            assert!(
                d.deck_a_zero_based() + 2 <= d.output_channels,
                "{}: deck A channel {} + 2 > output_channels {}",
                d.display_name,
                d.deck_a_zero_based(),
                d.output_channels
            );
            assert!(
                d.deck_b_zero_based() + 2 <= d.output_channels,
                "{}: deck B channel {} + 2 > output_channels {}",
                d.display_name,
                d.deck_b_zero_based(),
                d.output_channels
            );
        }
    }
}
