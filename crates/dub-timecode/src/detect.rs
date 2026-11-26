//! Which control record is on the platter, and at which speed.
//!
//! The session-start calibration hold already spins the record at the
//! pitch detent for several seconds, so that is where a deck learns its
//! vinyl (PRD §5.4). At the detent every record and speed sits on its
//! own carrier frequency:
//!
//! | Record        | 33⅓      | 45       |
//! |---------------|----------|----------|
//! | Serato CV02   | 1000 Hz  | 1350 Hz  |
//! | Traktor MK1   | 2000 Hz  | 2700 Hz  |
//! | Traktor MK2   | 2500 Hz  | 3375 Hz  |
//!
//! The closest pair, MK2 at 33⅓ and MK1 at 45, is 8 % apart, which a
//! pitch fader left off the detent can bridge. The position code
//! settles it: MK1 carries one the tracker locks onto within a second,
//! MK2 carries none we read. So a lock names the record outright and
//! the frequency only picks the speed; without a lock the frequency
//! picks both, and MK1 at 45 is ruled out.
//!
//! Detection is a calibration-time decision. During play a scratch can
//! put the carrier anywhere, so nothing here runs on a live deck.

use crate::Format;

/// How fast the platter turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatterSpeed {
    /// 33⅓ rpm, the speed every control record is cut for.
    Rpm33,
    /// 45 rpm: the carrier, and everything read from it, runs 1.35×.
    Rpm45,
}

impl PlatterSpeed {
    /// Groove speed relative to 33⅓ rpm. The decoder's rate and every
    /// groove position divide by this to come out in track time.
    #[must_use]
    pub fn factor(self) -> f64 {
        match self {
            Self::Rpm33 => 1.0,
            Self::Rpm45 => 45.0 / (100.0 / 3.0),
        }
    }

    /// The speed as a DJ says it: `"33"` or `"45"`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Rpm33 => "33",
            Self::Rpm45 => "45",
        }
    }
}

/// What calibration found on one needle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection {
    /// The control record.
    pub format: Format,
    /// The platter speed.
    pub speed: PlatterSpeed,
}

/// Furthest a carrier may sit from a candidate's nominal and still be
/// read as it: a ±8 % pitch fader plus a deck that runs a little off.
/// Further than that it is not a control record at either speed.
const MATCH_TOLERANCE: f64 = 0.10;

/// Every record at either speed.
const CANDIDATES: [Detection; 6] = {
    use Format::{SeratoCv02, TraktorMk1, TraktorMk2};
    use PlatterSpeed::{Rpm33, Rpm45};
    [
        Detection {
            format: SeratoCv02,
            speed: Rpm33,
        },
        Detection {
            format: SeratoCv02,
            speed: Rpm45,
        },
        Detection {
            format: TraktorMk1,
            speed: Rpm33,
        },
        Detection {
            format: TraktorMk1,
            speed: Rpm45,
        },
        Detection {
            format: TraktorMk2,
            speed: Rpm33,
        },
        Detection {
            format: TraktorMk2,
            speed: Rpm45,
        },
    ]
};

/// Name the record and speed from the carrier measured at the detent.
///
/// `carrier_hz` is the carrier's mean frequency over the measurement;
/// `locked` is the format whose position code locked during it, if
/// any. Returns `None` when no record and speed fits within
/// [`MATCH_TOLERANCE`]. Runs on the audio thread at the end of a
/// calibration: no allocation.
#[must_use]
pub fn detect(carrier_hz: f64, locked: Option<Format>) -> Option<Detection> {
    if !carrier_hz.is_finite() || carrier_hz <= 0.0 {
        return None;
    }
    CANDIDATES
        .into_iter()
        .filter(|d| match locked {
            Some(format) => d.format == format,
            None => !(d.format == Format::TraktorMk1 && d.speed == PlatterSpeed::Rpm45),
        })
        .map(|d| (d, (carrier_hz / nominal_hz(d)).ln().abs()))
        .filter(|&(_, off)| off <= (1.0 + MATCH_TOLERANCE).ln())
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(d, _)| d)
}

/// The carrier a record gives at the detent at this speed.
#[must_use]
pub fn nominal_hz(d: Detection) -> f64 {
    f64::from(d.format.carrier_hz()) * d.speed.factor()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Format::{SeratoCv02, TraktorMk1, TraktorMk2};
    use PlatterSpeed::{Rpm33, Rpm45};

    /// What `detect` returns for a named record: the assertions compare
    /// against it directly.
    #[allow(clippy::unnecessary_wraps)]
    fn found(format: Format, speed: PlatterSpeed) -> Option<Detection> {
        Some(Detection { format, speed })
    }

    #[test]
    fn every_record_at_either_speed_is_named_at_the_detent() {
        for format in [SeratoCv02, TraktorMk1, TraktorMk2] {
            for speed in [Rpm33, Rpm45] {
                let d = Detection { format, speed };
                let code = format.lfsr_taps().map(|_| format);
                assert_eq!(detect(nominal_hz(d), code), Some(d), "{d:?}");
            }
        }
    }

    #[test]
    fn a_pitch_fader_off_the_detent_still_reads_right() {
        for pitch in [-0.08, -0.03, 0.03, 0.08] {
            let at = |hz: f64| hz * (1.0 + pitch);
            assert_eq!(
                detect(at(1000.0), Some(SeratoCv02)),
                found(SeratoCv02, Rpm33)
            );
            assert_eq!(
                detect(at(1350.0), Some(SeratoCv02)),
                found(SeratoCv02, Rpm45)
            );
            assert_eq!(
                detect(at(2000.0), Some(TraktorMk1)),
                found(TraktorMk1, Rpm33)
            );
            assert_eq!(detect(at(3375.0), None), found(TraktorMk2, Rpm45));
        }
    }

    #[test]
    fn mk1_at_45_and_mk2_at_33_are_told_apart_by_the_position_code() {
        // MK1 at 45 pulled down 6 % sits nearer MK2's 2500 Hz than its
        // own 2700; MK2 at 33 pushed up 6 % sits nearer 2700. The lock
        // decides, not the frequency.
        assert_eq!(
            detect(2700.0 * 0.94, Some(TraktorMk1)),
            found(TraktorMk1, Rpm45)
        );
        assert_eq!(detect(2500.0 * 1.06, None), found(TraktorMk2, Rpm33));
    }

    #[test]
    fn a_tone_without_a_code_is_named_by_frequency() {
        // Phase, or a worn record whose code never locked.
        assert_eq!(detect(2500.2, None), found(TraktorMk2, Rpm33));
        assert_eq!(detect(1000.0, None), found(SeratoCv02, Rpm33));
        assert_eq!(detect(1350.0, None), found(SeratoCv02, Rpm45));
        assert_eq!(detect(2000.0, None), found(TraktorMk1, Rpm33));
    }

    #[test]
    fn nothing_near_a_control_record_is_not_named() {
        for hz in [0.0, -1.0, f64::NAN, 440.0, 1700.0, 5000.0] {
            assert_eq!(detect(hz, None), None, "{hz} Hz");
        }
        // A locked code but a carrier at neither speed: the measurement
        // is wrong, so name nothing rather than guess.
        assert_eq!(detect(1700.0, Some(SeratoCv02)), None);
    }

    #[test]
    fn fortyfive_is_one_point_three_five() {
        assert!((Rpm45.factor() - 1.35).abs() < 1e-12);
        assert!((Rpm33.factor() - 1.0).abs() < f64::EPSILON);
    }
}
