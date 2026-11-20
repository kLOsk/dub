//! The splice stretcher: play the record untouched, and choose where to
//! join it.
//!
//! Changing a record's length without its pitch means leaving pieces out or
//! playing pieces twice. Every stretcher does that; what differs is where
//! the joins fall. This one copies the record sample for sample and makes
//! a join only where it will not be heard:
//!
//! - **Right before a drum hit.** A hit masks what comes just before it, so
//!   the join can jump by any amount there — and it jumps by exactly what
//!   the tempo is owed, which puts the hit on its beat to within a
//!   millisecond. No hit is ever inside a join, so none is doubled or lost.
//! - **Where the record repeats itself.** Between hits a join jumps by one
//!   turn of the wave that is playing, found by comparing the record with
//!   itself, and waits for a good match until the debt forces it.
//!
//! Bass and the rest are joined separately. A 40 Hz note turns once in
//! 25 ms, so the bass has to jump that far or not at all, while a hi-hat
//! may not move by a tenth of that. The record is split with a filter that
//! adds back to the record exactly, and while both halves read the same
//! place the output is the record itself.
//!
//! ```text
//!   input ─┬─▶ low  ──▶ [copy · join] ─┐
//!          └─▶ rest ──▶ [copy · join] ─┴─▶ mid ──▶ [resample by r] ──▶ output
//! ```
//!
//! The resampler is the pitch control and reads at whole positions when
//! `pitch_scale == 1`, which is how the deck runs key lock: the record goes
//! in at its own rate and only its length changes.
//!
//! No allocation after `new`, no `unsafe`, bounded work per block.

use crate::TimeStretcher;

const MIN_RATIO: f64 = 0.25;
const MAX_RATIO: f64 = 4.0;

/// Running averages in a row ahead of the slow rate. Four are 50 dB down
/// where the slow rate would fold, at two additions a sample each.
const LOWPASS_STAGES: usize = 4;

/// A hit is a block of the record this many times louder than the blocks
/// before it. Measured on the treble-tilted signal, where a drum stands
/// well clear of a held note.
const HIT_RISE: f64 = 4.0;
/// Blocks the level is averaged over to judge a hit against.
const HIT_MEMORY: usize = 6;
/// Blocks after a hit in which a second one is not looked for.
const HIT_REST_BLOCKS: usize = 5;
/// The bass counts as struck by a hit when its peak rises this much.
const BASS_STRUCK_RISE: f32 = 1.6;
/// A hit hides a join made just before it when its loudest moment is this
/// many times the power of what came before. Before a softer hit the join
/// may stray further from the owed jump to find a place where the record
/// matches, since less is covering it.
const HIDES_RISE: f32 = 4.0;
/// The bass before a hit is out of the way when the hit's peak is this many
/// times its own. Under that, a bass note is being held through the hit
/// (a sub under a kick), and the join before the hit has to keep the note
/// in step — the record at least this alike across it — or not happen:
/// joined at whatever the tempo owed, a held sub dipped by decibels before
/// every kick.
const BASS_CLEAR_RISE: f32 = 4.0;
const LOW_HELD_MATCH: f32 = 0.6;
/// How many times further a join may stray before a soft hit.
const SOFT_HIT_REACH: usize = 3;

const MAX_HITS: usize = 48;

const LOW: usize = 0;
const REST: usize = 1;

/// How one band may be joined, in frames.
#[derive(Clone, Copy)]
struct BandRules {
    /// Debt to the tempo below which a band is left alone between hits.
    soft: f64,
    /// Debt at which a join between hits is made whatever the match.
    hard: f64,
    /// Largest debt a join may leave on the other side.
    over: f64,
    /// How alike the record has to be across a join between hits while
    /// there is still room to wait for better.
    match_first: f64,
    /// Length of the join made before a hit.
    hit_fade: usize,
    /// Length of a join between hits.
    gap_fade: usize,
    /// How far the join before a hit may stray from the owed jump to find a
    /// better match — and so how far a hit can land from its beat.
    hit_reach: usize,
    /// Smallest jump worth making before a hit.
    hit_least: usize,
    /// Smallest jump between hits.
    gap_least: usize,
    /// Stretch of record compared to find a matching jump.
    compare: usize,
    /// Every how many frames the comparison looks.
    stride: usize,
}

#[derive(Clone, Copy)]
struct Fade {
    /// Where the outgoing copy reads.
    old: usize,
    at: usize,
    len: usize,
    /// How alike the two copies are, 0–1: alike copies add by level, unlike
    /// ones by power, so neither a held note nor noise dips in the join.
    alike: f32,
}

#[derive(Clone, Copy)]
struct Band {
    read: usize,
    fade: Option<Fade>,
}

#[derive(Clone, Copy, PartialEq)]
enum Struck {
    Unknown,
    Yes,
    No,
}

#[derive(Clone, Copy)]
struct Hit {
    /// Frame since the last reset.
    at: u64,
    /// Whether it is loud enough over what came before to hide a join.
    loud: Struck,
    /// Whether the bass was struck too (a kick, a plucked note).
    bass: Struck,
    /// Whether the bass before it was quiet enough for the hit to hide a
    /// join in the bass outright.
    bass_clear: bool,
}

/// One running average over `len` frames of both channels.
struct Average {
    ring: Box<[[f64; 2]]>,
    next: usize,
    sum: [f64; 2],
    /// One over the length.
    scale: f64,
}

impl Average {
    fn new(len: usize) -> Self {
        Self {
            ring: vec![[0.0; 2]; len].into_boxed_slice(),
            next: 0,
            sum: [0.0; 2],
            #[allow(clippy::cast_precision_loss)]
            scale: 1.0 / len as f64,
        }
    }

    fn push(&mut self, frame: [f64; 2]) -> [f64; 2] {
        let leaving = self.ring[self.next];
        self.ring[self.next] = frame;
        self.next += 1;
        if self.next == self.ring.len() {
            self.next = 0;
        }
        self.sum[0] += frame[0] - leaving[0];
        self.sum[1] += frame[1] - leaving[1];
        [self.sum[0] * self.scale, self.sum[1] * self.scale]
    }

    fn clear(&mut self) {
        for frame in self.ring.iter_mut() {
            *frame = [0.0; 2];
        }
        self.next = 0;
        self.sum = [0.0; 2];
    }
}

/// The low-pass that splits bass from the rest: steep, and without
/// ringing either side of a hit differently.
///
/// What both halves carry is played twice a few milliseconds apart whenever
/// the halves read different places, and partly cancels. The first filter
/// here was a few running averages — cheap, and so gentle that everything
/// from 80 to 300 Hz was in both: records came out 0.3 to 1.7 dB short
/// below 170 Hz, and the DJ heard it. This one passes [`SPLIT_HZ`] and
/// under and is 60 dB down a third of an octave above, so the two share a
/// sliver.
///
/// A filter that steep at the record's own rate would be thousands of
/// taps a frame. Bass needs no such rate: the record is averaged down to a
/// sixteenth of it, filtered there with a few hundred taps every sixteenth
/// frame, and brought back up between those. Every stage is symmetric, so
/// the bass comes out exactly [`Lowpass::delay`] frames late and otherwise
/// in step with the record at every frequency.
struct Lowpass {
    /// Running averages that keep what is above the slow rate's reach out
    /// of it.
    stages: [Average; LOWPASS_STAGES],
    /// Frames until the next one kept for the slow rate.
    until_kept: usize,
    /// The last [`SLOW_TAPS`] frames kept, as a ring written twice over —
    /// at `kept_next` and [`SLOW_TAPS`] past it — so the newest
    /// [`SLOW_TAPS`] always lie in one unbroken run for the filter.
    kept: Box<[[f64; 2]]>,
    kept_next: usize,
    taps: Box<[f64]>,
    /// The last [`RISE_TAPS`] filtered frames at the slow rate, oldest first.
    slow: [[f64; 2]; RISE_TAPS],
    /// Weights that read `slow` at each of the [`SLOW_EVERY`] places
    /// between two slow frames.
    rise: Box<[[f64; RISE_TAPS]]>,
}

/// Bass is this and under, in Hz. Above where bass notes are played: the
/// filter's slope is a few semitones wide, a note on it is in both halves,
/// and at 110 Hz that was the G to B a funk bass line lives on — a record
/// came out a decibel short there. At 150 Hz the slope sits between the
/// bass guitar's notes and the snare's body.
const SPLIT_HZ: f64 = 150.0;
/// The slow rate is the record's over this.
const SLOW_EVERY: usize = 16;
/// Taps of the filter at the slow rate; odd, so its delay is whole.
const SLOW_TAPS: usize = 511;
/// Slow frames read to bring the bass back to the record's rate.
const RISE_TAPS: usize = 8;

/// Bessel's `I0`, for the Kaiser window.
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0f64, 1.0f64);
    for k in 1..40 {
        term *= (x / 2.0 / f64::from(k)).powi(2);
        sum += term;
    }
    sum
}

/// A Kaiser window at `at`, −1 to 1 across it.
fn kaiser(at: f64, beta: f64) -> f64 {
    if at.abs() >= 1.0 {
        return 0.0;
    }
    bessel_i0(beta * (1.0 - at * at).sqrt()) / bessel_i0(beta)
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
    }
}

impl Lowpass {
    /// Length of each running average: odd, and a little over the slow
    /// rate's step, so its nulls fall where the slow rate would fold.
    const AVERAGE_LEN: usize = 2 * SLOW_EVERY + 1;

    fn new(sample_rate: f64) -> Self {
        #[allow(clippy::cast_precision_loss)]
        let slow_rate = sample_rate / SLOW_EVERY as f64;
        let turn = 2.0 * SPLIT_HZ / slow_rate;
        // Two taps short of the whole: the correction below adds them.
        #[allow(clippy::cast_precision_loss)]
        let middle = (SLOW_TAPS - 3) as f64 / 2.0;
        #[allow(clippy::cast_precision_loss)]
        let plain: Vec<f64> = (0..SLOW_TAPS - 2)
            .map(|k| {
                let from_middle = k as f64 - middle;
                turn * sinc(turn * from_middle) * kaiser(from_middle / (middle + 1.0), 5.65)
            })
            .collect();
        // The running averages droop a little across the bass — a percent
        // at 60 Hz — which would leave that much of every bass note in the
        // rest. A three-tap lift of the same shape, upside down, undoes it:
        // four averages of length `B` fall as `1 − (2/3)(B·w / 2·16)²` at
        // slow-rate angle `w`, and `[−a, 1 + 2a, −a]` rises as `1 + a·w²`.
        #[allow(clippy::cast_precision_loss)]
        let lift = 2.0 / 3.0 * (Self::AVERAGE_LEN as f64 / (2.0 * SLOW_EVERY as f64)).powi(2);
        let mut taps = vec![0.0f64; SLOW_TAPS];
        for (k, tap) in plain.iter().enumerate() {
            taps[k] -= lift * tap;
            taps[k + 1] += (1.0 + 2.0 * lift) * tap;
            taps[k + 2] -= lift * tap;
        }
        let sum: f64 = taps.iter().sum();
        for tap in &mut taps {
            *tap /= sum;
        }

        // Place `p` of the way from slow frame 3 to slow frame 4 of the
        // eight: a windowed sinc, each row made to add to one so a held
        // level comes back as it went in.
        #[allow(clippy::cast_precision_loss)]
        let rise: Vec<[f64; RISE_TAPS]> = (0..SLOW_EVERY)
            .map(|p| {
                let frac = p as f64 / SLOW_EVERY as f64;
                let mut row = [0.0f64; RISE_TAPS];
                for (j, weight) in row.iter_mut().enumerate() {
                    let from = j as f64 - 3.0 - frac;
                    *weight = sinc(from) * kaiser(from / (RISE_TAPS as f64 / 2.0), 6.0);
                }
                let sum: f64 = row.iter().sum();
                row.map(|w| w / sum)
            })
            .collect();

        Self {
            stages: std::array::from_fn(|_| Average::new(Self::AVERAGE_LEN)),
            until_kept: 0,
            kept: vec![[0.0; 2]; 2 * SLOW_TAPS].into_boxed_slice(),
            kept_next: 0,
            taps: taps.into_boxed_slice(),
            slow: [[0.0; 2]; RISE_TAPS],
            rise: rise.into_boxed_slice(),
        }
    }

    /// Frames by which the bass that comes out trails the frame that went
    /// in: the averages, the slow filter's middle, and the four slow
    /// frames ahead that bringing it back up reads.
    fn delay() -> usize {
        LOWPASS_STAGES * (Self::AVERAGE_LEN - 1) / 2
            + ((SLOW_TAPS - 1) / 2 + RISE_TAPS / 2) * SLOW_EVERY
    }

    fn push(&mut self, frame: [f64; 2]) -> [f64; 2] {
        let mut averaged = frame;
        for average in &mut self.stages {
            averaged = average.push(averaged);
        }
        let place = if self.until_kept == 0 {
            self.kept[self.kept_next] = averaged;
            self.kept[self.kept_next + SLOW_TAPS] = averaged;
            self.kept_next = (self.kept_next + 1) % SLOW_TAPS;
            // Oldest first from `kept_next`; the taps read the same either
            // way round.
            let mut sum = [0.0f64; 2];
            let run = &self.kept[self.kept_next..self.kept_next + SLOW_TAPS];
            for (tap, frame) in self.taps.iter().zip(run) {
                sum[0] += tap * frame[0];
                sum[1] += tap * frame[1];
            }
            self.slow.rotate_left(1);
            self.slow[RISE_TAPS - 1] = sum;
            self.until_kept = SLOW_EVERY - 1;
            0
        } else {
            self.until_kept -= 1;
            SLOW_EVERY - 1 - self.until_kept
        };
        let mut out = [0.0f64; 2];
        for (slow, weight) in self.slow.iter().zip(&self.rise[place]) {
            out[0] += weight * slow[0];
            out[1] += weight * slow[1];
        }
        out
    }

    fn clear(&mut self) {
        for average in &mut self.stages {
            average.clear();
        }
        for frame in self.kept.iter_mut() {
            *frame = [0.0; 2];
        }
        self.until_kept = 0;
        self.kept_next = 0;
        self.slow = [[0.0; 2]; RISE_TAPS];
    }
}

/// How alike two stretches of `signal` are, −1 to 1. Two silences are alike.
fn alike(signal: &[f32], a: usize, b: usize, len: usize, stride: usize) -> f32 {
    let (mut dot, mut ea, mut eb) = (0.0f64, 0.0f64, 0.0f64);
    let mut k = 0;
    while k < len {
        let (p, q) = (f64::from(signal[a + k]), f64::from(signal[b + k]));
        dot += p * q;
        ea += p * p;
        eb += q * q;
        k += stride;
    }
    if ea < 1e-10 && eb < 1e-10 {
        return 1.0;
    }
    #[allow(clippy::cast_possible_truncation)]
    {
        (dot / (ea * eb).sqrt().max(1e-12)) as f32
    }
}

/// How many joins a stretcher has made since its last reset, by kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Joins {
    /// Hidden right before a drum hit.
    pub before_hits: u32,
    /// Between hits, where the record matched itself.
    pub matched: u32,
    /// Between hits, made because the debt to the tempo ran out of room
    /// before a good match came. These are the ones that can be heard.
    pub forced: u32,
}

/// Pure-Rust splice stretcher. See the module docs.
pub struct SpliceStretcher {
    // --- fixed at `new` ---
    rules: [BandRules; 2],
    /// Frames rendered between looks at whether to join.
    tick: usize,
    /// Frames a join is kept clear of a hit on either side.
    guard: usize,
    /// Delay of the band-splitting filter.
    split_delay: usize,
    /// Record kept ahead of the furthest reader.
    lookahead: usize,
    /// Record kept behind the nearest reader.
    history: usize,
    /// Frames of the level blocks hits are found in.
    hit_block: usize,
    bass_before: (usize, usize),
    bass_after: usize,
    /// Stretch of the rest band a hit's loudness is judged over.
    hide_span: usize,
    cap: usize,

    // --- live controls ---
    time_ratio: f64,
    pitch_scale: f64,

    // --- the record, as far as it has been fed ---
    x: Box<[f32]>,
    low: Box<[f32]>,
    low_mono: Box<[f32]>,
    rest_mono: Box<[f32]>,
    fill: usize,
    /// Frames of `low`, `low_mono` and `rest_mono` worked out so far.
    split_fill: usize,
    /// Frames the filter has still to swallow before its first output.
    split_warm: usize,
    lowpass: Lowpass,
    /// Frames dropped from the front of the buffers since the last reset.
    base: u64,

    // --- hits ---
    hits: [Hit; MAX_HITS],
    hit_count: usize,
    block_level: f64,
    block_frames: usize,
    block_last: f32,
    recent_levels: [f64; HIT_MEMORY],
    blocks_since_hit: usize,

    // --- reading ---
    /// Where the tempo says the record should be read, in buffer frames.
    nominal: f64,
    bands: [Band; 2],

    // --- output of the joins, feeding the resampler ---
    mid: Box<[f32]>,
    mid_len: usize,
    rs_pos: f64,
    joins: Joins,
}

impl SpliceStretcher {
    /// Construct a stretcher for `sample_rate` Hz at unity ratio. Allocates
    /// all working buffers up front; never allocates again.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8_000.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ms = |x: f32| ((sr * x / 1_000.0).round() as usize).max(1);
        let ms_f = |x: f32| f64::from(sr * x / 1_000.0);

        let split_delay = Lowpass::delay();

        let rules = [
            BandRules {
                soft: ms_f(6.0),
                hard: ms_f(16.0),
                over: ms_f(20.0),
                match_first: 0.995,
                hit_fade: ms(8.0),
                gap_fade: ms(20.0),
                hit_reach: ms(3.0),
                hit_least: ms(0.5),
                gap_least: ms(4.0),
                compare: ms(28.0),
                stride: (ms(1.0) / 3).max(1),
            },
            BandRules {
                soft: ms_f(3.0),
                hard: ms_f(10.0),
                over: ms_f(10.0),
                match_first: 0.95,
                hit_fade: ms(4.0),
                gap_fade: ms(6.0),
                hit_reach: ms(1.0),
                hit_least: ms(0.25),
                gap_least: ms(0.5),
                compare: ms(8.0),
                stride: (ms(1.0) / 12).max(1),
            },
        ];
        let lookahead = ms(72.0);
        let history = ms(40.0);
        let tick = ms(2.0);
        let cap = (history + lookahead + ms(60.0) + split_delay + 4 * tick).next_power_of_two();
        let cap_mid = 4 * tick + 4;

        Self {
            rules,
            tick,
            guard: ms(3.0),
            split_delay,
            lookahead,
            history,
            hit_block: ms(2.0),
            bass_before: (ms(30.0), ms(4.0)),
            bass_after: ms(24.0),
            hide_span: ms(6.0),
            cap,
            time_ratio: 1.0,
            pitch_scale: 1.0,
            x: vec![0.0; cap * 2].into_boxed_slice(),
            low: vec![0.0; cap * 2].into_boxed_slice(),
            low_mono: vec![0.0; cap].into_boxed_slice(),
            rest_mono: vec![0.0; cap].into_boxed_slice(),
            fill: 0,
            split_fill: 0,
            split_warm: split_delay,
            lowpass: Lowpass::new(f64::from(sr)),
            base: 0,
            hits: [Hit {
                at: 0,
                loud: Struck::No,
                bass: Struck::No,
                bass_clear: false,
            }; MAX_HITS],
            hit_count: 0,
            block_level: 0.0,
            block_frames: 0,
            block_last: 0.0,
            recent_levels: [0.0; HIT_MEMORY],
            blocks_since_hit: HIT_REST_BLOCKS,
            nominal: 0.0,
            bands: [Band {
                read: 0,
                fade: None,
            }; 2],
            mid: vec![0.0; cap_mid * 2].into_boxed_slice(),
            mid_len: 0,
            rs_pos: 0.0,
            joins: Joins::default(),
        }
    }

    /// Tell the stretcher the record is `frames` further on than the tempo
    /// it was given says (negative: further back). A timecode deck corrects
    /// its playhead to the groove every block; this carries the correction
    /// to the audio, which repays it at the next join. The record fed in
    /// is not touched.
    pub fn owe(&mut self, frames: f64) {
        if frames.is_finite() {
            self.nominal += frames;
        }
    }

    /// Joins made since the last reset.
    #[must_use]
    pub fn joins(&self) -> Joins {
        self.joins
    }

    fn stretch(&self) -> f64 {
        (self.time_ratio * self.pitch_scale).clamp(MIN_RATIO, MAX_RATIO)
    }

    /// The furthest and nearest places any copy is reading.
    fn readers(&self) -> (usize, usize) {
        let (mut near, mut far) = (usize::MAX, 0);
        for band in &self.bands {
            near = near.min(band.read);
            far = far.max(band.read);
            if let Some(fade) = band.fade {
                near = near.min(fade.old);
                far = far.max(fade.old);
            }
        }
        (near, far)
    }

    /// Take up to free-space frames from `input[cursor..]`, splitting each
    /// into bass and rest and listening for hits. Returns frames taken.
    fn append(&mut self, input: &[f32], cursor: &mut usize, in_total: usize) -> usize {
        if self.fill == self.cap {
            self.compact();
        }
        let n = (self.cap - self.fill).min(in_total - *cursor);
        for k in 0..n {
            let (l, r) = (input[(*cursor + k) * 2], input[(*cursor + k) * 2 + 1]);
            self.x[self.fill * 2] = l;
            self.x[self.fill * 2 + 1] = r;
            self.fill += 1;

            let frame = self.lowpass.push([f64::from(l), f64::from(r)]);
            if self.split_warm > 0 {
                self.split_warm -= 1;
                continue;
            }
            let i = self.split_fill;
            #[allow(clippy::cast_possible_truncation)]
            let (low_l, low_r) = (frame[0] as f32, frame[1] as f32);
            self.low[i * 2] = low_l;
            self.low[i * 2 + 1] = low_r;
            self.low_mono[i] = low_l + low_r;
            let rest = self.x[i * 2] + self.x[i * 2 + 1] - low_l - low_r;
            self.rest_mono[i] = rest;
            self.split_fill += 1;
            self.listen_for_hit(rest, i);
        }
        *cursor += n;
        n
    }

    /// Add one frame of the rest band to the level blocks; a block well above
    /// the ones before it is a hit starting where the block starts.
    fn listen_for_hit(&mut self, rest: f32, at: usize) {
        // The difference between neighbours tilts the level towards treble,
        // where a drum's attack is and a held note is not.
        let edge = f64::from(rest - self.block_last);
        self.block_last = rest;
        self.block_level += edge * edge;
        self.block_frames += 1;
        if self.block_frames < self.hit_block {
            return;
        }
        let level = self.block_level;
        self.block_level = 0.0;
        self.block_frames = 0;
        #[allow(clippy::cast_precision_loss)]
        let usual = self.recent_levels.iter().sum::<f64>() / HIT_MEMORY as f64;
        #[allow(clippy::cast_precision_loss)]
        let floor = self.hit_block as f64 * 1e-7;
        let is_hit = level > HIT_RISE * usual + floor && self.blocks_since_hit >= HIT_REST_BLOCKS;
        self.recent_levels.rotate_left(1);
        self.recent_levels[HIT_MEMORY - 1] = level;
        if !is_hit {
            self.blocks_since_hit += 1;
            return;
        }
        self.blocks_since_hit = 0;
        if self.hit_count == MAX_HITS {
            self.hits.copy_within(1.., 0);
            self.hit_count -= 1;
        }
        self.hits[self.hit_count] = Hit {
            at: (self.base + at as u64 + 1).saturating_sub(self.hit_block as u64),
            loud: Struck::Unknown,
            bass: Struck::Unknown,
            bass_clear: false,
        };
        self.hit_count += 1;
    }

    /// A hit's place in the buffers; negative once it has been dropped.
    fn hit_place(&self, hit: &Hit) -> isize {
        #[allow(clippy::cast_possible_wrap)]
        {
            hit.at as isize - self.base as isize
        }
    }

    /// Decide, for hits whose surroundings have arrived, whether they are
    /// loud enough to hide a join and whether the bass was struck with them.
    fn judge_hits(&mut self) {
        for k in 0..self.hit_count {
            if self.hits[k].bass != Struck::Unknown {
                continue;
            }
            let at = self.hit_place(&self.hits[k]);
            #[allow(clippy::cast_possible_wrap)]
            if at < 0 || at + self.bass_after as isize > self.split_fill as isize {
                continue;
            }
            let at = at.unsigned_abs();
            let peak = |from: usize, to: usize| -> f32 {
                self.low_mono[from..to]
                    .iter()
                    .fold(0.0f32, |m, s| m.max(s.abs()))
            };
            let before = peak(
                at.saturating_sub(self.bass_before.0),
                at.saturating_sub(self.bass_before.1),
            );
            let after = peak(at, at + self.bass_after);
            self.hits[k].bass_clear = after > BASS_CLEAR_RISE * before;
            self.hits[k].bass = if after > BASS_STRUCK_RISE * before && after > 1e-3 {
                Struck::Yes
            } else {
                Struck::No
            };
            let power = |from: usize, to: usize| -> f32 {
                self.rest_mono[from..to].iter().map(|s| s * s).sum()
            };
            let span = self.hide_span;
            let quiet = power(
                at.saturating_sub(span + self.guard),
                at.saturating_sub(self.guard),
            );
            // The loudest third of the stretch after the hit, scaled to the
            // whole stretch: a drum is over in a couple of milliseconds.
            let third = span / 3;
            let loudest = (0..3)
                .map(|n| power(at + n * third, at + (n + 1) * third))
                .fold(0.0f32, f32::max)
                * 3.0;
            self.hits[k].loud = if loudest > HIDES_RISE * quiet {
                Struck::Yes
            } else {
                Struck::No
            };
        }
    }

    /// Whether `band` must keep its joins off `hit`.
    fn concerns(band: usize, hit: &Hit) -> bool {
        band == REST || hit.bass != Struck::No
    }

    /// The first hit this band joins before at or after `from`, and whether
    /// it is loud enough to hide the join outright.
    fn next_hit(&self, band: usize, from: usize) -> Option<(usize, bool)> {
        #[allow(clippy::cast_possible_wrap)]
        let from = from as isize;
        self.hits[..self.hit_count]
            .iter()
            .filter(|hit| Self::concerns(band, hit))
            .map(|hit| {
                let hides = if band == LOW {
                    hit.bass_clear
                } else {
                    hit.loud != Struck::No
                };
                (self.hit_place(hit), hides)
            })
            .find(|&(at, _)| at >= from)
            .map(|(at, loud)| (at.unsigned_abs(), loud))
    }

    /// Whether no hit this band respects lies in `from..to`.
    fn clear_of_hits(&self, band: usize, from: isize, to: isize) -> bool {
        !self.hits[..self.hit_count]
            .iter()
            .filter(|hit| Self::concerns(band, hit))
            .map(|hit| self.hit_place(hit))
            .any(|at| at >= from && at < to)
    }

    /// The best jump for `band` among `least..=most` (by `step`), judged by
    /// how alike the record is across it, less `pull` for each frame it
    /// leaves the debt away from `aim`. Jumps that would put a hit inside
    /// the join, or twice or never in the output, are passed over.
    #[allow(clippy::too_many_arguments)]
    fn best_jump(
        &self,
        band: usize,
        least: isize,
        most: isize,
        step: usize,
        fade: usize,
        end_by: isize,
        compare: usize,
        aim: f64,
        pull: f64,
    ) -> Option<(isize, f32)> {
        let rules = &self.rules[band];
        let signal = if band == LOW {
            &self.low_mono
        } else {
            &self.rest_mono
        };
        #[allow(clippy::cast_possible_wrap)]
        let read = self.bands[band].read as isize;
        let reach = compare.max(fade + self.tick);
        #[allow(clippy::cast_possible_wrap)]
        let guard = self.guard as isize;
        let score_at = |jump: isize| -> Option<(f64, f32)> {
            let to = read + jump;
            #[allow(clippy::cast_possible_wrap)]
            if jump == 0 || to < 0 || to + reach as isize > self.split_fill as isize {
                return None;
            }
            #[allow(clippy::cast_possible_wrap)]
            let span_end = read.max(to) + fade as isize + guard;
            if span_end > end_by || !self.clear_of_hits(band, read.min(to) - guard, span_end) {
                return None;
            }
            let match_ = alike(
                signal,
                read.unsigned_abs(),
                to.unsigned_abs(),
                compare,
                rules.stride,
            );
            #[allow(clippy::cast_precision_loss)]
            let off = (jump as f64 - aim).abs();
            Some((f64::from(match_) - pull * off, match_))
        };

        let mut best: Option<(isize, f64, f32)> = None;
        let consider = |jump: isize, best: &mut Option<(isize, f64, f32)>| {
            if let Some((score, match_)) = score_at(jump) {
                if best.is_none_or(|(_, s, _)| score > s) {
                    *best = Some((jump, score, match_));
                }
            }
        };
        let mut jump = least;
        while jump <= most {
            consider(jump, &mut best);
            #[allow(clippy::cast_possible_wrap)]
            {
                jump += step as isize;
            }
        }
        if step > 1 {
            if let Some((coarse, _, _)) = best {
                #[allow(clippy::cast_possible_wrap)]
                let around = step as isize - 1;
                for jump in (coarse - around).max(least)..=(coarse + around).min(most) {
                    consider(jump, &mut best);
                }
            }
        }
        best.map(|(jump, _, match_)| (jump, match_))
    }

    fn join(&mut self, band: usize, jump: isize, len: usize, alike: f32) {
        let read = self.bands[band].read;
        self.bands[band].fade = Some(Fade {
            old: read,
            at: 0,
            len,
            alike: alike.clamp(0.0, 1.0),
        });
        self.bands[band].read = read.saturating_add_signed(jump);
    }

    /// Look at one band's debt to the tempo and start a join if now is the
    /// moment for one.
    fn plan(&mut self, band: usize) {
        if self.bands[band].fade.is_some() {
            return;
        }
        let rules = self.rules[band];
        let read = self.bands[band].read;
        #[allow(clippy::cast_precision_loss)]
        let debt = read as f64 - self.nominal;
        // Frames of debt added by every frame played.
        let drift = 1.0 - 1.0 / self.stretch();

        if debt.abs() > 2.0 * rules.hard {
            // Past anything a clean join can repay (a ratio far outside a
            // DJ's range): cut back to the tempo.
            #[allow(clippy::cast_possible_truncation)]
            let jump = (-debt).round() as isize;
            #[allow(clippy::cast_possible_wrap)]
            let room = self.split_fill as isize - (rules.hit_fade + self.tick) as isize;
            #[allow(clippy::cast_possible_wrap)]
            let jump = jump.clamp(-(read as isize), room - read as isize);
            self.join(band, jump, rules.hit_fade, 0.0);
            self.joins.forced += 1;
            return;
        }

        #[allow(clippy::cast_possible_wrap)]
        let (read_i, tick) = (read as isize, self.tick as isize);
        if let Some((hit, loud)) = self.next_hit(band, read + self.guard) {
            #[allow(clippy::cast_precision_loss)]
            let owed = -(debt + drift * (rules.hit_fade + self.guard) as f64);
            #[allow(clippy::cast_possible_truncation)]
            let jump = owed.round() as isize;
            // The last place a join can start and still be over, on both
            // copies, before the hit.
            #[allow(clippy::cast_possible_wrap)]
            let start_by = hit as isize - (self.guard + rules.hit_fade) as isize - jump.max(0);
            // Debt moves `start_by` a little every tick; the slack keeps a
            // tick from stepping over it.
            #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            let slack = (drift.abs() * tick as f64).ceil() as isize + 1;
            if read_i <= start_by {
                if read_i + tick + slack > start_by {
                    if jump.unsigned_abs() >= rules.hit_least {
                        #[allow(clippy::cast_possible_wrap)]
                        let reach = if loud {
                            rules.hit_reach
                        } else {
                            rules.hit_reach * SOFT_HIT_REACH
                        } as isize;
                        // Only jumps that still end before the hit.
                        let most = (jump + reach).min(jump.max(0) + (start_by - read_i));
                        // The hit hides the join, so the jump stays close to
                        // what is owed: a better match has to be clearly
                        // better to move the hit off its beat.
                        #[allow(clippy::cast_precision_loss)]
                        let pull = 0.3 / reach as f64;
                        if let Some((jump, match_)) = self.best_jump(
                            band,
                            jump - reach,
                            most,
                            1,
                            rules.hit_fade,
                            isize::MAX,
                            rules.hit_fade,
                            owed,
                            pull,
                        ) {
                            // A bass note held through the hit is not
                            // hidden by it: join it only where it runs on
                            // in step, or leave the debt for a join that
                            // does.
                            if band == LOW && !loud && match_ < LOW_HELD_MATCH {
                                return;
                            }
                            self.join(band, jump, rules.hit_fade, match_);
                            self.joins.before_hits += 1;
                        }
                    }
                    return;
                }
                // The join before the hit will repay the debt in time.
                #[allow(clippy::cast_precision_loss)]
                let debt_then = debt + drift * (start_by - read_i) as f64;
                if debt_then.abs() < rules.hard {
                    return;
                }
            }
        }

        // Between hits: only when the debt is growing, and big enough to
        // be worth a join.
        if debt * drift <= 0.0 || debt.abs() < rules.soft {
            return;
        }
        let urgency = (debt.abs() - rules.soft) / (rules.hard - rules.soft);
        let toward = if debt > 0.0 { -1.0 } else { 1.0 };
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let (least, most) = (rules.gap_least as isize, (debt.abs() + rules.over) as isize);
        let (from, to) = if toward > 0.0 {
            (least, most)
        } else {
            (-most, -least)
        };
        // Leave the join before the next loud hit room to happen.
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let end_by = self
            .next_hit(band, read + self.guard)
            .map_or(isize::MAX, |(hit, _)| {
                hit as isize - (rules.hit_fade + 2 * self.tick) as isize - rules.over as isize
            });
        // Aim a little past even, so the next join is as far off as it can be.
        let aim = -debt + toward * rules.soft;
        let found = self.best_jump(
            band,
            from,
            to,
            rules.stride,
            rules.gap_fade,
            end_by,
            rules.compare,
            aim,
            0.1 / rules.hard,
        );
        if let Some((jump, match_)) = found {
            // Hold out for a near-perfect match while there is room, and
            // settle for less only as the room runs out.
            let good = f64::from(match_) >= rules.match_first - 0.6 * urgency.powi(3);
            if good || urgency >= 1.0 {
                if good {
                    self.joins.matched += 1;
                } else {
                    self.joins.forced += 1;
                }
                self.join(band, jump, rules.gap_fade, match_);
            }
        }
    }

    /// One frame of `band` at buffer frame `i`.
    fn band_at(&self, band: usize, i: usize) -> (f32, f32) {
        let (l, r) = (self.low[i * 2], self.low[i * 2 + 1]);
        if band == LOW {
            (l, r)
        } else {
            (self.x[i * 2] - l, self.x[i * 2 + 1] - r)
        }
    }

    fn band_frame(&self, band: usize) -> (f32, f32) {
        let state = &self.bands[band];
        let (nl, nr) = self.band_at(band, state.read);
        let Some(fade) = state.fade else {
            return (nl, nr);
        };
        #[allow(clippy::cast_precision_loss)]
        let t = (fade.at as f32 + 0.5) / fade.len as f32;
        let rising = 0.5 - 0.5 * (std::f32::consts::PI * t).cos();
        let falling = 1.0 - rising;
        let level = (rising * rising + falling * falling + 2.0 * fade.alike * rising * falling)
            .sqrt()
            .max(1e-6);
        let (ol, or) = self.band_at(band, fade.old);
        (
            (ol * falling + nl * rising) / level,
            (or * falling + nr * rising) / level,
        )
    }

    /// Render one tick into `mid`.
    fn render(&mut self) {
        let step = 1.0 / self.stretch();
        let last = self.split_fill - 1;
        for _ in 0..self.tick {
            let together = self.bands[LOW].fade.is_none()
                && self.bands[REST].fade.is_none()
                && self.bands[LOW].read == self.bands[REST].read;
            let (l, r) = if together {
                let i = self.bands[LOW].read;
                (self.x[i * 2], self.x[i * 2 + 1])
            } else {
                let (ll, lr) = self.band_frame(LOW);
                let (rl, rr) = self.band_frame(REST);
                (ll + rl, lr + rr)
            };
            self.mid[self.mid_len * 2] = l;
            self.mid[self.mid_len * 2 + 1] = r;
            self.mid_len += 1;
            for band in &mut self.bands {
                band.read = (band.read + 1).min(last);
                if let Some(fade) = &mut band.fade {
                    fade.old = (fade.old + 1).min(last);
                    fade.at += 1;
                    if fade.at >= fade.len {
                        band.fade = None;
                    }
                }
            }
            self.nominal += step;
        }
    }

    /// Render one tick, taking more input first if the readers are short of
    /// lookahead. Returns `false` only when starved.
    fn step(&mut self, input: &[f32], cursor: &mut usize, in_total: usize) -> bool {
        loop {
            let (_, far) = self.readers();
            if self.split_fill >= far + self.lookahead + self.tick {
                break;
            }
            if self.append(input, cursor, in_total) == 0 {
                return false;
            }
        }
        self.judge_hits();
        self.plan(LOW);
        self.plan(REST);
        self.render();
        true
    }

    /// Drop the record no reader can go back to.
    fn compact(&mut self) {
        let (near, _) = self.readers();
        let drop = near.saturating_sub(self.history);
        if drop == 0 {
            return;
        }
        self.x.copy_within(drop * 2..self.fill * 2, 0);
        self.low.copy_within(drop * 2..self.split_fill * 2, 0);
        self.low_mono.copy_within(drop..self.split_fill, 0);
        self.rest_mono.copy_within(drop..self.split_fill, 0);
        self.fill -= drop;
        self.split_fill -= drop;
        #[allow(clippy::cast_precision_loss)]
        {
            self.nominal -= drop as f64;
        }
        for band in &mut self.bands {
            band.read -= drop;
            if let Some(fade) = &mut band.fade {
                fade.old -= drop;
            }
        }
        self.base += drop as u64;
        let gone = self.hits[..self.hit_count]
            .iter()
            .take_while(|hit| hit.at < self.base)
            .count();
        self.hits.copy_within(gone..self.hit_count, 0);
        self.hit_count -= gone;
    }

    fn compact_mid(&mut self) {
        let drop = self.rs_pos.floor();
        if drop < 1.0 {
            return;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let drop = (drop as usize).min(self.mid_len);
        self.mid.copy_within(drop * 2..self.mid_len * 2, 0);
        self.mid_len -= drop;
        #[allow(clippy::cast_precision_loss)]
        {
            self.rs_pos -= drop as f64;
        }
    }
}

impl TimeStretcher for SpliceStretcher {
    fn set_pitch_scale(&mut self, scale: f64) {
        if scale.is_finite() {
            self.pitch_scale = scale.clamp(MIN_RATIO, MAX_RATIO);
        }
    }

    fn set_time_ratio(&mut self, ratio: f64) {
        if ratio.is_finite() {
            self.time_ratio = ratio.clamp(MIN_RATIO, MAX_RATIO);
        }
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> (usize, usize) {
        let in_total = input.len() / 2;
        let out_cap = output.len() / 2;
        let r = self.pitch_scale.clamp(MIN_RATIO, MAX_RATIO);
        let mut cursor = 0usize;
        let mut produced = 0usize;

        while produced < out_cap {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            while self.rs_pos.floor() as usize + 1 >= self.mid_len {
                if !self.step(input, &mut cursor, in_total) {
                    return (cursor, produced);
                }
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let i0 = self.rs_pos.floor() as usize;
            #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            let frac = (self.rs_pos - i0 as f64) as f32;
            let a0 = i0 * 2;
            output[produced * 2] = self.mid[a0] + (self.mid[a0 + 2] - self.mid[a0]) * frac;
            output[produced * 2 + 1] =
                self.mid[a0 + 1] + (self.mid[a0 + 3] - self.mid[a0 + 1]) * frac;
            produced += 1;
            self.rs_pos += r;
            self.compact_mid();
        }
        (cursor, produced)
    }

    fn max_output_for(&self, in_frames: usize) -> usize {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let stretched = (in_frames as f64 * self.time_ratio).ceil() as usize;
        stretched + self.tick + 2
    }

    /// Nothing: the first frame out is the first frame in. The stretcher
    /// needs record ahead of where it plays, which the deck supplies by
    /// feeding it, not by waiting.
    fn latency_frames(&self) -> usize {
        0
    }

    fn reset(&mut self) {
        self.fill = 0;
        self.split_fill = 0;
        self.split_warm = self.split_delay;
        self.lowpass.clear();
        self.base = 0;
        self.hit_count = 0;
        self.block_level = 0.0;
        self.block_frames = 0;
        self.block_last = 0.0;
        self.recent_levels = [0.0; HIT_MEMORY];
        self.blocks_since_hit = HIT_REST_BLOCKS;
        self.nominal = 0.0;
        self.bands = [Band {
            read: 0,
            fade: None,
        }; 2];
        self.mid_len = 0;
        self.rs_pos = 0.0;
        self.joins = Joins::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// Something like music: a bass note, a chord and a noise burst twice a
    /// second, different in the two channels.
    fn music(frames: usize) -> Vec<f32> {
        let mut seed = 0x1234_5678u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            #[allow(clippy::cast_precision_loss)]
            {
                (seed >> 8) as f32 / 8_388_608.0 - 1.0
            }
        };
        let mut out = vec![0.0f32; frames * 2];
        for i in 0..frames {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / SR;
            let tone = |hz: f32| (std::f32::consts::TAU * hz * t).sin();
            let since_hit = i % 24_000;
            #[allow(clippy::cast_precision_loss)]
            let hit = if since_hit < 400 {
                noise() * 0.6 * (-(since_hit as f32) / 90.0).exp()
            } else {
                0.0
            };
            out[i * 2] = 0.3 * tone(55.0) + 0.1 * tone(220.0) + hit;
            out[i * 2 + 1] = 0.3 * tone(55.0) + 0.1 * tone(330.0) + hit * 0.7;
        }
        out
    }

    fn drive(s: &mut SpliceStretcher, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::new();
        let mut scratch = vec![0.0f32; 512 * 2];
        let mut cursor = 0usize;
        loop {
            let (consumed, produced) = s.process(&input[cursor * 2..], &mut scratch);
            cursor += consumed;
            out.extend_from_slice(&scratch[..produced * 2]);
            if consumed == 0 && produced == 0 {
                return out;
            }
        }
    }

    /// With the platter at the record's own speed there is nothing to join:
    /// what comes out is the record, sample for sample, from its first frame.
    #[test]
    fn at_the_records_own_speed_it_is_the_record() {
        let input = music(96_000);
        let out = drive(&mut SpliceStretcher::new(SR), &input);
        assert!(out.len() > 150_000, "most of the record comes out");
        assert_eq!(out[..], input[..out.len()]);
    }

    /// The two halves share next to nothing: a bass note is in one, a note
    /// an octave and a half up in the other. This also pins the filter's
    /// delay — a frame out, and the bass would not cancel from the rest.
    #[test]
    fn the_split_puts_bass_in_one_half_and_the_rest_in_the_other() {
        let level = |hz: f32, low_half: bool| -> f32 {
            let mut s = SpliceStretcher::new(SR);
            let input: Vec<f32> = (0..24_000)
                .flat_map(|i| {
                    #[allow(clippy::cast_precision_loss)]
                    let v = (std::f32::consts::TAU * hz * i as f32 / SR).sin() * 0.5;
                    [v, v]
                })
                .collect();
            let mut cursor = 0;
            s.append(&input, &mut cursor, input.len() / 2);
            let half = if low_half { &s.low_mono } else { &s.rest_mono };
            // Clear of the filter's start.
            let body = &half[8_000..s.split_fill];
            #[allow(clippy::cast_precision_loss)]
            let rms = (body.iter().map(|v| v * v).sum::<f32>() / body.len() as f32).sqrt();
            // Against the note itself, both channels summed.
            20.0 * (rms / (2.0 * 0.5 * std::f32::consts::FRAC_1_SQRT_2)).log10()
        };
        assert!(
            level(60.0, true) > -0.1,
            "60 Hz in the bass: {} dB",
            level(60.0, true)
        );
        assert!(
            level(60.0, false) < -50.0,
            "60 Hz in the rest: {} dB",
            level(60.0, false)
        );
        assert!(
            level(120.0, false) < -40.0,
            "120 Hz in the rest: {} dB",
            level(120.0, false)
        );
        assert!(
            level(200.0, true) < -50.0,
            "200 Hz in the bass: {} dB",
            level(200.0, true)
        );
        assert!(
            level(200.0, false) > -0.1,
            "200 Hz in the rest: {} dB",
            level(200.0, false)
        );
    }

    #[test]
    fn the_record_lasts_as_long_as_the_ratio_says() {
        for ratio in [0.8_f64, 0.926, 1.087, 1.25] {
            let input = music(144_000);
            let mut s = SpliceStretcher::new(SR);
            s.set_time_ratio(ratio);
            let out = drive(&mut s, &input);
            // The stretcher keeps its lookahead back when the input ends.
            #[allow(clippy::cast_precision_loss)]
            let fed = (input.len() / 2 - s.lookahead - s.split_delay) as f64;
            #[allow(clippy::cast_precision_loss)]
            let got = (out.len() / 2) as f64;
            assert!(
                (got - fed * ratio).abs() < 0.05 * f64::from(SR),
                "ratio {ratio}: {got} frames out for {fed} in"
            );
        }
    }

    /// Between joins the output is the record itself, so both channels of a
    /// stereo record stay exactly as far apart as they were.
    #[test]
    fn both_channels_are_joined_at_the_same_places() {
        let mut input = music(96_000);
        for frame in input.chunks_exact_mut(2) {
            frame[1] = frame[0] * 0.5;
        }
        let mut s = SpliceStretcher::new(SR);
        s.set_time_ratio(1.0 / 1.06);
        let out = drive(&mut s, &input);
        assert!(
            s.joins().before_hits + s.joins().matched > 0,
            "joins were made"
        );
        for frame in out.chunks_exact(2) {
            assert!((frame[1] - frame[0] * 0.5).abs() < 1e-5);
        }
    }

    #[test]
    fn a_ratio_far_outside_a_djs_range_still_keeps_up() {
        for ratio in [0.25_f64, 0.5, 2.0, 4.0] {
            let input = music(240_000);
            let mut s = SpliceStretcher::new(SR);
            s.set_time_ratio(ratio);
            let out = drive(&mut s, &input);
            #[allow(clippy::cast_precision_loss)]
            let got = (out.len() / 2) as f64;
            assert!(
                (got / (240_000.0 * ratio) - 1.0).abs() < 0.1,
                "ratio {ratio}: {got} frames out"
            );
            assert!(out.iter().all(|s| s.is_finite() && s.abs() < 4.0));
        }
    }

    #[test]
    fn debt_owed_to_the_deck_is_repaid() {
        let input = music(144_000);
        let mut s = SpliceStretcher::new(SR);
        let mut scratch = vec![0.0f32; 256 * 2];
        let (mut cursor, mut produced) = (0usize, 0usize);
        // A deck that finds the record 0.5 frames further on every block.
        while produced < 96_000 {
            let (c, p) = s.process(&input[cursor * 2..], &mut scratch);
            cursor += c;
            produced += p;
            s.owe(0.5);
        }
        #[allow(clippy::cast_precision_loss)]
        let debt = s.bands[REST].read as f64 - s.nominal;
        assert!(debt.abs() <= s.rules[REST].hard, "debt left: {debt} frames");
        assert!(s.joins().before_hits + s.joins().matched > 0);
    }

    #[test]
    fn reset_starts_the_record_again() {
        let input = music(48_000);
        let mut s = SpliceStretcher::new(SR);
        s.set_time_ratio(0.9);
        let first = drive(&mut s, &input);
        s.reset();
        let second = drive(&mut s, &input);
        assert_eq!(first, second);
    }
}
