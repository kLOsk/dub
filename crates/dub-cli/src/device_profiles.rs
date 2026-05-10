//! Known-device routing profiles (M5.5.2).
//!
//! Maps recognised audio interfaces to their canonical per-deck output
//! channel layout. The matching is case-insensitive substring against
//! the CoreAudio device name as returned by `dub_audio::query_default_output`,
//! so users don't have to type the exact device name.
//!
//! ## Why a table, not a formula
//!
//! Earlier drafts assumed a simple `2N+1, 2N+2` formula (deck 0 →
//! ch 1+2, deck 1 → ch 3+4). That's wrong for the SL3, which wires
//! aux to ch 1+2 and decks to ch 3+4 / 5+6 internally. A formula
//! that's wrong for our reference device is worse than no formula —
//! the CLI would silently send deck audio to the wrong physical
//! pairs. So we keep a small explicit table and require unknown
//! devices to opt in via manual flags.
//!
//! ## Adding a new device
//!
//! 1. Plug the device into a Mac; run `dub measure-latency` and copy
//!    the `name` field exactly. Use a substring that's unique in the
//!    set of devices a user is likely to have.
//! 2. Find the device's per-deck channel mapping in its manual or
//!    driver panel (Native Instruments and Serato both publish this).
//!    Channels in the profile are **0-based** to match
//!    `dub_engine::OutputRouting`. CLI flags are 1-based for user
//!    ergonomics — see `parse_one_based_channel`.
//! 3. Add a new `KnownDevice` entry to [`KNOWN_DEVICES`].
//! 4. Add a unit test — at minimum, that `match_device(name)` finds
//!    the new entry and that `routing_for_device` returns the expected
//!    `OutputRouting`.

/// Per-deck output routing profile for a recognised audio interface.
///
/// All channel indices are 0-based — they go directly into
/// [`dub_engine::OutputRouting`]. Convert from the user-facing
/// 1-based CLI flags via [`one_based_to_zero_based`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownDevice {
    /// Case-insensitive substring matched against the CoreAudio
    /// device name. The first entry that matches wins.
    pub name_pattern: &'static str,
    /// Human-readable display name (used in CLI output).
    pub display_name: &'static str,
    /// Total physical output channels we open the AU with. The SL3
    /// has 6; opening only 4 would silently drop the deck-B → ch 5+6
    /// route.
    pub output_channels: u32,
    /// First (0-based) output channel for deck A's stereo pair.
    pub deck_a_first_channel: u32,
    /// First (0-based) output channel for deck B's stereo pair.
    pub deck_b_first_channel: u32,
    /// Whether this profile has been physically validated by us.
    /// Unverified entries are best-effort guesses based on
    /// driver-panel docs; we still apply them but warn in the CLI
    /// output that the user should double-check the routing the
    /// first time they spin a deck.
    pub verified: bool,
}

/// Static table of known devices. Order matters when patterns overlap
/// — the first match wins, so put more-specific patterns earlier.
///
/// Verified devices (validated end-to-end against real hardware):
///
/// - **SL 3** (Serato Scratch Live): aux ch 1+2, deck A out ch 3+4,
///   deck B out ch 5+6. Mirrors the SL3's input wiring (which we
///   already use for timecode in M5.2 — input deck A is also ch 3+4).
///   The wiring is internal to the box: ch 3+4 of the SL3 outputs
///   physically connect to the "Deck A → Mixer" RCAs on the back.
///
/// Unverified (driver-panel guesses, marked `verified: false`):
///
/// - **Traktor Audio 6**: deck A out ch 1+2, deck B out ch 3+4 (per
///   user recollection; will validate when a unit is on hand).
pub const KNOWN_DEVICES: &[KnownDevice] = &[
    KnownDevice {
        name_pattern: "SL 3",
        display_name: "Serato SL 3",
        output_channels: 6,
        deck_a_first_channel: 2, // 1-based ch 3 → 0-based 2
        deck_b_first_channel: 4, // 1-based ch 5 → 0-based 4
        verified: true,
    },
    KnownDevice {
        name_pattern: "Audio 6",
        display_name: "Traktor Audio 6",
        output_channels: 6,
        deck_a_first_channel: 0, // 1-based ch 1 → 0-based 0
        deck_b_first_channel: 2, // 1-based ch 3 → 0-based 2
        verified: false,
    },
];

/// Match a device name (from `query_default_output`) against
/// [`KNOWN_DEVICES`] and return the first profile whose
/// `name_pattern` is found (case-insensitive) as a substring of the
/// device name. Returns `None` for unknown devices.
#[must_use]
pub fn match_device(device_name: &str) -> Option<&'static KnownDevice> {
    let haystack = device_name.to_ascii_lowercase();
    KNOWN_DEVICES
        .iter()
        .find(|d| haystack.contains(&d.name_pattern.to_ascii_lowercase()))
}

/// Look up a profile by `name_pattern` (case-insensitive exact match
/// — used by the `--device-profile <name>` CLI override). Returns
/// `None` if no profile has that exact pattern.
#[must_use]
pub fn profile_by_pattern(pattern: &str) -> Option<&'static KnownDevice> {
    KNOWN_DEVICES
        .iter()
        .find(|d| d.name_pattern.eq_ignore_ascii_case(pattern))
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
        assert_eq!(d.deck_a_first_channel, 2);
        assert_eq!(d.deck_b_first_channel, 4);
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
        assert_eq!(d.deck_a_first_channel, 0);
        assert_eq!(d.deck_b_first_channel, 2);
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
        assert_eq!(one_based_to_zero_based(0), None); // no channel 0 in 1-based
    }

    #[test]
    fn deck_routing_pairs_are_disjoint() {
        // A profile that puts deck A and deck B on overlapping
        // channels is almost always a bug. Pin the property: every
        // verified profile has deck A and deck B on non-overlapping
        // pairs (each pair takes 2 channels).
        for d in KNOWN_DEVICES {
            let a = d.deck_a_first_channel;
            let b = d.deck_b_first_channel;
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
        for d in KNOWN_DEVICES {
            assert!(
                d.deck_a_first_channel + 2 <= d.output_channels,
                "{}: deck A channel {} + 2 > output_channels {}",
                d.display_name,
                d.deck_a_first_channel,
                d.output_channels
            );
            assert!(
                d.deck_b_first_channel + 2 <= d.output_channels,
                "{}: deck B channel {} + 2 > output_channels {}",
                d.display_name,
                d.deck_b_first_channel,
                d.output_channels
            );
        }
    }
}
