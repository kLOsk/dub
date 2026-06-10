//! Timecode definition table — the per-pressing LFSR/carrier parameters.
//!
//! These are the numeric *format constants* of every control signal Dub
//! can decode, mirroring xwax's `timecodes[]` table (Mark Hills,
//! GPL-3.0). Clean-room: parameters only — the decode *algorithm* lives
//! in [`crate::absolute`] (also derived from the published xwax method).
//!
//! Each [`TimecodeDef`] is one pressing/side. The same record family has
//! distinct seeds per side (so side A and side B never alias), and the CD
//! uses a different polynomial again — a capture only decodes against its
//! matching def, which is why the decoder auto-detects across candidates
//! ([`serato_candidates`]).

/// Read the bit on the secondary channel's *trailing* edge instead of the
/// leading one (phase-inverted timing). xwax `SWITCH_PHASE`.
pub const SWITCH_PHASE: u8 = 0x1;
/// Primary (data) channel is the **left**/ch0 instead of right/ch1.
/// xwax `SWITCH_PRIMARY`.
pub const SWITCH_PRIMARY: u8 = 0x2;
/// Sample the bit on the primary's negative half-cycle. xwax
/// `SWITCH_POLARITY`.
pub const SWITCH_POLARITY: u8 = 0x4;

/// One control-signal pressing: its carrier, LFSR, length, and decode
/// flags. Field names and values match xwax's `struct timecode_def`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimecodeDef {
    /// Stable key (xwax name), e.g. `"serato_cd"`.
    pub name: &'static str,
    /// Human description.
    pub desc: &'static str,
    /// Carrier frequency in Hz (xwax calls this `resolution`).
    pub resolution: u32,
    /// LFSR width in bits — the position-window size.
    pub bits: u32,
    /// LFSR seed (the position-0 state).
    pub seed: u32,
    /// Galois feedback taps (used as `taps | 1` in the recurrence).
    pub taps: u32,
    /// Total bits pressed on the side.
    pub length: u32,
    /// Bits within which the signal is reliable (inside `length`).
    pub safe: u32,
    /// `SWITCH_*` decode flags.
    pub flags: u8,
}

impl TimecodeDef {
    /// Primary (data) channel is the right/ch1 unless `SWITCH_PRIMARY`.
    #[must_use]
    pub fn primary_right(&self) -> bool {
        self.flags & SWITCH_PRIMARY == 0
    }

    /// Read on the primary's positive half unless `SWITCH_POLARITY`.
    #[must_use]
    pub fn switch_polarity(&self) -> bool {
        self.flags & SWITCH_POLARITY != 0
    }

    /// Read on the secondary's trailing edge if `SWITCH_PHASE`.
    #[must_use]
    pub fn switch_phase(&self) -> bool {
        self.flags & SWITCH_PHASE != 0
    }
}

/// Every supported pressing. Values are xwax's `timecodes[]`.
pub static TIMECODE_DEFS: &[TimecodeDef] = &[
    TimecodeDef {
        name: "serato_2a",
        desc: "Serato CV02, side A",
        resolution: 1000,
        bits: 20,
        seed: 0x5_9017,
        taps: 0x3_61e4,
        length: 712_000,
        safe: 707_000,
        flags: 0,
    },
    TimecodeDef {
        name: "serato_2b",
        desc: "Serato CV02, side B",
        resolution: 1000,
        bits: 20,
        seed: 0x8_f3c6,
        taps: 0x4_f0d8,
        length: 922_000,
        safe: 917_000,
        flags: 0,
    },
    TimecodeDef {
        name: "serato_cd",
        desc: "Serato Control CD",
        resolution: 1000,
        bits: 20,
        seed: 0xd_8b40,
        taps: 0x3_4d54,
        length: 950_000,
        safe: 940_000,
        flags: 0,
    },
    TimecodeDef {
        name: "traktor_a",
        desc: "Traktor Scratch, side A",
        resolution: 2000,
        bits: 23,
        seed: 0x13_4503,
        taps: 0x04_1040,
        length: 1_500_000,
        safe: 1_480_000,
        flags: SWITCH_PRIMARY | SWITCH_POLARITY | SWITCH_PHASE,
    },
    TimecodeDef {
        name: "traktor_b",
        desc: "Traktor Scratch, side B",
        resolution: 2000,
        bits: 23,
        seed: 0x32_066c,
        taps: 0x04_1040,
        length: 2_110_000,
        safe: 2_090_000,
        flags: SWITCH_PRIMARY | SWITCH_POLARITY | SWITCH_PHASE,
    },
    TimecodeDef {
        name: "mixvibes_v2",
        desc: "MixVibes V2, side A",
        resolution: 1300,
        bits: 20,
        seed: 0x2_2c90,
        taps: 0x0_0008,
        length: 950_000,
        safe: 923_000,
        flags: SWITCH_PHASE,
    },
    TimecodeDef {
        name: "mixvibes_7inch",
        desc: "MixVibes 7-inch",
        resolution: 1300,
        bits: 20,
        seed: 0x2_2c90,
        taps: 0x0_0008,
        length: 312_000,
        safe: 310_000,
        flags: SWITCH_PHASE,
    },
    TimecodeDef {
        name: "pioneer_a",
        desc: "Pioneer RekordBox DVS, side A",
        resolution: 1000,
        bits: 20,
        seed: 0x7_8370,
        taps: 0x7_933a,
        length: 635_000,
        safe: 614_000,
        flags: SWITCH_POLARITY,
    },
    TimecodeDef {
        name: "pioneer_b",
        desc: "Pioneer RekordBox DVS, side B",
        resolution: 1000,
        bits: 20,
        seed: 0xf_7012,
        taps: 0x2_ef1c,
        length: 918_500,
        safe: 913_000,
        flags: SWITCH_POLARITY,
    },
];

/// Look up a def by its xwax `name`.
#[must_use]
pub fn find_def(name: &str) -> Option<&'static TimecodeDef> {
    TIMECODE_DEFS.iter().find(|d| d.name == name)
}

/// The Serato pressings to auto-detect across (both vinyl sides + CD).
/// A capture decodes against exactly one; the tracker locks whichever's
/// LFSR validates.
///
/// # Panics
/// Never in practice — the three names are compile-time entries of
/// [`TIMECODE_DEFS`]; the `unwrap`s would only fire if that table were
/// edited inconsistently, which the `names_are_unique` test guards.
#[must_use]
pub fn serato_candidates() -> [&'static TimecodeDef; 3] {
    [
        find_def("serato_2a").unwrap(),
        find_def("serato_2b").unwrap(),
        find_def("serato_cd").unwrap(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_defs_are_well_formed() {
        for d in TIMECODE_DEFS {
            assert!(
                d.bits >= 16 && d.bits <= 24,
                "{}: odd bits {}",
                d.name,
                d.bits
            );
            assert!(
                d.seed != 0,
                "{}: zero seed can't seed an m-sequence",
                d.name
            );
            assert!(d.taps != 0, "{}: zero taps", d.name);
            assert!(d.safe <= d.length, "{}: safe > length", d.name);
            assert!(d.resolution >= 500, "{}: implausible carrier", d.name);
            // The seed must fit in `bits`.
            assert!(
                d.seed < (1u32 << d.bits),
                "{}: seed wider than bits",
                d.name
            );
        }
    }

    #[test]
    fn names_are_unique() {
        for (i, a) in TIMECODE_DEFS.iter().enumerate() {
            for b in &TIMECODE_DEFS[i + 1..] {
                assert_ne!(a.name, b.name, "duplicate def name {}", a.name);
            }
        }
    }

    #[test]
    fn serato_variants_have_distinct_polynomials() {
        // The whole point of the CD-vs-vinyl finding: a capture only
        // decodes against the matching taps. They must differ.
        let s = serato_candidates();
        for (i, a) in s.iter().enumerate() {
            for b in &s[i + 1..] {
                assert_ne!(a.taps, b.taps, "{} and {} share taps", a.name, b.name);
            }
        }
    }

    #[test]
    fn flags_decode() {
        let t = find_def("traktor_a").unwrap();
        assert!(
            !t.primary_right(),
            "traktor has SWITCH_PRIMARY → left primary"
        );
        assert!(t.switch_polarity());
        assert!(t.switch_phase());
        let s = find_def("serato_cd").unwrap();
        assert!(s.primary_right());
        assert!(!s.switch_polarity());
        assert!(!s.switch_phase());
    }
}
