//! Which octave a song's tempo is read in.
//!
//! A beat can be counted at half or double speed and both are "right":
//! 70 or 140 for dubstep, 87 or 174 for drum & bass. The audio does not
//! settle it, so Dub reads every tempo inside one octave — a
//! [`TempoOctave`], `[low, 2 × low)` BPM — the way Traktor, Serato and
//! rekordbox do with their BPM-range setting. The genre tag picks the
//! octave where it says something ([`OctaveProfile`]); the DJ's
//! preference picks it for everything else.
//!
//! This replaced a prior over 60–200 BPM plus nine genre-specific
//! rejection rules (2026-11-19). Scored against 1,758 grids the operator
//! had locked in Traktor, those agreed on 62 % of tempos — one rule,
//! written for reggae, threw the right tempo away on every untagged
//! four-on-the-floor song. The same detector with no rules and these
//! octaves agrees on 96 %. See
//! `docs/investigations/BPM-DETECTOR-V2-INVESTIGATION.md` §8.
//!
//! ## Genre tags recognised
//!
//! The matcher covers the urban-scratch niche the product targets
//! (PRD §1). Anything else is [`OctaveProfile::Default`]: the DJ's own
//! range.
//!
//! * **HipHop** — `hip-hop`, `hip hop`, `hiphop`, `rap`, `trap`,
//!   `r&b`, `r & b`, `rnb`, `boom bap` / `boom-bap` / `boombap`,
//!   `drill`, `lo-fi` / `lofi` / `lo fi`, `reggaeton` (dembow —
//!   explicit override of the `reggae` substring trap).
//! * **FourOnFloor** — `house`, `garage`, `ukg`, `4x4` / `4×4`,
//!   `2-step` / `2step` / `two-step`, `bassline`, `grime`, `dubstep`,
//!   `jersey club`, `baltimore club`, `techno`, `trance`, `electro`,
//!   `club`.
//! * **DrumAndBass** — `drum & bass` / `drum and bass` /
//!   `drum n bass` / `drum'n'bass` / `drumandbass`, `dnb`, `d&b`,
//!   `jungle`, `neurofunk`, `breakcore`, `liquid funk`, `footwork`,
//!   `juke`, `uk hardcore`, `happy hardcore`.
//! * **Dancehall** — `dancehall`, `ragga`, `bashment`.
//! * **Dub** — `dub` (not `dubstep`).
//! * **RootsReggae** — `reggae` (after the reggaeton override),
//!   `rocksteady`, `roots`, `lovers`, `stepper(s)`, and `ska` as a word
//!   of its own (`Skate Punk` is not ska).

/// One octave of tempo: `[low, 2 × low)` BPM. Every tempo Dub reads for a
/// song lands inside the song's octave, so half and double are never both
/// on the table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoOctave {
    low: f64,
}

impl TempoOctave {
    /// The lowest an octave may start: [`crate::MIN_BPM`].
    pub const MIN_LOW: f64 = crate::MIN_BPM;
    /// The highest an octave may start, so it still ends at
    /// [`crate::MAX_BPM`].
    pub const MAX_LOW: f64 = crate::MAX_BPM / 2.0;

    /// 78–156: the octave that holds the most of an unlabelled library.
    /// Of the operator's 629 locked Traktor grids with no recognised
    /// genre, 97.9 % sit in it.
    pub const DEFAULT: Self = Self { low: 78.0 };

    /// The octaves offered as a preference, by their low end.
    pub const CHOICES: [f64; 5] = [60.0, 68.0, 78.0, 88.0, 100.0];

    /// The octave starting at `low` BPM, or `None` outside
    /// [`Self::MIN_LOW`]..=[`Self::MAX_LOW`].
    #[must_use]
    pub fn from_low(low: f64) -> Option<Self> {
        (low.is_finite() && (Self::MIN_LOW..=Self::MAX_LOW).contains(&low)).then_some(Self { low })
    }

    /// The slowest tempo in the octave (inclusive).
    #[must_use]
    pub const fn low(self) -> f64 {
        self.low
    }

    /// Twice [`Self::low`]: the first tempo past the octave.
    #[must_use]
    pub fn high(self) -> f64 {
        self.low * 2.0
    }
}

impl Default for TempoOctave {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The genre family a tag belongs to, as far as tempo goes: which octave
/// its songs are mixed in. [`OctaveProfile::tempo_octave`] holds the
/// numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OctaveProfile {
    /// No genre, or one that says nothing about tempo: the DJ's own
    /// range decides.
    #[default]
    Default,
    /// Roots reggae, rocksteady, ska: counted at the one-drop, 60–120.
    RootsReggae,
    /// Dub (the reggae kind): 60–120.
    Dub,
    /// Dancehall, ragga, bashment: 75–150.
    Dancehall,
    /// House, garage, techno, grime, dubstep: 88–176. Dubstep is mixed
    /// at 140, not 70 — 157 of the operator's 158 locked dubstep grids
    /// sit between 120 and 150.
    FourOnFloor,
    /// Hip-hop, rap, trap, R&B: 68–136.
    HipHop,
    /// Drum & bass, jungle: 100–200.
    DrumAndBass,
}

impl OctaveProfile {
    /// The octave this family's tempos are read in; `fallback` (the DJ's
    /// preference) for [`OctaveProfile::Default`].
    ///
    /// Each is the octave holding the most of that family among the
    /// operator's locked Traktor grids (2026-11-19): hip-hop 98 %,
    /// house/garage/dubstep 99.7 %, reggae and dub 95 %, drum & bass
    /// 100 %, dancehall 98 %.
    #[must_use]
    pub fn tempo_octave(self, fallback: TempoOctave) -> TempoOctave {
        let low = match self {
            Self::Default => return fallback,
            Self::RootsReggae | Self::Dub => 60.0,
            Self::HipHop => 68.0,
            Self::Dancehall => 75.0,
            Self::FourOnFloor => 88.0,
            Self::DrumAndBass => 100.0,
        };
        TempoOctave { low }
    }
}

/// Map a container genre string to an [`OctaveProfile`].
///
/// Matching is case-insensitive and substring-based so tags like
/// `"Reggae / Dub"` or `"UK Garage 4x4"` resolve sensibly.
///
/// **Ordering is load-bearing.** The chain is `reggaeton →
/// dancehall → dubstep → dub → reggae → four-on-floor (incl. UKG /
/// grime) → drum & bass → hip-hop → default`. Substring greediness
/// forces these overrides:
///
/// * `"Reggaeton"` contains `"reggae"` but is a 90–100 BPM dembow
///   genre mixed at the kick, not the one-drop.
/// * `"Dubstep"` contains `"dub"` but is mixed at 140 with garage and
///   grime, so it is caught first — `"Dubstep - Reggae"` included.
/// * `"Skate Punk"` contains `"ska"`; ska only counts as a word of its
///   own.
/// * `"Lo-Fi House"` and `"4x4 House"` contain `"house"` and must
///   stay in the four-on-floor arm, which runs before the hip-hop arm
///   where `"lo-fi"` lives.
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
    if g.contains("dubstep") {
        return OctaveProfile::FourOnFloor;
    }
    if g.contains("dub") {
        return OctaveProfile::Dub;
    }
    if g.contains("reggae")
        || g.contains("rocksteady")
        || g.contains("roots")
        || g.contains("lovers")
        || g.contains("stepper")
        || g.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word == "ska")
    {
        return OctaveProfile::RootsReggae;
    }
    // Grime sits here with garage: both are mixed at 130–140, not at
    // the half-time a hip-hop tag would give them.
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
        "dub" => OctaveProfile::Dub,
        "dancehall" | "ragga" | "bashment" => OctaveProfile::Dancehall,
        "house" | "four_on_floor" | "four-on-floor" | "garage" | "ukg" | "uk_garage"
        | "uk-garage" | "4x4" | "2-step" | "2step" | "two_step" | "bassline" | "grime"
        | "dubstep" | "techno" | "trance" | "electro" => OctaveProfile::FourOnFloor,
        "hip_hop" | "hiphop" | "hip-hop" | "rap" | "trap" | "rnb" | "drill" | "lofi" | "lo-fi"
        | "lo_fi" | "reggaeton" => OctaveProfile::HipHop,
        "dnb" | "drum_and_bass" | "drum-and-bass" | "drumandbass" | "jungle" | "neurofunk"
        | "breakcore" | "liquid_funk" | "footwork" | "juke" | "uk_hardcore" | "happy_hardcore" => {
            OctaveProfile::DrumAndBass
        }
        _ => OctaveProfile::Default,
    }
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

    #[test]
    fn dubstep_is_read_with_garage_not_with_dub() {
        for tag in [
            "Dubstep",
            "Dubstep - Bretter",
            "Dubstep - Reggae",
            "dubstep",
        ] {
            assert_eq!(
                octave_profile_from_genre(tag),
                OctaveProfile::FourOnFloor,
                "{tag}"
            );
        }
        assert_eq!(octave_profile_from_genre("Dub"), OctaveProfile::Dub);
        assert_eq!(
            octave_profile_from_label("dubstep"),
            OctaveProfile::FourOnFloor
        );
    }

    #[test]
    fn ska_is_a_word_not_a_fragment() {
        for tag in ["Ska", "Reggae - Ska", "2 Tone / Ska"] {
            assert_eq!(
                octave_profile_from_genre(tag),
                OctaveProfile::RootsReggae,
                "{tag}"
            );
        }
        for tag in ["Skate Punk", "Skate Punk - Rock", "Alaska"] {
            assert_eq!(
                octave_profile_from_genre(tag),
                OctaveProfile::Default,
                "{tag}"
            );
        }
    }

    #[test]
    fn each_family_reads_in_its_own_octave() {
        let fallback = TempoOctave::from_low(88.0).unwrap();
        let low = |p: OctaveProfile| p.tempo_octave(fallback).low();
        assert_eq!(low(OctaveProfile::Default), 88.0, "the DJ's range");
        assert_eq!(low(OctaveProfile::RootsReggae), 60.0);
        assert_eq!(low(OctaveProfile::Dub), 60.0);
        assert_eq!(low(OctaveProfile::HipHop), 68.0);
        assert_eq!(low(OctaveProfile::Dancehall), 75.0);
        assert_eq!(low(OctaveProfile::FourOnFloor), 88.0);
        assert_eq!(low(OctaveProfile::DrumAndBass), 100.0);
    }

    #[test]
    fn an_octave_stays_inside_what_the_analyser_can_read() {
        assert!(TempoOctave::from_low(59.9).is_none());
        assert!(TempoOctave::from_low(100.1).is_none());
        assert!(TempoOctave::from_low(f64::NAN).is_none());
        let o = TempoOctave::from_low(60.0).unwrap();
        assert_eq!((o.low(), o.high()), (60.0, 120.0));
        assert_eq!(TempoOctave::from_low(100.0).unwrap().high(), crate::MAX_BPM);
        assert_eq!(TempoOctave::default(), TempoOctave::DEFAULT);
        for low in TempoOctave::CHOICES {
            assert!(TempoOctave::from_low(low).is_some(), "{low}");
        }
    }
}
