//! The dub siren's bank — the five shots the app offers.
//!
//! The DSP crate keeps three complete chip recreations (the HK628 toy IC,
//! the Benidub DS01E voice, the SN76477) and each is a faithful table of
//! *that chip's* sounds; the HK623 twin even indexes into the HK628's. The
//! product cut happens here instead: one flat bank that picks the five
//! sounds worth a key, each naming the voice it plays on. The UI never
//! sees a unit — there is no selector to relocate the pads — and firing
//! routes by the shot, not by a per-deck mode.

/// Which chip recreation a shot plays on, and the program / preset index
/// in that chip's own table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SirenVoiceId {
    /// A [`dub_dsp::Hk628`] program (the GS1-style toy-chip shots).
    Hk628(usize),
    /// A Benidub DS01E MODE patch on the generic [`dub_dsp::SirenVoice`].
    Ds01e(usize),
    /// A [`dub_dsp::Sn76477`] preset patch.
    Sn76477(usize),
}

/// One shot in the bank: the name the key carries and the voice it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SirenShot {
    /// Display name, as printed on the key.
    pub name: &'static str,
    /// The chip and the entry in its table that this shot plays.
    pub voice: SirenVoiceId,
}

/// The bank, in fire order — the index is the preset id the FFI takes and
/// the key's position on the box, left to right.
pub const SIREN_BANK: [SirenShot; 5] = [
    SirenShot {
        name: "Rifle Gun",
        voice: SirenVoiceId::Hk628(0),
    },
    SirenShot {
        name: "Alarm",
        voice: SirenVoiceId::Hk628(2),
    },
    // The DS01E's Sine 1 — "1" only meant something beside Sine 2.
    SirenShot {
        name: "Sine",
        voice: SirenVoiceId::Ds01e(0),
    },
    SirenShot {
        name: "Laser",
        voice: SirenVoiceId::Sn76477(0),
    },
    SirenShot {
        name: "Siren",
        voice: SirenVoiceId::Sn76477(4),
    },
];

/// Number of shots in the bank.
pub const SIREN_BANK_COUNT: usize = SIREN_BANK.len();

/// Shot `id`, or `None` past the end of the bank.
#[must_use]
pub fn siren_shot(id: usize) -> Option<&'static SirenShot> {
    SIREN_BANK.get(id)
}

/// Display name of shot `id` (empty past the end of the bank).
#[must_use]
pub fn siren_shot_name(id: usize) -> &'static str {
    SIREN_BANK.get(id).map_or("", |s| s.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shot_points_inside_its_chips_table() {
        for shot in &SIREN_BANK {
            let in_range = match shot.voice {
                SirenVoiceId::Hk628(i) => i < dub_dsp::HK628_PROGRAM_COUNT,
                SirenVoiceId::Ds01e(i) => i < dub_dsp::BENIDUB_PRESET_COUNT,
                SirenVoiceId::Sn76477(i) => i < dub_dsp::SN76477_PRESET_COUNT,
            };
            assert!(in_range, "{} points past its chip's table", shot.name);
        }
    }

    #[test]
    fn the_bank_plays_the_sounds_it_names() {
        // The bank renames only Sine; every other name is the chip's own,
        // so a re-ordered chip table would be caught here.
        let chip_name = |v: SirenVoiceId| match v {
            SirenVoiceId::Hk628(i) => dub_dsp::hk628_program_name(i),
            SirenVoiceId::Ds01e(i) => dub_dsp::benidub_preset_name(i),
            SirenVoiceId::Sn76477(i) => dub_dsp::sn76477_preset_name(i),
        };
        assert_eq!(chip_name(SIREN_BANK[0].voice), "Rifle Gun");
        assert_eq!(chip_name(SIREN_BANK[1].voice), "Alarm");
        assert_eq!(chip_name(SIREN_BANK[2].voice), "Sine 1");
        assert_eq!(chip_name(SIREN_BANK[3].voice), "Laser");
        assert_eq!(chip_name(SIREN_BANK[4].voice), "Siren");
    }

    #[test]
    fn names_are_unique_and_lookups_bound() {
        let mut names: Vec<&str> = SIREN_BANK.iter().map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SIREN_BANK_COUNT);
        assert_eq!(siren_shot_name(SIREN_BANK_COUNT), "");
        assert!(siren_shot(SIREN_BANK_COUNT).is_none());
        assert_eq!(siren_shot(4).map(|s| s.name), Some("Siren"));
    }
}
