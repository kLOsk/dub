//! Genre-aware octave disambiguation (M11c.3d+).
//!
//! Offline library analysis can pass an [`OctaveProfile`] derived from
//! ID3 genre tags. Thru-mode streaming keeps [`OctaveProfile::Default`]
//! because live wax has no tag until fingerprint match (v1.1).
//!
//! ## Scope of the genre matcher
//!
//! The matcher prioritises the urban-scratch DJ niche the product
//! targets (PRD §1): hip-hop, reggae / dub family, dancehall, UK
//! garage, jungle / DnB, dubstep, plus their substyles. Genres
//! outside that niche fall to [`OctaveProfile::Default`] and rely
//! on the M11c.3a perceptual prior — they still analyse, just
//! without a tempo-band hint. Tags accepted today:
//!
//! * **HipHop** — `hip-hop`, `hip hop`, `hiphop`, `rap`, `trap`,
//!   `r&b`, `r & b`, `rnb`, `boom bap` / `boom-bap` / `boombap`,
//!   `drill` (UK / Brooklyn / Chicago, kick perceived at ~70 BPM),
//!   `lo-fi` / `lofi` / `lo fi` (chill hop), `reggaeton`
//!   (dembow, 90–100 BPM — explicit override of the `reggae`
//!   substring trap).
//! * **FourOnFloor** — `house`, `garage`, `ukg`, `4x4` / `4×4`,
//!   `2-step` / `2step` / `two-step`, `bassline`, `grime` (140 BPM
//!   mix tempo, sparse kick), `jersey club`, `baltimore club`,
//!   `techno`, `trance`, `electro`, `club`.
//! * **DrumAndBass** — `drum & bass` / `drum and bass` /
//!   `drum n bass` / `drum'n'bass` / `drumandbass`, `dnb`, `d&b`,
//!   `jungle`, `neurofunk`, `breakcore`, `liquid funk`, `footwork`,
//!   `juke`, `uk hardcore`, `happy hardcore`.
//! * **Dancehall** — `dancehall`, `ragga`, `bashment`.
//! * **Dub** — `dub` (but not `dubstep` greediness), `dubstep`.
//! * **RootsReggae** — `reggae` (after reggaeton override),
//!   `rocksteady`, `roots`, `lovers`, `stepper(s)`, `ska`.

/// How pass 2 resolves octave / subdivision ambiguity when metadata
/// supplies genre context. [`Default`] is the mixable-band prior from
/// M11c.3a.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OctaveProfile {
    /// Untagged or genre-tagged "other" — existing perceptual prior.
    /// No lower- or upper-octave bias; pass 2 picks the spectral
    /// winner.
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
    /// PRD-BEATS Round 6 §6d — Hip-hop, rap, trap, R&B, boom-bap.
    /// Mix tempo lives at 75–105 BPM (kick on 1+3 grid). Without
    /// a lower-octave bias, the busy hi-hat / 16th-note rhythm
    /// section wins spectral energy at 150–210 BPM and the auto-
    /// analyser picks that octave instead of the perceptual kick
    /// tempo. Mirrors `RootsReggae`'s sibling-rejection logic
    /// against the same 150–200 BPM band.
    HipHop,
    /// PRD-BEATS Round 6 §6e — Drum & bass, jungle. Mix tempo
    /// lives at 160–185 BPM (rolling-kick / amen-break grid).
    /// Without an upper-octave preference an analyser presented
    /// with a strong snare backbeat at the 2-beat period can
    /// resolve to half tempo (~82 BPM) — the same K-S-backbeat
    /// half-tempo problem the rolling-dnb fixture in
    /// `tests/genre_octave.rs` documents. Mirrors `Dancehall`'s
    /// "keep full-tempo" behaviour for the fast-urban band.
    DrumAndBass,
}

/// Map a container genre string to an [`OctaveProfile`].
///
/// Matching is case-insensitive and substring-based so tags like
/// `"Reggae / Dub"` or `"UK Garage 4x4"` resolve sensibly.
///
/// **Ordering is load-bearing.** The chain is `reggaeton →
/// dancehall → dub → reggae → four-on-floor (incl. UKG / grime)
/// → drum & bass → hip-hop → default`. Two cases force an explicit
/// override of substring greediness:
///
/// * `"Reggaeton"` contains `"reggae"` but is a 90–100 BPM dembow
///   genre that mixes at the kick (HipHop band), not the half-
///   time one-drop. Without the up-front check, every reggaeton
///   tag misroutes to [`OctaveProfile::RootsReggae`] and the
///   half-time bias actively flips the perceived ~95 BPM into
///   ~48.
/// * `"Lo-Fi House"` and `"4x4 House"` contain `"house"` and must
///   stay in the four-on-floor arm. Ordering is fine here because
///   the four-on-floor arm runs before the hip-hop arm (`"lo-fi"`
///   lives), so the house tag wins by position.
#[must_use]
pub fn octave_profile_from_genre(genre: &str) -> OctaveProfile {
    let g = genre.trim().to_ascii_lowercase();
    if g.is_empty() {
        return OctaveProfile::Default;
    }
    // Reggaeton MUST win before the reggae arm. Substring trap:
    // `"reggaeton".contains("reggae") == true`. See header comment.
    if g.contains("reggaeton") {
        return OctaveProfile::HipHop;
    }
    if g.contains("dancehall") || g.contains("ragga") || g.contains("bashment") {
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
        || g.contains("stepper")
        || g.contains("ska")
    {
        return OctaveProfile::RootsReggae;
    }
    // UK garage family lives here. The 4/4 kick + 130–135 BPM mix
    // tempo of UKG, 4x4 garage, speed garage, 2-step (kick on 1+3
    // perceptually still mixed at 130), and bassline all sit
    // squarely inside the FourOnFloor band (115–145 BPM) and benefit
    // from `four_on_floor_halfbar_rejected` killing the ~87 BPM
    // half-time twin. `"speed garage"` already passes via `"garage"`;
    // explicit entries for `"ukg"`, `"4x4"`, `"4×4"`, `"2-step"`,
    // `"bassline"` close the gaps where the bare token is what
    // ships in id3.
    //
    // Grime: 140 BPM mix tempo, sparse kick on 1+3, snare on 2+4,
    // bass synth at 140. UK grime DJs mix at 140, not 70. Mapping
    // to FourOnFloor keeps the 140 octave intact (the HipHop
    // profile would actively reject 140 in favour of a 70 BPM
    // sibling, which is the wrong call for grime).
    if g.contains("house")
        || g.contains("garage")
        || g.contains("ukg")
        || g.contains("4x4")
        || g.contains("4×4")
        || g.contains("2-step")
        || g.contains("2step")
        || g.contains("2 step")
        || g.contains("two-step")
        || g.contains("two step")
        || g.contains("bassline")
        || g.contains("grime")
        || g.contains("jersey club")
        || g.contains("baltimore club")
        || g.contains("techno")
        || g.contains("trance")
        || g.contains("electro")
        || g.contains("club")
    {
        return OctaveProfile::FourOnFloor;
    }
    // PRD-BEATS Round 6 §6e. Match DnB / jungle BEFORE the
    // hip-hop branch: "drum & bass" and "drum and bass" don't
    // contain the substring "hip" but several DnB sub-genres
    // (`"liquid hip-hop"`, `"hip-hop jungle"`) ship with a
    // composite tag, and the user's mix tempo for those is the
    // DnB tempo, not the hip-hop tempo. `"jungle"` is its own
    // umbrella term for the early-90s DnB lineage.
    //
    // Extended for the urban-niche DnB family: neurofunk (170–180),
    // breakcore (170–220), liquid funk (170–175), footwork / juke
    // (Chicago, 150–160), and the hardcore-rave variants UK
    // hardcore + happy hardcore (165–180). Bare `"hardcore"` is
    // NOT matched: it's overloaded by hardcore punk, hardcore rap,
    // hardcore techno, gabber-style hardcore, etc. — the explicit
    // `"uk hardcore"` / `"happy hardcore"` tokens are the only
    // ones whose tempo band is unambiguous.
    if g.contains("drum & bass")
        || g.contains("drum and bass")
        || g.contains("drum n bass")
        || g.contains("drum'n'bass")
        || g.contains("drumandbass")
        || g.contains("dnb")
        || g.contains("d&b")
        || g.contains("jungle")
        || g.contains("neurofunk")
        || g.contains("breakcore")
        || g.contains("liquid funk")
        || g.contains("footwork")
        || g.contains("juke")
        || g.contains("uk hardcore")
        || g.contains("happy hardcore")
    {
        return OctaveProfile::DrumAndBass;
    }
    // PRD-BEATS Round 6 §6d. `"r&b"` is matched as `"r & b"`
    // and `"rnb"`. `"trap"` is matched even though there's a
    // niche reggae-trap fusion; the mix tempo of the fusion
    // material is overwhelmingly in the 75–105 hip-hop band,
    // not the 65–95 roots band, so HipHop is the right call.
    //
    // Urban subgenre additions:
    // * `"drill"` (UK drill, Brooklyn drill, Chicago drill) —
    //   kick on the 1 at ~70 BPM, snares at ~140; DJs mix at
    //   ~70 so the HipHop sibling rule (kill 135–180 when paired
    //   with a 60–100 sibling) is exactly what we want.
    // * `"lo-fi"` / `"lofi"` (lo-fi hip-hop, chill hop, study
    //   beats) — 60–90 BPM kick-driven instrumentals. Note that
    //   `"Lo-fi House"` resolves to FourOnFloor above by chain
    //   ordering: the FourOnFloor arm runs first and `"house"`
    //   wins. So `"lo-fi"` only lands here when no house token
    //   accompanies it.
    if g.contains("hip-hop")
        || g.contains("hip hop")
        || g.contains("hiphop")
        || g.contains("rap")
        || g.contains("trap")
        || g.contains("r&b")
        || g.contains("r & b")
        || g.contains("rnb")
        || g.contains("boom bap")
        || g.contains("boom-bap")
        || g.contains("boombap")
        || g.contains("drill")
        || g.contains("lo-fi")
        || g.contains("lofi")
        || g.contains("lo fi")
    {
        return OctaveProfile::HipHop;
    }
    OctaveProfile::Default
}

/// Parse a manifest profile label (`roots`, `dub`, `house`, …).
///
/// Used by `dub diagnose --profile <label>` and the test corpus
/// manifest. Mirrors the genre-tag matcher's family groupings,
/// but accepts canonical short tokens rather than substring
/// fragments so the CLI surface stays unambiguous.
#[must_use]
pub fn octave_profile_from_label(label: &str) -> OctaveProfile {
    match label.trim().to_ascii_lowercase().as_str() {
        "roots" | "roots_reggae" | "reggae" | "rocksteady" | "ska" | "steppers" => {
            OctaveProfile::RootsReggae
        }
        "dub" | "dubstep" => OctaveProfile::Dub,
        "dancehall" | "ragga" | "bashment" => OctaveProfile::Dancehall,
        "house" | "four_on_floor" | "four-on-floor" | "garage" | "ukg" | "uk_garage"
        | "uk-garage" | "4x4" | "2-step" | "2step" | "two_step" | "bassline" | "grime"
        | "techno" | "trance" | "electro" => OctaveProfile::FourOnFloor,
        "hip_hop" | "hiphop" | "hip-hop" | "rap" | "trap" | "rnb" | "drill" | "lofi" | "lo-fi"
        | "lo_fi" | "reggaeton" => OctaveProfile::HipHop,
        "dnb" | "drum_and_bass" | "drum-and-bass" | "drumandbass" | "jungle" | "neurofunk"
        | "breakcore" | "liquid_funk" | "footwork" | "juke" | "uk_hardcore" | "happy_hardcore" => {
            OctaveProfile::DrumAndBass
        }
        _ => OctaveProfile::Default,
    }
}

/// Reggae skank pass false-flips house kicks at ~129 BPM (M11c.3f).
#[must_use]
pub fn profile_skips_skank_pass(profile: OctaveProfile) -> bool {
    matches!(profile, OctaveProfile::FourOnFloor)
}

/// Returns `true` when the profile forbids the post-pass-2
/// `octave_self_verify` from swapping the chosen BPM down to its
/// half (BPM → BPM/2).
///
/// Genres whose mix tempo lives in the **upper** octave (4/4 kick
/// or kick-roll grid) suffer a structural LSQ trap at the half
/// octave: when the production has sparse kick patterns (UK
/// garage 2-step, broken-beat house, DnB kick rolls), the predicted
/// beat positions at the true tempo land in dead air half the time,
/// while the half-tempo grid only predicts every-other-beat and
/// happens to align with the kicks that *do* land. RMS at the half
/// octave reads dramatically tighter even though the half octave is
/// musically wrong — Oppidan & Cutty Ranks "Armed & Dangerous" was
/// the canonical case: true 133 BPM UKG with rms 31.5 ms at 133,
/// rms 5.8 ms at 66.5; profile-blind self-verify swapped to 66.5
/// and surfaced it in the deck header.
///
/// The profile-driven block is the right layer to fix this: the
/// genre tag already says "the kick / mix tempo is in the upper
/// octave, don't second-guess pass 2's pick". Pass 2 itself
/// already prefers the upper octave for these profiles via the
/// perceptual prior + `four_on_floor_halfbar_rejected` /
/// `profile_halftime_rejected` (DnB) / Dancehall's
/// continue-don't-reject behaviour; the self-verify must respect
/// that decision instead of undoing it.
#[must_use]
pub fn profile_blocks_half_octave_swap(profile: OctaveProfile) -> bool {
    matches!(
        profile,
        OctaveProfile::FourOnFloor | OctaveProfile::DrumAndBass | OctaveProfile::Dancehall
    )
}

/// Returns `true` when the profile forbids the post-pass-2
/// `octave_self_verify` from swapping the chosen BPM up to its
/// double (BPM → BPM×2).
///
/// Mirror of [`profile_blocks_half_octave_swap`]. Genres whose mix
/// tempo lives in the **lower** octave (one-drop, dub, hip-hop kick
/// on 1+3) suffer the inverse LSQ trap: when the production has a
/// dense hi-hat or shaker layer, the upper-octave grid finds
/// tighter onset alignment to the hat / shaker pulses than the
/// kick-aligned lower octave does. The profile already encodes
/// "mix at the lower octave"; the self-verify must respect that.
#[must_use]
pub fn profile_blocks_double_octave_swap(profile: OctaveProfile) -> bool {
    matches!(
        profile,
        OctaveProfile::RootsReggae | OctaveProfile::Dub | OctaveProfile::HipHop
    )
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

/// PRD-BEATS Round 6 §6d — Hip-hop sibling-rejection threshold.
/// Same shape as `ROOTS_SIBLING_MIN_RAW_RATIO` but slightly
/// stricter (the hip-hop hi-hat layer is often 4-to-1 louder than
/// the kick in raw spectral terms, so we accept the lower octave
/// even when its raw spectral score is only 70 % of the upper
/// octave's). Validated against the synthetic hip-hop fixture
/// (`drum_pattern_hip_hop`) at 80, 88, and 100 BPM.
const HIPHOP_SIBLING_MIN_RAW_RATIO: f64 = 0.70;

/// Returns `true` when a qualifying upper-octave candidate should be
/// discarded because genre context says the lower octave is the mix
/// tempo (M11c.3d). Extended in PRD-BEATS Round 6 §6d to cover
/// [`OctaveProfile::HipHop`].
pub(crate) fn profile_doubletime_rejected(
    profile: OctaveProfile,
    candidate_bpm: f64,
    candidate_raw: f64,
    qualified: &[(f64, f64)],
) -> bool {
    if !matches!(
        profile,
        OctaveProfile::RootsReggae | OctaveProfile::Dub | OctaveProfile::HipHop
    ) {
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
            OctaveProfile::HipHop => HIPHOP_SIBLING_MIN_RAW_RATIO,
            OctaveProfile::Dub => {
                let raw_gap = (candidate_raw - other_raw).abs() / candidate_raw.max(other_raw);
                if raw_gap <= DUB_NEAR_TIE_MAX_GAP {
                    return true;
                }
                DUB_SIBLING_MIN_RAW_RATIO
            }
            OctaveProfile::Default
            | OctaveProfile::Dancehall
            | OctaveProfile::FourOnFloor
            | OctaveProfile::DrumAndBass => {
                continue;
            }
        };

        if other_raw >= candidate_raw * min_ratio {
            return true;
        }
    }
    false
}

/// PRD-BEATS Round 6 §6e — DnB / jungle: REJECT the lower-octave
/// candidate when a credible 150–200 BPM sibling exists at the
/// 2:1 ratio. Inverse of [`profile_doubletime_rejected`]: that
/// function discards an UPPER-octave candidate (for genres whose
/// mix tempo is low); this discards a LOWER-octave candidate
/// (for genres whose mix tempo is high).
///
/// DnB at 165–185 mixed at the kick-roll grid often produces a
/// strong half-time autocorrelation peak at 82–92 BPM because
/// the snare backbeat lands every other beat (same K-S half-time
/// problem documented in the rolling-dnb regression test). The
/// existing `Dancehall` profile sidesteps the symmetric issue by
/// NOT actively rejecting either side (the spectral winner just
/// happens to be the upper); this rule is more decisive because
/// for DnB the upper octave should ALWAYS win when paired —
/// nobody DJs a 170 BPM DnB track at 85.
pub(crate) fn profile_halftime_rejected(
    profile: OctaveProfile,
    candidate_bpm: f64,
    candidate_raw: f64,
    qualified: &[(f64, f64)],
) -> bool {
    if !matches!(profile, OctaveProfile::DrumAndBass) {
        return false;
    }

    if !(PROFILE_LOW_BPM_MIN..=PROFILE_LOW_BPM_MAX).contains(&candidate_bpm) {
        return false;
    }

    for &(other_bpm, other_raw) in qualified {
        if !(PROFILE_HIGH_BPM_MIN..=PROFILE_HIGH_BPM_MAX).contains(&other_bpm) {
            continue;
        }
        let ratio = other_bpm / candidate_bpm;
        if (ratio - PROFILE_OCTAVE_RATIO).abs() > PROFILE_OCTAVE_TOLERANCE * PROFILE_OCTAVE_RATIO {
            continue;
        }
        // Upper sibling must carry at least 70 % of the candidate
        // (lower) raw score. Symmetric with `HIPHOP_SIBLING_MIN_
        // RAW_RATIO`: the K-S half-time problem regularly pushes
        // the lower octave 20–40 % above the upper on raw score,
        // so requiring the upper to be near-equal would never
        // fire. 0.70 catches the structural-DnB case while
        // staying silent when the lower octave is overwhelmingly
        // dominant (true 85 BPM track tagged "DnB" by mistake).
        if other_raw >= candidate_raw * 0.70 {
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
        assert_eq!(
            octave_profile_from_genre("Reggae / Dub"),
            OctaveProfile::Dub
        );
    }

    /// PRD-BEATS Round 6 §6d: every common hip-hop tag must map
    /// to the `HipHop` profile, not `Default`. The user-reported
    /// "Bangin' Westside Connection" track ships with the tag
    /// `"Hip-Hop"`; pre-fix this mapped to `Default` and the
    /// auto-analyser locked at 175 BPM (the hat octave) instead
    /// of 88 BPM (the kick octave).
    #[test]
    fn genre_mapping_covers_hip_hop_family() {
        for tag in [
            "Hip-Hop",
            "Hip Hop",
            "HipHop",
            "hip-hop",
            "Rap",
            "Trap",
            "R&B",
            "R & B",
            "RnB",
            "Boom Bap",
            "Boom-Bap",
            "Boombap",
            "Conscious Hip-Hop",
            "West Coast Rap",
        ] {
            assert_eq!(
                octave_profile_from_genre(tag),
                OctaveProfile::HipHop,
                "tag {tag:?} should map to HipHop"
            );
        }
    }

    /// PRD-BEATS Round 6 §6e + niche-genres pass: every common
    /// DnB / jungle / fast-urban tag must map to the `DrumAndBass`
    /// profile so an inverted K-S half-time peak at ~85 BPM cannot
    /// beat the true ~170 BPM tempo when the track is tagged.
    #[test]
    fn genre_mapping_covers_drum_and_bass_family() {
        for tag in [
            "Drum & Bass",
            "Drum and Bass",
            "Drum n Bass",
            "Drum'n'Bass",
            "DnB",
            "D&B",
            "drumandbass",
            "Jungle",
            "Liquid DnB",
            "Liquid Funk",
            "Neurofunk",
            "Breakcore",
            "Footwork",
            "Juke",
            "UK Hardcore",
            "Happy Hardcore",
        ] {
            assert_eq!(
                octave_profile_from_genre(tag),
                OctaveProfile::DrumAndBass,
                "tag {tag:?} should map to DrumAndBass"
            );
        }
    }

    /// Niche-genres pass: UK garage family + grime. Every tag
    /// that the urban-scratch DJ niche uses to mean "130–140 BPM
    /// 4/4 (or 4/4-perceptible) kick" must land on
    /// `FourOnFloor` so the half-bar and shuffle-high
    /// rejections fire. `"speed garage"` and `"uk garage"` ride
    /// the existing `"garage"` substring; the explicit tokens
    /// here pin the bare shorthand IDs that ship in id3.
    #[test]
    fn genre_mapping_covers_uk_garage_family() {
        for tag in [
            "UKG",
            "UK Garage",
            "Speed Garage",
            "4x4",
            "4x4 Garage",
            "4×4",
            "2-Step",
            "2 Step",
            "2step",
            "Two-Step",
            "Two Step",
            "Bassline",
            "Grime",
            "Jersey Club",
            "Baltimore Club",
        ] {
            assert_eq!(
                octave_profile_from_genre(tag),
                OctaveProfile::FourOnFloor,
                "tag {tag:?} should map to FourOnFloor"
            );
        }
    }

    /// Niche-genres pass: hip-hop subgenres added in the same
    /// cycle as UKG. Drill (kick at 70), lo-fi (60–90 chill hop),
    /// and reggaeton (90–100 dembow) all sit in or near the
    /// HipHop 75–105 mix-tempo band and want the 2:1 upper-octave
    /// rejection rule that profile carries.
    #[test]
    fn genre_mapping_covers_hip_hop_subgenres() {
        for tag in [
            "Drill",
            "UK Drill",
            "Brooklyn Drill",
            "Chicago Drill",
            "Lo-Fi",
            "Lo-Fi Hip Hop",
            "Lofi",
            "LoFi",
            "Lo Fi",
            "Reggaeton",
            "Latin Reggaeton",
        ] {
            assert_eq!(
                octave_profile_from_genre(tag),
                OctaveProfile::HipHop,
                "tag {tag:?} should map to HipHop"
            );
        }
    }

    /// Regression for the substring trap that motivated the
    /// reggaeton up-front check: `"Reggaeton".contains("reggae")
    /// == true`, so without the override the reggae arm would
    /// claim it and the RootsReggae half-time bias would actively
    /// degrade analysis of a 90–100 BPM dembow track.
    #[test]
    fn reggaeton_does_not_leak_into_roots_reggae() {
        assert_eq!(
            octave_profile_from_genre("Reggaeton"),
            OctaveProfile::HipHop
        );
        assert_eq!(
            octave_profile_from_genre("Reggaeton / Trap"),
            OctaveProfile::HipHop
        );
        // The reggae arm still claims plain reggae tags — the
        // override is strictly scoped to the `"reggaeton"`
        // substring.
        assert_eq!(
            octave_profile_from_genre("Reggae"),
            OctaveProfile::RootsReggae
        );
        assert_eq!(
            octave_profile_from_genre("Roots Reggae"),
            OctaveProfile::RootsReggae
        );
    }

    /// Lo-fi house must stay in FourOnFloor — the `"house"`
    /// substring wins by chain order over the new `"lo-fi"`
    /// addition in the HipHop arm. Pure `"lo-fi"` (no house
    /// modifier) must reach the HipHop arm.
    #[test]
    fn lo_fi_house_stays_in_four_on_floor() {
        assert_eq!(
            octave_profile_from_genre("Lo-Fi House"),
            OctaveProfile::FourOnFloor
        );
        assert_eq!(
            octave_profile_from_genre("Lofi House"),
            OctaveProfile::FourOnFloor
        );
        assert_eq!(octave_profile_from_genre("Lo-Fi"), OctaveProfile::HipHop);
    }

    /// Bashment is Jamaican slang for dancehall; it must land in
    /// the same profile as `"Dancehall"` so the 130–145 BPM mix
    /// tempo survives octave disambiguation.
    #[test]
    fn bashment_maps_to_dancehall() {
        assert_eq!(
            octave_profile_from_genre("Bashment"),
            OctaveProfile::Dancehall
        );
        assert_eq!(
            octave_profile_from_genre("Dancehall / Bashment"),
            OctaveProfile::Dancehall
        );
    }

    /// Roots-reggae subgenre extensions: steppers (75–85 BPM
    /// roots, deliberate steady kick on every beat) joins the
    /// existing one-drop / rocksteady / lovers / ska family.
    #[test]
    fn steppers_maps_to_roots_reggae() {
        assert_eq!(
            octave_profile_from_genre("Steppers"),
            OctaveProfile::RootsReggae
        );
        assert_eq!(
            octave_profile_from_genre("Roots Steppers"),
            OctaveProfile::RootsReggae
        );
    }

    /// PRD-BEATS Round 6 §6d: hip-hop profile rejects the upper
    /// octave with a less strict raw-score gate than the global
    /// M11c.3e rule (0.70 instead of 0.96). Validates against a
    /// Bangin'-shape qualified list where the 175 BPM peak wins
    /// on raw but the 87 BPM peer carries 75 % of its score.
    #[test]
    fn hiphop_profile_rejects_bangin_shape() {
        let qualified = [(175.0, 1.0), (87.0, 0.75)];
        assert!(
            profile_doubletime_rejected(OctaveProfile::HipHop, 175.0, 1.0, &qualified),
            "HipHop must reject the 175 BPM peak when a 75%-strength 87 BPM peer exists"
        );
        // Default must NOT reject — that's the pre-Round 6 behaviour
        // that produced Bangin's wrong octave in the first place.
        assert!(
            !profile_doubletime_rejected(OctaveProfile::Default, 175.0, 1.0, &qualified),
            "Default must keep the 175 BPM spectral winner (regression \
             of the Bangin' diagnosis)"
        );
    }

    /// PRD-BEATS Round 6 §6e: DnB profile rejects the lower
    /// octave when paired with a credible upper-octave sibling.
    /// Mirror of the hip-hop test but inverted: the 87 BPM peak
    /// gets rejected even though it wins on raw, because the
    /// 174 BPM peer is credible (70 % of the candidate's score).
    #[test]
    fn drumandbass_profile_rejects_inverted_halftime() {
        let qualified = [(87.0, 1.0), (174.0, 0.75)];
        assert!(
            profile_halftime_rejected(OctaveProfile::DrumAndBass, 87.0, 1.0, &qualified),
            "DrumAndBass must reject the 87 BPM half-time peak when a \
             75%-strength 174 BPM peer exists"
        );
        // Non-DnB profiles must keep the 87 BPM candidate.
        for p in [
            OctaveProfile::Default,
            OctaveProfile::HipHop,
            OctaveProfile::RootsReggae,
        ] {
            assert!(
                !profile_halftime_rejected(p, 87.0, 1.0, &qualified),
                "{p:?} must not reject the 87 BPM peak"
            );
        }
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
