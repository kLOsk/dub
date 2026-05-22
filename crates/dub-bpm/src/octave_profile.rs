//! Genre-aware octave disambiguation (M11c.3d+).
//!
//! Offline library analysis can pass an [`OctaveProfile`] derived from
//! ID3 genre tags. Thru-mode streaming keeps [`OctaveProfile::Default`]
//! because live wax has no tag until fingerprint match (v1.1).

/// How pass 2 resolves octave / subdivision ambiguity when metadata
/// supplies genre context. [`Default`] is the mixable-band prior from
/// M11c.3a.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OctaveProfile {
    /// Hip-hop / DnB / untagged — existing perceptual prior.
    #[default]
    Default,
    /// Roots reggae, rocksteady: prefer one-drop / skank root (~65–95).
    RootsReggae,
    /// Dub, dubstep (half-time feel): prefer ~65–75 when tied.
    Dub,
    /// Fast dancehall / ragga: keep full-tempo detections (e.g. ~132).
    Dancehall,
    /// House, garage, techno: prefer 4/4 kick grid (~120–140).
    FourOnFloor,
}

/// Map a container genre string to an [`OctaveProfile`].
///
/// Matching is case-insensitive and substring-based so tags like
/// `"Reggae / Dub"` or `"UK Garage"` resolve sensibly.
#[must_use]
pub fn octave_profile_from_genre(genre: &str) -> OctaveProfile {
    let g = genre.trim().to_ascii_lowercase();
    if g.is_empty() {
        return OctaveProfile::Default;
    }
    if g.contains("dancehall") || g.contains("ragga") {
        return OctaveProfile::Dancehall;
    }
    if g.contains("dub") && !g.contains("dubstep") {
        return OctaveProfile::Dub;
    }
    if g.contains("dubstep") {
        return OctaveProfile::Dub;
    }
    if g.contains("reggae")
        || g.contains("rocksteady")
        || g.contains("roots")
        || g.contains("lovers")
        || g.contains("ska")
    {
        return OctaveProfile::RootsReggae;
    }
    if g.contains("house")
        || g.contains("garage")
        || g.contains("techno")
        || g.contains("trance")
        || g.contains("electro")
        || g.contains("club")
    {
        return OctaveProfile::FourOnFloor;
    }
    OctaveProfile::Default
}

/// Parse a manifest profile label (`roots`, `dub`, `house`, …).
#[must_use]
pub fn octave_profile_from_label(label: &str) -> OctaveProfile {
    match label.trim().to_ascii_lowercase().as_str() {
        "roots" | "roots_reggae" | "reggae" => OctaveProfile::RootsReggae,
        "dub" => OctaveProfile::Dub,
        "dancehall" | "ragga" => OctaveProfile::Dancehall,
        "house" | "four_on_floor" | "garage" | "techno" => OctaveProfile::FourOnFloor,
        _ => OctaveProfile::Default,
    }
}

/// Reggae skank pass false-flips house kicks at ~129 BPM (M11c.3f).
#[must_use]
pub fn profile_skips_skank_pass(profile: OctaveProfile) -> bool {
    matches!(profile, OctaveProfile::FourOnFloor)
}

/// Upper BPM band for profile-driven double-time rejection.
const PROFILE_HIGH_BPM_MIN: f64 = 135.0;
const PROFILE_HIGH_BPM_MAX: f64 = 180.0;

/// Lower BPM band paired with the profile high band (2:1 ratio).
const PROFILE_LOW_BPM_MIN: f64 = 60.0;
const PROFILE_LOW_BPM_MAX: f64 = 100.0;

const PROFILE_OCTAVE_RATIO: f64 = 2.0;
const PROFILE_OCTAVE_TOLERANCE: f64 = 0.04;

const ROOTS_SIBLING_MIN_RAW_RATIO: f64 = 0.75;
const DUB_NEAR_TIE_MAX_GAP: f64 = 0.02;
const DUB_SIBLING_MIN_RAW_RATIO: f64 = 0.80;

/// 4/4 house: false half-bar candidates in this band.
const FOF_HALF_BAR_LOW_MIN: f64 = 80.0;
const FOF_HALF_BAR_LOW_MAX: f64 = 100.0;
const FOF_HALF_BAR_HIGH_MIN: f64 = 115.0;
const FOF_HALF_BAR_HIGH_MAX: f64 = 145.0;
const FOF_HALF_BAR_RATIO: f64 = 1.5;
const FOF_HALF_BAR_MIN_RAW_RATIO: f64 = 0.85;

/// Shuffle-feel phantom peaks ~4/3 above the true house tempo.
const FOF_SHUFFLE_HIGH_MIN: f64 = 152.0;
const FOF_SHUFFLE_HIGH_MAX: f64 = 170.0;
const FOF_SHUFFLE_LOW_MIN: f64 = 118.0;
const FOF_SHUFFLE_LOW_MAX: f64 = 130.0;
const FOF_SHUFFLE_RATIO: f64 = 4.0 / 3.0;
const FOF_SHUFFLE_MIN_RAW_RATIO: f64 = 0.85;

/// Dubstep mid-band false peaks (~93 vs true ~70).
const DUB_MID_LOW_MIN: f64 = 85.0;
const DUB_MID_LOW_MAX: f64 = 100.0;
const DUB_MID_ROOT_MIN: f64 = 65.0;
const DUB_MID_ROOT_MAX: f64 = 74.0;
const DUB_MID_RATIO: f64 = 4.0 / 3.0;
const DUB_MID_MIN_RAW_RATIO: f64 = 0.80;

fn ratio_matches(actual: f64, target: f64, tolerance: f64) -> bool {
    (actual - target).abs() <= tolerance * target
}

/// Returns `true` when a qualifying upper-octave candidate should be
/// discarded because genre context says the lower octave is the mix
/// tempo (M11c.3d).
pub(crate) fn profile_doubletime_rejected(
    profile: OctaveProfile,
    candidate_bpm: f64,
    candidate_raw: f64,
    qualified: &[(f64, f64)],
) -> bool {
    if !matches!(profile, OctaveProfile::RootsReggae | OctaveProfile::Dub) {
        return false;
    }

    if !(PROFILE_HIGH_BPM_MIN..=PROFILE_HIGH_BPM_MAX).contains(&candidate_bpm) {
        return false;
    }

    for &(other_bpm, other_raw) in qualified {
        if !(PROFILE_LOW_BPM_MIN..=PROFILE_LOW_BPM_MAX).contains(&other_bpm) {
            continue;
        }
        let ratio = candidate_bpm / other_bpm;
        if (ratio - PROFILE_OCTAVE_RATIO).abs() > PROFILE_OCTAVE_TOLERANCE * PROFILE_OCTAVE_RATIO {
            continue;
        }

        let min_ratio = match profile {
            OctaveProfile::RootsReggae => ROOTS_SIBLING_MIN_RAW_RATIO,
            OctaveProfile::Dub => {
                let raw_gap = (candidate_raw - other_raw).abs() / candidate_raw.max(other_raw);
                if raw_gap <= DUB_NEAR_TIE_MAX_GAP {
                    return true;
                }
                DUB_SIBLING_MIN_RAW_RATIO
            }
            OctaveProfile::Default | OctaveProfile::Dancehall | OctaveProfile::FourOnFloor => {
                continue;
            }
        };

        if other_raw >= candidate_raw * min_ratio {
            return true;
        }
    }
    false
}

/// Profile-specific pass-2 rejections beyond the global rules.
pub(crate) fn profile_subdivision_rejected(
    profile: OctaveProfile,
    candidate_bpm: f64,
    candidate_raw: f64,
    qualified: &[(f64, f64)],
) -> bool {
    match profile {
        OctaveProfile::FourOnFloor => {
            four_on_floor_halfbar_rejected(candidate_bpm, candidate_raw, qualified)
                || four_on_floor_shuffle_high_rejected(candidate_bpm, candidate_raw, qualified)
        }
        OctaveProfile::Dub => dub_midband_rejected(candidate_bpm, candidate_raw, qualified),
        _ => false,
    }
}

fn four_on_floor_halfbar_rejected(
    candidate_bpm: f64,
    candidate_raw: f64,
    qualified: &[(f64, f64)],
) -> bool {
    if !(FOF_HALF_BAR_LOW_MIN..=FOF_HALF_BAR_LOW_MAX).contains(&candidate_bpm) {
        return false;
    }
    for &(other_bpm, other_raw) in qualified {
        if !(FOF_HALF_BAR_HIGH_MIN..=FOF_HALF_BAR_HIGH_MAX).contains(&other_bpm) {
            continue;
        }
        let ratio = other_bpm / candidate_bpm;
        if !ratio_matches(ratio, FOF_HALF_BAR_RATIO, PROFILE_OCTAVE_TOLERANCE) {
            continue;
        }
        if other_raw >= candidate_raw * FOF_HALF_BAR_MIN_RAW_RATIO {
            return true;
        }
    }
    false
}

fn four_on_floor_shuffle_high_rejected(
    candidate_bpm: f64,
    candidate_raw: f64,
    qualified: &[(f64, f64)],
) -> bool {
    if !(FOF_SHUFFLE_HIGH_MIN..=FOF_SHUFFLE_HIGH_MAX).contains(&candidate_bpm) {
        return false;
    }
    for &(other_bpm, other_raw) in qualified {
        if !(FOF_SHUFFLE_LOW_MIN..=FOF_SHUFFLE_LOW_MAX).contains(&other_bpm) {
            continue;
        }
        let ratio = candidate_bpm / other_bpm;
        if !ratio_matches(ratio, FOF_SHUFFLE_RATIO, PROFILE_OCTAVE_TOLERANCE) {
            continue;
        }
        if other_raw >= candidate_raw * FOF_SHUFFLE_MIN_RAW_RATIO {
            return true;
        }
    }
    false
}

fn dub_midband_rejected(candidate_bpm: f64, candidate_raw: f64, qualified: &[(f64, f64)]) -> bool {
    if !(DUB_MID_LOW_MIN..=DUB_MID_LOW_MAX).contains(&candidate_bpm) {
        return false;
    }
    for &(other_bpm, other_raw) in qualified {
        if !(DUB_MID_ROOT_MIN..=DUB_MID_ROOT_MAX).contains(&other_bpm) {
            continue;
        }
        let ratio = candidate_bpm / other_bpm;
        if !ratio_matches(ratio, DUB_MID_RATIO, PROFILE_OCTAVE_TOLERANCE) {
            continue;
        }
        if other_raw >= candidate_raw * DUB_MID_MIN_RAW_RATIO {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genre_mapping_covers_urban_tags() {
        assert_eq!(
            octave_profile_from_genre("Reggae"),
            OctaveProfile::RootsReggae
        );
        assert_eq!(octave_profile_from_genre("Dub"), OctaveProfile::Dub);
        assert_eq!(
            octave_profile_from_genre("Dancehall"),
            OctaveProfile::Dancehall
        );
        assert_eq!(
            octave_profile_from_genre("House"),
            OctaveProfile::FourOnFloor
        );
        assert_eq!(
            octave_profile_from_genre("UK Garage"),
            OctaveProfile::FourOnFloor
        );
        assert_eq!(octave_profile_from_genre("Hip-Hop"), OctaveProfile::Default);
        assert_eq!(
            octave_profile_from_genre("Reggae / Dub"),
            OctaveProfile::Dub
        );
    }

    #[test]
    fn roots_profile_rejects_here_i_come_shape() {
        let qualified = [(171.0, 2.33), (85.0, 2.71)];
        assert!(profile_doubletime_rejected(
            OctaveProfile::RootsReggae,
            171.0,
            2.33,
            &qualified
        ));
    }

    #[test]
    fn dancehall_profile_spares_all_night_shape() {
        let qualified = [(132.5, 3.48), (66.3, 3.41)];
        assert!(!profile_doubletime_rejected(
            OctaveProfile::Dancehall,
            132.5,
            3.48,
            &qualified
        ));
    }

    #[test]
    fn dub_profile_rejects_blind_prophet_shape() {
        let qualified = [(139.67, 5.034), (69.84, 4.992)];
        assert!(profile_doubletime_rejected(
            OctaveProfile::Dub,
            139.67,
            5.034,
            &qualified
        ));
    }

    #[test]
    fn four_on_floor_rejects_molly_half_bar() {
        let qualified = [(129.20, 8.754), (86.13, 8.002), (65.42, 4.498)];
        assert!(profile_subdivision_rejected(
            OctaveProfile::FourOnFloor,
            86.13,
            8.002,
            &qualified
        ));
        assert!(!profile_subdivision_rejected(
            OctaveProfile::FourOnFloor,
            129.20,
            8.754,
            &qualified
        ));
    }

    #[test]
    fn four_on_floor_rejects_jaden_shuffle_high() {
        let qualified = [(164.0, 5.45), (123.05, 6.73), (97.51, 5.34)];
        assert!(profile_subdivision_rejected(
            OctaveProfile::FourOnFloor,
            164.0,
            5.45,
            &qualified
        ));
    }

    #[test]
    fn dub_profile_rejects_midband_false_peak() {
        let qualified = [(93.0, 2.5), (70.0, 2.2)];
        assert!(profile_subdivision_rejected(
            OctaveProfile::Dub,
            93.0,
            2.5,
            &qualified
        ));
    }

    #[test]
    fn profile_skips_skank_only_for_house() {
        assert!(profile_skips_skank_pass(OctaveProfile::FourOnFloor));
        assert!(!profile_skips_skank_pass(OctaveProfile::Default));
    }
}
