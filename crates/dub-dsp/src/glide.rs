//! A control value that glides to where it is set.
//!
//! The DUB FX channel's knobs arrive as steps — a drag sends about sixty
//! values a second — and a gain or a loop coefficient taken in steps is a
//! zipper: nine of the rack's knobs read 4–134× sharper on a drag than held
//! still (`dub-engine/tests/fx_by_ear.rs`). A [`Glide`] moves a fraction of
//! the way to its target each sample, a few milliseconds in all, which a
//! hand on the knob cannot tell from instant.

use crate::echo::one_pole_coeff;

/// Corner of the glide: a 4 ms time constant.
const KNOB_GLIDE_HZ: f32 = 40.0;

/// The per-sample coefficient of a knob's glide at `sample_rate`. Calls
/// `exp`: resolve it at construction.
pub(crate) fn knob_glide_coeff(sample_rate: f32) -> f32 {
    one_pole_coeff(KNOB_GLIDE_HZ, sample_rate)
}

/// A knob's value and where it is headed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Glide {
    value: f32,
    target: f32,
}

impl Glide {
    pub(crate) const fn new(value: f32) -> Self {
        Self {
            value,
            target: value,
        }
    }

    /// Aim at `v`. `snap` lands there at once: before any audio has run
    /// there is nothing to glide from.
    pub(crate) fn set(&mut self, v: f32, snap: bool) {
        self.target = v;
        if snap {
            self.value = v;
        }
    }

    pub(crate) fn snap(&mut self) {
        self.value = self.target;
    }

    pub(crate) fn target(self) -> f32 {
        self.target
    }

    /// One sample of the glide; the value now.
    #[inline]
    pub(crate) fn next(&mut self, coeff: f32) -> f32 {
        let d = self.target - self.value;
        self.value = if d.abs() < 1e-7 {
            self.target
        } else {
            self.value + coeff * d
        };
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glide_lands_within_milliseconds_and_never_steps() {
        let sr = 48_000.0;
        let c = knob_glide_coeff(sr);
        let mut g = Glide::new(0.0);
        g.set(1.0, false);
        let mut last = 0.0_f32;
        let mut biggest = 0.0_f32;
        for _ in 0..(sr * 0.03) as usize {
            let v = g.next(c);
            biggest = biggest.max(v - last);
            last = v;
        }
        assert!((last - 1.0).abs() < 1e-3, "not there after 30 ms: {last}");
        assert!(biggest < 0.03, "a step of {biggest} in one sample");
    }

    #[test]
    fn a_snap_lands_at_once() {
        let mut g = Glide::new(0.2);
        g.set(0.9, true);
        assert_eq!(g.next(0.001), 0.9);
        assert_eq!(g.target(), 0.9);
    }
}
