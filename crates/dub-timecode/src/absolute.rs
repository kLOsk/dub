//! Absolute-position decoding for control-vinyl timecode (M6).
//!
//! Serato/Traktor encode the absolute groove position as **one bit per
//! carrier cycle**, amplitude-modulated onto the carrier. The bit stream
//! over a whole side is a single **maximal-length sequence** (m-sequence)
//! produced by a Galois LFSR (`Format::lfsr_taps` / `lfsr_seed`). The
//! defining property of an m-sequence: every window of `N =
//! position_bits` consecutive bits is **unique** (except the forbidden
//! all-zeros window), so an `N`-bit window maps to exactly one absolute
//! position. [`PositionLut`] precomputes that window → position map.
//!
//! Why absolute decoding matters here (Dub stays **relative**): the
//! carrier-phase rate has a small residual bias (non-ideal cartridge) and
//! the deck playhead is a pure integral of it, so error accumulates as
//! drift. Absolute position is immune to carrier imperfections, so its
//! per-block *delta* gives an exact, drift-free velocity. We apply only
//! the motion (deltas), never the absolute value — a needle drop does not
//! jump the playhead (see `crate::DecodeOutput` + the engine's lift
//! policy).
//!
//! **Clean-room.** The LFSR is implemented from the published xwax/Mixxx
//! algorithm description; the per-format taps/seed are format constants
//! (see `crate::Format`), not copied code — consistent with the
//! relative decoder's clean-room provenance.
//!
//! **RT-safety.** [`PositionLut::build`] allocates (the table is up to
//! 4 MB for a 20-bit code) and MUST run off the audio thread, at
//! construction. Lookups are a single array index — alloc-free, used on
//! the hot path.

/// Parity (XOR) of the bits of `x` selected by `mask` — the LFSR
/// feedback term.
#[inline]
fn parity(x: u32, mask: u32) -> u32 {
    (x & mask).count_ones() & 1
}

/// One forward LFSR step (record moving forward). Matches the xwax
/// convention: a Fibonacci LFSR that shifts **right**, inserting a new
/// feedback bit `parity(state & (taps|1))` at the top (bit `bits-1`). For
/// a primitive `taps` this visits every non-zero `bits`-wide state
/// exactly once over its `2^bits − 1` period. The decoder reconstructs
/// the state by shifting received bits in at the top the same way, so the
/// running register **is** the LFSR state — which is what the LUT keys on.
#[inline]
#[must_use]
pub(crate) fn lfsr_fwd(state: u32, taps: u32, bits: u32) -> u32 {
    let fb = parity(state, taps | 0x1);
    (state >> 1) | (fb << (bits - 1))
}

/// One reverse LFSR step (record moving backward) — the exact inverse of
/// [`lfsr_fwd`]: shift **left** (masked), inserting `parity(state &
/// ((taps>>1) | (1<<(bits-1))))` at the LSB.
#[inline]
#[must_use]
pub(crate) fn lfsr_rev(state: u32, taps: u32, bits: u32) -> u32 {
    let mask = (1u32 << bits) - 1;
    let fb = parity(state, (taps >> 1) | (1 << (bits - 1)));
    ((state << 1) & mask) | fb
}

/// The modulation bit recorded at a given LFSR state — the state's MSB
/// (bit `bits-1`). The decoder shifts exactly this bit in at the top to
/// track the state.
#[inline]
#[must_use]
pub(crate) fn lfsr_output_bit(state: u32, bits: u32) -> u32 {
    (state >> (bits - 1)) & 1
}

/// Sentinel stored for the forbidden all-zeros window (never appears in
/// an m-sequence) and any window the LUT didn't fill.
const NO_POSITION: u32 = u32::MAX;

/// Precomputed map from an `N`-bit output-window to its absolute position
/// (in bits = carrier cycles from the start of the side).
///
/// Built once, off the audio thread, from a format's LFSR parameters.
/// `2^N` `u32` entries: 4 MB for Serato's 20-bit code.
#[derive(Debug, Clone)]
pub struct PositionLut {
    bits: u32,
    table: Box<[u32]>,
}

impl PositionLut {
    /// Build the window → position table for an `N`-bit LFSR with the
    /// given `taps` and `seed`. Returns `None` for a degenerate config
    /// (zero bits, > 24 bits — guards against a 64 MB+ allocation —, or a
    /// zero seed, which can't seed a maximal sequence).
    ///
    /// **Off-RT only.** Allocates `2^N` `u32`s and walks the full
    /// `2^N − 1` period.
    #[must_use]
    pub fn build(bits: u32, taps: u32, seed: u32) -> Option<Self> {
        if bits == 0 || bits > 24 || seed == 0 {
            return None;
        }
        let size = 1usize << bits;
        let mut table = vec![NO_POSITION; size].into_boxed_slice();

        // The decoder's running bit-register equals the LFSR state, so we
        // key the table on the state directly: walk the whole period
        // recording `state -> position`. A primitive `taps` visits each
        // non-zero state exactly once, so there are no genuine collisions.
        let mut state = seed;
        let period = (1u32 << bits) - 1;
        for i in 0..period {
            if table[state as usize] == NO_POSITION {
                table[state as usize] = i;
            }
            state = lfsr_fwd(state, taps, bits);
        }

        Some(Self { bits, table })
    }

    /// Number of bits in a position window (`N`).
    #[must_use]
    pub fn bits(&self) -> u32 {
        self.bits
    }

    /// Look up the absolute position (in bits/cycles) of an `N`-bit
    /// output-window. `None` for the all-zeros window or an unfilled
    /// slot. RT-safe — one array index.
    #[inline]
    #[must_use]
    pub fn lookup(&self, window: u32) -> Option<u32> {
        match self.table.get(window as usize).copied() {
            Some(NO_POSITION) | None => None,
            Some(pos) => Some(pos),
        }
    }
}

/// Minimum samples a completed cycle must span for its envelope bit to
/// be trusted (sliced into the level trackers / verified against the
/// LFSR prediction). Shorter traversals — boundary jitter during a
/// scratch, a direction flip mid-cycle — still advance the position and
/// the LFSR state, but their half-measured envelopes are ignored so
/// they can't poison the slicer or trip the error counter.
const MIN_CYCLE_SAMPLES: u32 = 4;

/// Consecutive *sequence-consistent* LUT hits required to declare lock.
/// Each clean window must look up to exactly `previous + 1`.
///
/// In a maximal-length sequence **every** non-zero window is in the
/// LUT, so a hit alone proves nothing — the `previous + 1` check is the
/// entire defense, and random bits pass it with probability ½ per
/// cycle. The first on-rig run shipped with 4 here and spurious-locked
/// at random groove positions ~60×/s (2⁻⁴ per cycle at 1 kHz). 32
/// pushes false locks to 2⁻³² per cycle (~once per 50 days) while only
/// stretching acquisition to ~52 ms at unity — still inaudible.
const ACQUIRE_VERIFY: u32 = 32;

/// Consecutive bit-prediction mismatches that force an unlock. One or
/// two flipped bits (dust, a click) are absorbed; a sustained run means
/// the register has desynced from the groove (a slipped cycle) and the
/// only safe recovery is a fresh acquisition.
const MAX_ERROR_RUN: u32 = 4;

/// Per-cycle EMA pole for the slicer's high/low level trackers. Each
/// level only updates on cycles classified into it, so the threshold
/// holds steady through the m-sequence's longest same-bit runs.
/// (Used by the offline `--sweep` slicer; the live tracker uses the
/// peak/trough follower below.)
const LEVEL_ALPHA: f64 = 0.08;

/// Slow "unstick" pole for the live slicer's *unselected* rail. Keeps
/// the rail the current cycle wasn't classified into drifting gently
/// toward the signal so it can't get stranded (the all-zero-bitstream
/// deadlock). Far slower than `LEVEL_ALPHA` so it barely perturbs the
/// threshold but survives the m-sequence's ~20-cycle same-bit runs.
const LEVEL_UNSTICK: f64 = 0.003;

/// EMA pole for the bit-agreement confidence readout.
const AGREE_ALPHA: f32 = 0.05;

/// Locked `agreement` below this unlocks — catches sustained ~50 %
/// desyncs (reverse play, foreign carrier) that the consecutive-error
/// counter slips past. Clean forward play sits at ~1.0, so this never
/// fires on a healthy lock.
const UNLOCK_AGREEMENT: f32 = 0.7;

/// EMA pole for the read-interval (cycle period) estimate that drives
/// sub-cycle position interpolation. Fast enough to track playback-speed
/// changes within a few cycles, slow enough to smooth read jitter.
const READ_INTERVAL_ALPHA: f64 = 0.1;

/// Carrier-presence floor on |s|² — used by the offline `--sweep`
/// front-end ([`extract_cycles`]); the live tracker gates on the xwax
/// zero-crossing hysteresis + the no-read timeout instead.
const PRESENCE_FLOOR_SQ: f64 = 1e-4;

/// Per-variant LFSR acquisition/lock state. The tracker holds one per
/// auto-detect candidate (Serato 2a/2b/cd) and locks whichever's LFSR
/// validates against the shared bit stream.
struct VariantState {
    name: &'static str,
    lut: PositionLut,
    taps: u32,
    bits: u32,
    /// Fill window during acquisition; once locked, the LFSR state whose
    /// MSB is the current cycle's bit.
    register: u32,
    filled: u32,
    last_lookup: Option<u32>,
    consec_hits: u32,
}

/// Per-deck absolute-position tracker (M6) — AM bit read off the
/// whitened carrier phasor.
///
/// Fed the decoder's **whitened, high-passed** complex carrier `(re, im)`
/// per sample ([`AbsoluteTracker::on_sample`]) — the same phasor the
/// relative path uses. The running unwrapped phase marks each carrier
/// cycle; one data bit is read per cycle from the cycle's **magnitude
/// RMS** (the amplitude-modulated bit), sliced by a two-level threshold.
/// This per-cycle integration reads real vinyl at ~95 % vs ~80 % for the
/// raw zero-crossing peak read it replaces (validated by `--sweep` on a
/// real capture); the bit model is unchanged (still AM, not the rejected
/// phase read). The bit stream feeds N parallel LFSR validators (one per
/// candidate pressing); the first to validate locks, and its
/// [`PositionLut`] resolves the absolute groove position, sub-cycle-
/// interpolated between reads.
///
/// All methods are alloc-free except [`AbsoluteTracker::new`], which
/// builds the LUTs and MUST run off the audio thread.
pub struct AbsoluteTracker {
    /// Input frames per carrier cycle at unity — the absolute-position
    /// scale (`sample_rate / carrier_hz`).
    samples_per_cycle: f64,
    /// Auto-detect candidates (share carrier + extraction flags).
    variants: Vec<VariantState>,

    // --- whitened-phasor cycle detector ---
    /// Previous sample's carrier phase, for unwrapping.
    prev_phase: f64,
    /// Running unwrapped carrier phase; its integer-cycle crossings are
    /// the bit-read boundaries.
    unwrapped: f64,
    /// Whether `prev_phase` / `unwrapped` hold a real sample yet.
    phase_primed: bool,
    /// Cycle index of the last completed boundary (its sign change tracks
    /// play direction).
    prev_cycle: i64,
    /// Per-cycle `Σ|s|²` and sample count → the cycle's magnitude RMS,
    /// the AM data-bit observable.
    cyc_mag_sq: f64,
    cyc_n: u32,
    /// Two-level deadlock-free bit-slicer rails (EMAs of the high/low
    /// per-cycle RMS); the bit is `rms > midpoint(env_high, env_low)`.
    env_high: f64,
    env_low: f64,

    // --- read interval / sub-cycle position interpolation ---
    /// Samples since the last bit read (the sub-cycle numerator).
    samples_since_read: u32,
    /// EMA of samples between reads — the actual cycle period (encodes
    /// speed); the sub-cycle denominator.
    read_interval: f64,
    read_primed: bool,
    /// Samples without a read before the carrier is treated as gone.
    no_read_limit: u32,

    // --- lock state ---
    /// Index into `variants` of the locked candidate, or `None`.
    locked_variant: Option<usize>,
    /// Absolute cycle index of the last read while locked.
    bit_index: i64,
    /// Direction of the last read — signs the sub-cycle interpolation.
    last_forward: bool,
    error_run: u32,
    agreement: f32,

    // --- diagnostics (off-RT inspection only) ---
    dbg_reads: u64,
    dbg_lut_hits: u64,
    dbg_max_consec: u32,
    dbg_first_means: [f32; 48],
    dbg_first_bits: u64,
}

impl AbsoluteTracker {
    /// Create a tracker for a format, auto-detecting across its candidate
    /// pressings. Serato builds 2a/2b/cd validators (both vinyl sides +
    /// the Control CD) and locks whichever the signal matches. Returns
    /// `None` for formats whose absolute bitstream we don't decode yet
    /// (Traktor — its def flags differ; future work).
    ///
    /// **Off-RT only** — builds one position LUT per candidate (~4 MB
    /// each for Serato).
    ///
    /// # Panics
    /// `sample_rate` must be positive.
    #[must_use]
    pub fn new(format: crate::Format, sample_rate: f32) -> Option<Self> {
        assert!(sample_rate > 0.0, "sample rate must be > 0");
        match format {
            crate::Format::SeratoCv02 => {
                Self::with_candidates(sample_rate, &crate::defs::serato_candidates())
            }
            crate::Format::TraktorMk1 | crate::Format::TraktorMk2 => None,
        }
    }

    /// Build a tracker over an explicit candidate set. All candidates
    /// must share carrier frequency and extraction flags (a mismatched
    /// one is skipped). Off-RT — builds a LUT per candidate.
    #[must_use]
    pub fn with_candidates(
        sample_rate: f32,
        candidates: &[&crate::defs::TimecodeDef],
    ) -> Option<Self> {
        let first = candidates.first()?;
        let mut variants = Vec::with_capacity(candidates.len());
        for def in candidates {
            if def.primary_right() != first.primary_right()
                || def.switch_polarity() != first.switch_polarity()
                || def.resolution != first.resolution
            {
                continue;
            }
            let lut = PositionLut::build(def.bits, def.taps, def.seed)?;
            variants.push(VariantState {
                name: def.name,
                lut,
                taps: def.taps,
                bits: def.bits,
                register: 0,
                filled: 0,
                last_lookup: None,
                consec_hits: 0,
            });
        }
        if variants.is_empty() {
            return None;
        }
        let spc = f64::from(sample_rate) / f64::from(first.resolution);
        Some(Self {
            samples_per_cycle: spc,
            variants,
            prev_phase: 0.0,
            unwrapped: 0.0,
            phase_primed: false,
            prev_cycle: 0,
            cyc_mag_sq: 0.0,
            cyc_n: 0,
            env_high: 0.0,
            env_low: 0.0,
            samples_since_read: 0,
            read_interval: spc,
            read_primed: false,
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            no_read_limit: (spc * 8.0) as u32,
            locked_variant: None,
            bit_index: 0,
            last_forward: true,
            error_run: 0,
            agreement: 0.0,
            dbg_reads: 0,
            dbg_lut_hits: 0,
            dbg_max_consec: 0,
            dbg_first_means: [0.0; 48],
            dbg_first_bits: 0,
        })
    }

    /// Whether the LFSR position is currently locked to a variant.
    #[must_use]
    pub fn locked(&self) -> bool {
        self.locked_variant.is_some()
    }

    /// The locked variant's name (e.g. `"serato_cd"`), or `None`.
    #[must_use]
    pub fn locked_variant_name(&self) -> Option<&'static str> {
        self.locked_variant.map(|i| self.variants[i].name)
    }

    /// Diagnostics: `(reads, lut_hits, max_consecutive_hits)`. Off-RT.
    #[must_use]
    pub fn debug_acquisition(&self) -> (u64, u64, u32) {
        (self.dbg_reads, self.dbg_lut_hits, self.dbg_max_consec)
    }

    /// First 48 `(bit_peak, bit)` reads — for the `--sweep` probe to
    /// compare the live reader against the offline one. Off-RT.
    #[must_use]
    pub fn debug_first_cycles(&self) -> ([f32; 48], u64) {
        (self.dbg_first_means, self.dbg_first_bits)
    }

    /// Bit-agreement confidence in `[0, 1]`; `0` while unlocked.
    #[must_use]
    pub fn confidence(&self) -> f32 {
        if self.locked() {
            self.agreement
        } else {
            0.0
        }
    }

    /// Absolute groove position in carrier cycles: the LFSR integer
    /// anchor plus a sub-cycle interpolation between reads (so the engine
    /// gets a smooth, not cycle-quantized, position). `None` until locked.
    #[must_use]
    pub fn position_cycles(&self) -> Option<f64> {
        if !self.locked() {
            return None;
        }
        let frac = if self.read_interval > 0.0 {
            (f64::from(self.samples_since_read) / self.read_interval).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // Interpolate in the direction of travel: forward play advances
        // past the last read, reverse play recedes from it.
        let signed = if self.last_forward { frac } else { -frac };
        #[allow(clippy::cast_precision_loss)] // |bit_index| ≪ 2^52
        Some(self.bit_index as f64 + signed)
    }

    /// Absolute groove position in input frames at unity speed.
    /// `None` until locked.
    #[must_use]
    pub fn position_frames(&self) -> Option<f64> {
        self.position_cycles().map(|c| c * self.samples_per_cycle)
    }

    /// Full reset (new record / deck re-cue): unlock, forget the phase
    /// tracker and read clock. RT-safe.
    pub fn reset(&mut self) {
        self.unlock();
        self.phase_primed = false;
        self.cyc_mag_sq = 0.0;
        self.cyc_n = 0;
        self.samples_since_read = 0;
        self.read_primed = false;
    }

    /// Feed one sample's **whitened** complex carrier `(re, im)` — the
    /// same high-passed, whitened phasor the decoder computes for the
    /// relative path (`s = re + j·im = A·e^(jφ)`). Tracks the unwrapped
    /// carrier phase and, on each completed cycle, reads one AM data bit
    /// from the cycle's magnitude RMS. Alloc-free; per-sample on the
    /// audio thread.
    pub fn on_sample(&mut self, re: f64, im: f64) {
        let mag_sq = re * re + im * im;
        self.samples_since_read = self.samples_since_read.saturating_add(1);

        if mag_sq < PRESENCE_FLOOR_SQ {
            // Carrier gone (stylus lift / silence): drop the phase lock
            // and the in-progress cycle; unlock after a sustained gap.
            self.phase_primed = false;
            self.cyc_mag_sq = 0.0;
            self.cyc_n = 0;
            if self.samples_since_read >= self.no_read_limit {
                self.unlock();
                self.read_primed = false;
                self.samples_since_read = 0;
            }
            return;
        }

        let phase = im.atan2(re);
        if !self.phase_primed {
            self.prev_phase = phase;
            self.unwrapped = phase;
            self.prev_cycle = floor_cycles(self.unwrapped);
            self.phase_primed = true;
            self.cyc_mag_sq = mag_sq;
            self.cyc_n = 1;
            self.samples_since_read = 0;
            return;
        }

        // Unwrap the phase step and integrate the cycle's energy.
        let mut d = phase - self.prev_phase;
        if d > std::f64::consts::PI {
            d -= std::f64::consts::TAU;
        } else if d < -std::f64::consts::PI {
            d += std::f64::consts::TAU;
        }
        self.prev_phase = phase;
        self.unwrapped += d;
        self.cyc_mag_sq += mag_sq;
        self.cyc_n += 1;

        let cycle = floor_cycles(self.unwrapped);
        if cycle == self.prev_cycle {
            return;
        }
        // A carrier cycle completed. Direction is the sign of the cycle
        // step; the data bit is the cycle's magnitude RMS, two-level
        // sliced.
        let forward = cycle > self.prev_cycle;
        self.prev_cycle = cycle;
        #[allow(clippy::cast_precision_loss)]
        let rms = (self.cyc_mag_sq / f64::from(self.cyc_n.max(1))).sqrt();
        let full = self.cyc_n >= MIN_CYCLE_SAMPLES;
        self.cyc_mag_sq = 0.0;
        self.cyc_n = 0;

        let dt = f64::from(self.samples_since_read);
        self.samples_since_read = 0;

        if !self.read_primed {
            // Seed the slicer + read clock from the first full cycle; no
            // bit is trustworthy until then.
            if full {
                self.read_primed = true;
                self.read_interval = dt;
                self.env_high = rms;
                self.env_low = rms;
            }
            return;
        }
        self.read_interval += READ_INTERVAL_ALPHA * (dt - self.read_interval);

        // Two-level deadlock-free slice. The rails update only on full
        // cycles so a scratch-turnaround partial can't poison the
        // threshold; the partial's (untrusted) bit still steps the LFSR
        // so the position keeps tracking the groove.
        let threshold = 0.5 * (self.env_high + self.env_low);
        let bit = u32::from(rms > threshold);
        if full {
            if bit == 1 {
                self.env_high += LEVEL_ALPHA * (rms - self.env_high);
                self.env_low += LEVEL_UNSTICK * (rms - self.env_low);
            } else {
                self.env_low += LEVEL_ALPHA * (rms - self.env_low);
                self.env_high += LEVEL_UNSTICK * (rms - self.env_high);
            }
        }

        #[allow(clippy::cast_possible_truncation)]
        if (self.dbg_reads as usize) < self.dbg_first_means.len() {
            let idx = self.dbg_reads as usize;
            self.dbg_first_means[idx] = rms as f32;
            self.dbg_first_bits |= u64::from(bit) << idx;
        }
        self.on_bit_read(bit, forward);
    }

    /// Feed one decoded bit (with its play direction) to the locked
    /// variant (validate + step the position in that direction) or, while
    /// unlocked, to every candidate validator.
    fn on_bit_read(&mut self, bit: u32, forward: bool) {
        self.dbg_reads += 1;
        if let Some(vi) = self.locked_variant {
            let v = &mut self.variants[vi];
            // The register holds the current cycle's state; its bit is the
            // one being read. Verify, then step the state in the motion
            // direction (fwd or its exact inverse rev).
            let predicted = lfsr_output_bit(v.register, v.bits);
            v.register = if forward {
                lfsr_fwd(v.register, v.taps, v.bits)
            } else {
                lfsr_rev(v.register, v.taps, v.bits)
            };
            if bit == predicted {
                self.error_run = 0;
                self.agreement += AGREE_ALPHA * (1.0 - self.agreement);
            } else {
                self.error_run += 1;
                self.agreement -= AGREE_ALPHA * self.agreement;
                // Hard desync (a slipped cycle / dropout): a short run of
                // consecutive misses.
                if self.error_run >= MAX_ERROR_RUN {
                    self.unlock();
                    return;
                }
            }
            // Soft desync: sustained ~50 % agreement — a bare/foreign
            // carrier. The hard run-counter misses these because matches
            // keep resetting it.
            if self.agreement < UNLOCK_AGREEMENT {
                self.unlock();
                return;
            }
            self.bit_index += if forward { 1 } else { -1 };
            self.last_forward = forward;
        } else if forward {
            // Acquire only on forward play (DJs cue forward); a reverse
            // window would index the LUT backward and never self-verify.
            // A stray reverse read (sampling jitter near a crossing) is
            // simply skipped, not treated as a window break.
            self.acquire_all(bit);
        }
    }

    /// Acquisition: shift the bit into every candidate's register (MSB),
    /// and lock to the first whose LUT positions advance by exactly one
    /// for `ACQUIRE_VERIFY` consecutive reads — the m-sequence's own
    /// predict-and-verify, run in parallel so the right pressing wins.
    fn acquire_all(&mut self, bit: u32) {
        let mut lut_hits = 0u64;
        let mut max_consec = self.dbg_max_consec;
        let mut lock_to: Option<(usize, i64)> = None;
        for (i, v) in self.variants.iter_mut().enumerate() {
            v.register = (v.register >> 1) | (bit << (v.bits - 1));
            v.filled = (v.filled + 1).min(v.bits);
            if v.filled < v.bits {
                continue;
            }
            let Some(p) = v.lut.lookup(v.register) else {
                v.last_lookup = None;
                v.consec_hits = 0;
                continue;
            };
            lut_hits += 1;
            let period = (1u32 << v.bits) - 1;
            let consistent = v.last_lookup.is_some_and(|prev| p == (prev + 1) % period);
            v.consec_hits = if consistent { v.consec_hits + 1 } else { 0 };
            max_consec = max_consec.max(v.consec_hits);
            v.last_lookup = Some(p);
            if v.consec_hits >= ACQUIRE_VERIFY && lock_to.is_none() {
                lock_to = Some((i, i64::from(p)));
            }
        }
        self.dbg_lut_hits += lut_hits;
        self.dbg_max_consec = max_consec;

        if let Some((i, p)) = lock_to {
            // The just-read bit completed the window at sequence index
            // `p`; we've now entered cycle `p + 1`, so anchor there and
            // advance the register to that state (matches the absolute
            // position the generator/groove is at right now).
            let v = &mut self.variants[i];
            v.register = lfsr_fwd(v.register, v.taps, v.bits);
            self.locked_variant = Some(i);
            self.bit_index = p + 1;
            self.last_forward = true;
            self.agreement = 1.0;
            self.error_run = 0;
        }
    }

    fn unlock(&mut self) {
        self.locked_variant = None;
        self.agreement = 0.0;
        self.error_run = 0;
        self.reset_acquisition();
    }

    fn reset_acquisition(&mut self) {
        for v in &mut self.variants {
            v.register = 0;
            v.filled = 0;
            v.last_lookup = None;
            v.consec_hits = 0;
        }
    }
}

/// Cycle index of an unwrapped phase (floor of `phase / 2π`).
#[inline]
fn floor_cycles(unwrapped: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)] // |cycles| ≪ 2^63
    {
        (unwrapped / std::f64::consts::TAU).floor() as i64
    }
}

// ===================================================================
// Offline convention sweep (M6 on-rig debugging — NOT on the RT path).
//
// The synthetic generator and the tracker share one bitstream
// convention, so the unit tests can't catch a *real-pressing* mismatch:
// if a real Serato disc encodes the AM bit with the opposite polarity,
// the reverse bit order, or the reciprocal polynomial, the tracker
// extracts bits that are uncorrelated with our LFSR and never locks
// (the first on-rig run's signature exactly). There are only a handful
// of plausible conventions; rather than guess one and rebuild, this
// sweep tries them all against a captured WAV and reports which one (if
// any) yields a coherent maximal-length sequence.
//
// All of this allocates and runs offline (the `dub decode-timecode
// --sweep` tool), never on the audio thread.
// ===================================================================

/// One completed carrier cycle's observation, extracted from a capture.
///
/// Carries several candidate "bit observables" so the deep sweep can ask
/// *where* the data bit actually lives — total amplitude (our original
/// AM model), per-channel amplitude, the channel difference (a
/// differential/biphase scheme), or the residual DC phasor (a
/// phase-modulation scheme). On a real Serato signal the right one is
/// the question; the unit-test generator only exercises `mean`.
#[derive(Clone, Copy, Debug)]
pub struct CycleObs {
    /// RMS of the whitened carrier magnitude — the original AM-bit model.
    pub mean: f64,
    /// RMS of raw channel 0 over the cycle.
    pub rms_ch0: f64,
    /// RMS of raw channel 1 over the cycle.
    pub rms_ch1: f64,
    /// `rms_ch0 − rms_ch1` — a differential-amplitude observable.
    pub diff: f64,
    /// Residual DC phasor magnitude after removing the uniform carrier
    /// rotation — nonzero when the carrier phase is modulated (PSK).
    pub dc_re: f64,
    /// Imaginary part of the residual DC phasor (see `dc_re`).
    pub dc_im: f64,
    /// Direction of travel across this cycle boundary.
    pub forward: bool,
    /// `true` if the cycle spanned enough samples to trust its envelope.
    pub full: bool,
}

/// Run the decoder front-end (channel whitening + carrier-phase cycle
/// detection, identical to [`AbsoluteTracker::on_sample`]) over a raw
/// stereo capture and return one [`CycleObs`] per completed carrier
/// cycle. Off-RT: computes whitening over the whole buffer and
/// allocates the result vector.
///
/// `stereo` is interleaved `[ch0, ch1, ch0, ch1, …]` (same convention
/// the engine feeds the decoder). Returns an empty vec for a non-AM
/// format (MK2) or a buffer with no detectable carrier.
#[must_use]
pub fn extract_cycles(format: crate::Format, stereo: &[f32]) -> Vec<CycleObs> {
    if format.lfsr_taps().is_none() {
        return Vec::new();
    }
    let w = crate::decoder::compute_whitening(stereo);

    let mut out = Vec::new();
    let mut prev_phase = 0.0_f64;
    let mut phase_primed = false;
    let mut unwrapped = 0.0_f64;
    let mut prev_cycle = 0_i64;

    // Reference NCO for the phase-residual (PSK) observable: a
    // free-running oscillator at the carrier's mean angular velocity,
    // estimated on the fly by an EMA of the per-sample phase step. The
    // residual `unwrapped − nco` is slowly varying for a clean carrier
    // and flips by π wherever the carrier phase is data-modulated.
    let mut omega_est = 0.0_f64;
    let mut nco = 0.0_f64;
    let mut omega_primed = false;

    // Per-cycle accumulators.
    let mut acc = CycleAcc::default();

    for frame in stereo.chunks_exact(2) {
        let re_raw = f64::from(frame[1]);
        let im_raw = f64::from(frame[0]);
        let re = w[0][0] * re_raw + w[0][1] * im_raw;
        let im = w[1][0] * re_raw + w[1][1] * im_raw;
        let mag_sq = re * re + im * im;
        if mag_sq < PRESENCE_FLOOR_SQ {
            phase_primed = false;
            acc = CycleAcc::default();
            continue;
        }
        let phase = im.atan2(re);
        if !phase_primed {
            prev_phase = phase;
            phase_primed = true;
            unwrapped = phase;
            nco = phase;
            prev_cycle = floor_cycles(unwrapped);
            acc = CycleAcc::default();
            continue;
        }
        let mut d = phase - prev_phase;
        if d > std::f64::consts::PI {
            d -= std::f64::consts::TAU;
        } else if d < -std::f64::consts::PI {
            d += std::f64::consts::TAU;
        }
        prev_phase = phase;
        unwrapped += d;
        if omega_primed {
            omega_est += 0.001 * (d - omega_est);
        } else {
            omega_est = d;
            omega_primed = true;
        }
        nco += omega_est;
        let resid = unwrapped - nco;

        acc.mag_sq += mag_sq;
        acc.ch0_sq += im_raw * im_raw;
        acc.ch1_sq += re_raw * re_raw;
        acc.cos += resid.cos();
        acc.sin += resid.sin();
        acc.n += 1;

        let cycle = floor_cycles(unwrapped);
        if cycle != prev_cycle {
            out.push(acc.finish(cycle > prev_cycle));
            acc = CycleAcc::default();
        }
        prev_cycle = cycle;
    }
    out
}

/// Per-cycle accumulator for [`extract_cycles`]'s candidate observables.
#[derive(Default)]
struct CycleAcc {
    mag_sq: f64,
    ch0_sq: f64,
    ch1_sq: f64,
    cos: f64,
    sin: f64,
    n: u32,
}

impl CycleAcc {
    fn finish(&self, forward: bool) -> CycleObs {
        let n = f64::from(self.n.max(1));
        let rms_ch0 = (self.ch0_sq / n).sqrt();
        let rms_ch1 = (self.ch1_sq / n).sqrt();
        CycleObs {
            mean: (self.mag_sq / n).sqrt(),
            rms_ch0,
            rms_ch1,
            diff: rms_ch0 - rms_ch1,
            dc_re: self.cos / n,
            dc_im: self.sin / n,
            forward,
            full: self.n >= MIN_CYCLE_SAMPLES,
        }
    }
}

/// Reverse the low `bits` bits of `x` (the reciprocal-polynomial /
/// opposite-bit-order tap variant).
#[must_use]
fn reverse_bits(x: u32, bits: u32) -> u32 {
    let mut r = 0u32;
    for i in 0..bits {
        r |= ((x >> i) & 1) << (bits - 1 - i);
    }
    r
}

/// A candidate bitstream convention and how well the captured bits obey
/// the LFSR recurrence under it.
#[derive(Clone, Copy, Debug)]
pub struct ConventionResult {
    /// AM bit read with inverted polarity (high envelope = 0).
    pub polarity_inverted: bool,
    /// Bit stream consumed in reverse order (encoding direction flip).
    pub reversed: bool,
    /// Reciprocal tap polynomial (`reverse_bits(taps)`).
    pub tap_reversed: bool,
    /// Fraction of predicted bits that matched the observation `[0, 1]`.
    /// ≈1.0 = this is the pressing's convention; ≈0.5 = uncorrelated.
    pub agreement: f64,
    /// Longest run of consecutive correct predictions — a sharper
    /// signal than `agreement` (a true m-sequence predicts perfectly
    /// for thousands of bits once the window fills).
    pub longest_run: usize,
    /// Predictions evaluated (bits beyond the first window).
    pub predictions: usize,
    /// Fraction of the stream that is 1-bits. A real m-sequence is
    /// **balanced** (≈0.5); a degenerate constant stream (≈0 or ≈1)
    /// trivially satisfies the recurrence at 100% and must be rejected.
    pub balance: f64,
}

/// Is a stream balanced enough to be a real m-sequence (not a constant
/// artifact that trivially obeys the recurrence)?
#[must_use]
fn is_balanced(balance: f64) -> bool {
    (0.4..=0.6).contains(&balance)
}

/// Slice cycle observations into a clean forward bit stream using the
/// same self-calibrating two-level EMA slicer the tracker uses, then
/// score every plausible convention against the LFSR recurrence.
///
/// Returns the candidates sorted best-first. If the top candidate's
/// `agreement` is near 1.0 with a long run, its flags are the pressing's
/// real convention; if every candidate sits near 0.5, the one-bit-per-
/// cycle AM model itself doesn't fit this disc.
#[must_use]
pub fn sweep_conventions(format: crate::Format, cycles: &[CycleObs]) -> Vec<ConventionResult> {
    let bits = format.position_bits();
    let Some(taps) = format.lfsr_taps() else {
        return Vec::new();
    };
    // Forward, full cycles only: a steady-play capture is almost all of
    // these, and they're the only ones the tracker trusts for a bit.
    let values: Vec<f64> = cycles
        .iter()
        .filter(|c| c.forward && c.full)
        .map(|c| c.mean)
        .collect();
    let base = slice_values(&values);
    let mut results = scan_conventions(&base, taps, bits);
    results.sort_by(cmp_conventions);
    results
}

/// Slice a stream of per-cycle observable values into bits with the same
/// deadlock-free two-level slicer the live tracker uses (fast-EMA the
/// selected rail, slow-unstick the other). Must match
/// [`AbsoluteTracker::slice`] so a sweep result predicts the tracker.
fn slice_values(values: &[f64]) -> Vec<u8> {
    let mut bits = Vec::with_capacity(values.len());
    let mut env_high = 0.0_f64;
    let mut env_low = 0.0_f64;
    let mut primed = false;
    for &v in values {
        if !primed {
            env_high = v;
            env_low = v;
            primed = true;
            continue;
        }
        let threshold = 0.5 * (env_high + env_low);
        let high = v > threshold;
        if high {
            env_high += LEVEL_ALPHA * (v - env_high);
            env_low += LEVEL_UNSTICK * (v - env_low);
        } else {
            env_low += LEVEL_ALPHA * (v - env_low);
            env_high += LEVEL_UNSTICK * (v - env_high);
        }
        bits.push(u8::from(high));
    }
    bits
}

/// Score all 8 polarity × bit-order × tap-polynomial conventions for a
/// sliced bit stream.
fn scan_conventions(base: &[u8], taps: u32, bits: u32) -> Vec<ConventionResult> {
    let mut results = Vec::new();
    for polarity_inverted in [false, true] {
        for reversed in [false, true] {
            for tap_reversed in [false, true] {
                let t = if tap_reversed {
                    reverse_bits(taps, bits)
                } else {
                    taps
                };
                let mut stream: Vec<u8> = base
                    .iter()
                    .map(|&b| if polarity_inverted { b ^ 1 } else { b })
                    .collect();
                if reversed {
                    stream.reverse();
                }
                let (agreement, longest_run, predictions, balance) =
                    score_recurrence(&stream, t, bits);
                results.push(ConventionResult {
                    polarity_inverted,
                    reversed,
                    tap_reversed,
                    agreement,
                    longest_run,
                    predictions,
                    balance,
                });
            }
        }
    }
    results
}

/// Best-first ordering. The true discriminator of a real decode is
/// **agreement → 1.0** on a **balanced** stream. `longest_run` is *not*
/// reliable: a constant stream (all 0) and even a slowly-drifting sliced
/// stream rack up huge runs of trivially-correct `predict-0/observe-0`
/// while agreement stays at chance. So: balanced first, then agreement,
/// and only then run length as a tie-break.
fn cmp_conventions(a: &ConventionResult, b: &ConventionResult) -> std::cmp::Ordering {
    is_balanced(b.balance)
        .cmp(&is_balanced(a.balance))
        .then(
            b.agreement
                .partial_cmp(&a.agreement)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
        .then(b.longest_run.cmp(&a.longest_run))
}

/// Which per-cycle observable a [`DeepResult`] scored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observable {
    /// Whitened total-magnitude RMS (the original AM model).
    TotalAmplitude,
    /// Raw channel-0 RMS.
    Ch0Amplitude,
    /// Raw channel-1 RMS.
    Ch1Amplitude,
    /// Channel difference `rms_ch0 − rms_ch1` (differential AM).
    Differential,
    /// Residual DC phasor real part (phase / PSK modulation, drift-prone).
    PhaseRe,
    /// Residual DC phasor imaginary part (phase / PSK modulation).
    PhaseIm,
    /// **Differential** phase, real part: `Re(z_k · conj(z_{k-1}))` of the
    /// per-cycle residual phasor. The shared slow reference drift cancels
    /// between neighbours, so this reads a BPSK/biphase bit drift-free.
    DiffPhaseRe,
    /// Differential phase, imaginary part: `Im(z_k · conj(z_{k-1}))` —
    /// catches a ±90° (quadrature) phase encoding.
    DiffPhaseIm,
}

impl Observable {
    /// Short label for the sweep report.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Observable::TotalAmplitude => "total-amp",
            Observable::Ch0Amplitude => "ch0-amp",
            Observable::Ch1Amplitude => "ch1-amp",
            Observable::Differential => "diff",
            Observable::PhaseRe => "phase-re",
            Observable::PhaseIm => "phase-im",
            Observable::DiffPhaseRe => "dphase-re",
            Observable::DiffPhaseIm => "dphase-im",
        }
    }

    /// Whether this observable is a differential (needs the previous
    /// cycle's phasor) rather than a per-cycle scalar.
    fn is_differential(self) -> bool {
        matches!(self, Observable::DiffPhaseRe | Observable::DiffPhaseIm)
    }

    fn value(self, c: &CycleObs) -> f64 {
        match self {
            Observable::TotalAmplitude => c.mean,
            Observable::Ch0Amplitude => c.rms_ch0,
            Observable::Ch1Amplitude => c.rms_ch1,
            Observable::Differential => c.diff,
            Observable::PhaseRe | Observable::DiffPhaseRe => c.dc_re,
            Observable::PhaseIm | Observable::DiffPhaseIm => c.dc_im,
        }
    }

    /// Build the value series for this observable over a forward-cycle
    /// list. Per-cycle observables map 1:1; differential ones return
    /// `z_k · conj(z_{k-1})` (real or imaginary part), one shorter.
    fn series(self, forward: &[CycleObs]) -> Vec<f64> {
        if !self.is_differential() {
            return forward.iter().map(|c| self.value(c)).collect();
        }
        let mut out = Vec::with_capacity(forward.len().saturating_sub(1));
        for w in forward.windows(2) {
            let (a, b) = (w[0], w[1]);
            // d = z_b · conj(z_a), z = dc_re + j·dc_im.
            let re = b.dc_re * a.dc_re + b.dc_im * a.dc_im;
            let im = b.dc_im * a.dc_re - b.dc_re * a.dc_im;
            out.push(match self {
                Observable::DiffPhaseIm => im,
                _ => re,
            });
        }
        out
    }

    /// All observables, in report order.
    #[must_use]
    pub fn all() -> [Observable; 8] {
        [
            Observable::TotalAmplitude,
            Observable::Ch0Amplitude,
            Observable::Ch1Amplitude,
            Observable::Differential,
            Observable::PhaseRe,
            Observable::PhaseIm,
            Observable::DiffPhaseRe,
            Observable::DiffPhaseIm,
        ]
    }
}

/// The best convention found for one (observable, bit-rate, phase)
/// hypothesis — the deep sweep's unit of comparison.
#[derive(Clone, Copy, Debug)]
pub struct DeepResult {
    /// Which per-cycle quantity supplied the bit.
    pub observable: Observable,
    /// Carrier cycles per data bit (1 = one bit per cycle).
    pub decimation: usize,
    /// Sub-period sampling phase `0..decimation`.
    pub phase: usize,
    /// Best convention (polarity/order/taps) for this hypothesis.
    pub convention: ConventionResult,
}

/// Deep convention sweep: for every candidate bit observable, bit rate
/// (1 carrier cycle per bit up to `max_decimation`), and sub-period
/// sampling phase, slice a bit stream and find its best LFSR convention.
/// Returns all hypotheses sorted best-first.
///
/// This is the reverse-engineering probe for a real pressing whose data
/// bit isn't in per-cycle total amplitude: a near-1.0 agreement with a
/// long run on any row identifies *where* and *at what rate* the bit
/// lives.
#[must_use]
pub fn deep_sweep(
    format: crate::Format,
    cycles: &[CycleObs],
    max_decimation: usize,
) -> Vec<DeepResult> {
    let bits = format.position_bits();
    let Some(taps) = format.lfsr_taps() else {
        return Vec::new();
    };
    let forward: Vec<CycleObs> = cycles
        .iter()
        .filter(|c| c.forward && c.full)
        .copied()
        .collect();

    let mut results = Vec::new();
    for observable in Observable::all() {
        let values = observable.series(&forward);
        for decimation in 1..=max_decimation.max(1) {
            for phase in 0..decimation {
                // Decimate: one value per `decimation` cycles, starting
                // at `phase`. A bit that spans K cycles collapses to one
                // sample per bit at the matching K + alignment.
                let decimated: Vec<f64> = values
                    .iter()
                    .skip(phase)
                    .step_by(decimation)
                    .copied()
                    .collect();
                if decimated.len() <= bits as usize * 2 {
                    continue;
                }
                let base = slice_values(&decimated);
                if let Some(best) = scan_conventions(&base, taps, bits)
                    .into_iter()
                    .min_by(cmp_conventions)
                {
                    results.push(DeepResult {
                        observable,
                        decimation,
                        phase,
                        convention: best,
                    });
                }
            }
        }
    }
    results.sort_by(|a, b| cmp_conventions(&a.convention, &b.convention));
    results
}

// --- xwax-style decoder (the real Serato algorithm) -----------------
//
// xwax/Mixxx do NOT integrate a whitened phasor. Each channel runs its
// own zero-crossing detector against a slow running average; one data
// bit is read per cycle from the **primary** channel's level *at the
// instant the secondary channel crosses zero* (with the primary at a set
// polarity) — i.e. the primary's peak, 90° from the secondary. The bit
// is `peak > ref_level`, where `ref_level` is an EMA of recent peaks.
// Source: xwax timecoder.c (GPL), Mark Hills.

/// Hysteresis around the running zero, normalized to ±1.0 full-scale
/// (xwax's `128 << 16` against 16-bit-in-32 samples = 128/32768).
const XWAX_ZERO_THRESHOLD: f64 = 128.0 / 32768.0;
/// Time constant (seconds) of the per-channel zero running-average.
const XWAX_ZERO_RC: f64 = 0.001;
/// EMA window (in read peaks) for the adaptive bit reference level.
const XWAX_REF_PEAKS_AVG: f64 = 48.0;

/// Extract the timecode bit stream the xwax way for one channel/polarity
/// hypothesis. `primary_right`: primary (data) = ch1/right, secondary
/// (timing) = ch0/left (xwax's default; `false` swaps them).
/// `switch_polarity`: read on the primary's negative half instead of the
/// positive. Returns one bit per detected cycle.
#[must_use]
pub fn extract_bits_xwax(
    stereo: &[f32],
    sample_rate: f32,
    primary_right: bool,
    switch_polarity: bool,
) -> Vec<u8> {
    let dt = 1.0 / f64::from(sample_rate);
    let alpha = dt / (XWAX_ZERO_RC + dt);
    let th = XWAX_ZERO_THRESHOLD;
    let want_positive = !switch_polarity;

    let mut p_zero = 0.0_f64;
    let mut p_pos = false;
    let mut s_zero = 0.0_f64;
    let mut s_pos = false;
    let mut ref_level = th;

    let mut bits = Vec::new();
    for frame in stereo.chunks_exact(2) {
        let left = f64::from(frame[0]);
        let right = f64::from(frame[1]);
        let (p, s) = if primary_right {
            (right, left)
        } else {
            (left, right)
        };

        // Primary zero-crossing.
        if p > p_zero + th && !p_pos {
            p_pos = true;
        } else if p < p_zero - th && p_pos {
            p_pos = false;
        }
        p_zero += alpha * (p - p_zero);

        // Secondary zero-crossing (the read trigger).
        let mut s_swapped = false;
        if s > s_zero + th && !s_pos {
            s_pos = true;
            s_swapped = true;
        } else if s < s_zero - th && s_pos {
            s_pos = false;
            s_swapped = true;
        }
        s_zero += alpha * (s - s_zero);

        if s_swapped && p_pos == want_positive {
            let m = (p - p_zero).abs();
            bits.push(u8::from(m > ref_level));
            ref_level -= ref_level / XWAX_REF_PEAKS_AVG;
            ref_level += m / XWAX_REF_PEAKS_AVG;
        }
    }
    bits
}

/// One xwax variant hypothesis and its best convention.
#[derive(Clone, Copy, Debug)]
pub struct XwaxResult {
    /// Timecode variant name whose def (taps/bits/flags) was used.
    pub variant: &'static str,
    /// Primary (data) channel = right/ch1 (xwax default) vs left/ch0.
    pub primary_right: bool,
    /// Read on the primary's negative half (the `SWITCH_POLARITY` flag).
    pub switch_polarity: bool,
    /// Best polarity/order/tap convention for this hypothesis.
    pub convention: ConventionResult,
}

/// Sweep the xwax decoder over **every** timecode variant in the
/// definition table, using each variant's own channel/polarity flags and
/// taps/bits, scoring against the LFSR recurrence. A balanced,
/// ~1.0-agreement row is the real decode for that capture.
#[must_use]
pub fn sweep_xwax(stereo: &[f32], sample_rate: f32) -> Vec<XwaxResult> {
    let mut results = Vec::new();
    for def in crate::defs::TIMECODE_DEFS {
        let primary_right = def.primary_right();
        let switch_polarity = def.switch_polarity();
        let stream = extract_bits_xwax(stereo, sample_rate, primary_right, switch_polarity);
        if let Some(best) = scan_conventions(&stream, def.taps, def.bits)
            .into_iter()
            .min_by(cmp_conventions)
        {
            results.push(XwaxResult {
                variant: def.name,
                primary_right,
                switch_polarity,
                convention: best,
            });
        }
    }
    results.sort_by(|a, b| cmp_conventions(&a.convention, &b.convention));
    results
}

/// Predict-and-verify a bit stream against the Fibonacci-LFSR
/// recurrence: the register shifts each observed bit in at the MSB
/// (mirroring [`AbsoluteTracker::acquire`]), and the next bit is
/// predicted as `parity(register, taps | 1)`. Returns `(agreement,
/// longest_consecutive_correct_run, predictions_made, ones_fraction)`.
/// The ones-fraction guards against a degenerate constant stream, which
/// obeys the recurrence trivially.
fn score_recurrence(stream: &[u8], taps: u32, bits: u32) -> (f64, usize, usize, f64) {
    if stream.len() <= bits as usize {
        return (0.0, 0, 0, 0.0);
    }
    #[allow(clippy::naive_bytecount)]
    let ones = stream.iter().filter(|&&b| b == 1).count();
    #[allow(clippy::cast_precision_loss)]
    let balance = ones as f64 / stream.len() as f64;

    let mut register = 0u32;
    for &b in &stream[..bits as usize] {
        register = (register >> 1) | (u32::from(b) << (bits - 1));
    }
    let mut correct = 0usize;
    let mut predictions = 0usize;
    let mut run = 0usize;
    let mut longest = 0usize;
    for &observed in &stream[bits as usize..] {
        #[allow(clippy::cast_possible_truncation)] // parity ∈ {0, 1}
        let predicted = parity(register, taps | 0x1) as u8;
        predictions += 1;
        if predicted == observed {
            correct += 1;
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
        register = (register >> 1) | (u32::from(observed) << (bits - 1));
    }
    #[allow(clippy::cast_precision_loss)]
    let agreement = if predictions > 0 {
        correct as f64 / predictions as f64
    } else {
        0.0
    };
    (agreement, longest, predictions, balance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Format;

    fn serato() -> (u32, u32, u32) {
        let f = Format::SeratoCv02;
        (
            f.position_bits(),
            f.lfsr_taps().unwrap(),
            f.lfsr_seed().unwrap(),
        )
    }

    #[test]
    fn fwd_and_rev_are_exact_inverses() {
        // Independent of primitivity: rev must undo fwd for any state. If
        // the two step functions ever drift apart, reverse-direction
        // tracking would silently corrupt — this catches it.
        let (bits, taps, _) = serato();
        for state in [1u32, 0x59017, 0xA_AAAA, 0xF_FFFF, 0x5_5555, 0x1] {
            let f = lfsr_fwd(state, taps, bits);
            assert_eq!(
                lfsr_rev(f, taps, bits),
                state,
                "rev∘fwd != id for {state:#x}"
            );
            let r = lfsr_rev(state, taps, bits);
            assert_eq!(
                lfsr_fwd(r, taps, bits),
                state,
                "fwd∘rev != id for {state:#x}"
            );
        }
    }

    #[test]
    fn build_rejects_degenerate_configs() {
        assert!(PositionLut::build(0, 1, 1).is_none());
        assert!(PositionLut::build(25, 1, 1).is_none(), "too large");
        assert!(PositionLut::build(8, 1, 0).is_none(), "zero seed");
    }

    #[test]
    fn serato_sequence_is_maximal_and_lut_round_trips() {
        // The real 20-bit Serato sequence (taps 0x361e4, seed 0x59017,
        // xwax convention): build the 4 MB LUT and confirm it's a maximal
        // m-sequence — the state visits every non-zero value exactly once
        // over the full 2^20 - 1 period, the all-zeros state never occurs,
        // and each state resolves to its own position. This pins both the
        // primitivity of the taps and the correctness of the LUT.
        let (bits, taps, seed) = serato();
        assert_eq!(bits, 20);
        let lut = PositionLut::build(bits, taps, seed).unwrap();
        assert_eq!(lut.bits(), 20);
        assert_eq!(lut.lookup(0), None, "all-zeros state never occurs");

        let period = (1u32 << bits) - 1;
        let mut state = seed;
        let mut visited = vec![false; 1usize << bits];
        for i in 0..period {
            assert!(state != 0, "max-length sequence never hits zero");
            assert!(
                !visited[state as usize],
                "state {state:#x} repeated before full period"
            );
            visited[state as usize] = true;
            assert_eq!(
                lut.lookup(state),
                Some(i),
                "state {state:#x} -> position {i}"
            );
            state = lfsr_fwd(state, taps, bits);
        }
        assert_eq!(
            state, seed,
            "sequence wraps to the seed after one full period"
        );
    }

    // ------------------------------------------------------------------
    // AbsoluteTracker (fed from the signal generator's AM modulation)
    // ------------------------------------------------------------------

    use crate::signal::Generator;

    const SR: f32 = 48_000.0;
    const CARRIER: f64 = 1_000.0; // Serato CV02
                                  // Single-channel AM depth for the tracker tests. The xwax reader
                                  // samples the primary at the secondary crossing (instantaneous),
                                  // so at high pitch (~44 samples/cycle) borderline bits need a bit
                                  // of headroom to separate — real discs modulate at least this deep.
    const DEPTH: f32 = 0.3;

    fn modulated_generator() -> Generator {
        let mut g = Generator::new(Format::SeratoCv02, SR);
        assert!(g.enable_absolute(Format::SeratoCv02, DEPTH));
        g
    }

    fn tracker() -> AbsoluteTracker {
        AbsoluteTracker::new(Format::SeratoCv02, SR).unwrap()
    }

    /// Feed an interleaved stereo buffer to the tracker as the whitened
    /// phasor. The tests use identity whitening + a clean (DC-free)
    /// generator, so the phasor is just `re = ch1 = frame[1]`,
    /// `im = ch0 = frame[0]` — the mapping the decoder applies before
    /// `on_sample` (`s = re + j·im = A·e^(jφ)`).
    fn feed(t: &mut AbsoluteTracker, buf: &[f32]) {
        for frame in buf.chunks_exact(2) {
            t.on_sample(f64::from(frame[1]), f64::from(frame[0]));
        }
    }

    /// Ground-truth absolute position (cycles, incl. sub-cycle phase)
    /// at the generator's current state.
    #[allow(clippy::cast_precision_loss)]
    fn ground_truth(g: &Generator) -> f64 {
        g.absolute_position().unwrap() as f64 + g.phase() / std::f64::consts::TAU
    }

    /// `frames` of modulated timecode at `rate`, fed to the tracker.
    fn run(g: &mut Generator, t: &mut AbsoluteTracker, rate: f64, frames: usize) {
        let mut buf = vec![0.0f32; frames * 2];
        g.render(&mut buf, rate, 0.5);
        feed(t, &buf);
    }

    #[test]
    fn tracker_locks_and_reports_ground_truth_position() {
        let mut g = modulated_generator();
        let mut t = tracker();
        run(&mut g, &mut t, 1.0, 9_600); // 200 ms ≈ 200 cycles
        assert!(t.locked(), "tracker should lock within 200 cycles");
        assert!(t.confidence() > 0.9, "confidence = {}", t.confidence());
        let got = t.position_cycles().unwrap();
        let truth = ground_truth(&g);
        // The generator's phase is one step ahead of the last rendered
        // sample (~0.02 cycles at unity) — inside the tolerance.
        assert!((got - truth).abs() < 0.1, "got {got}, truth {truth}");
        // And in frames: cycles × (SR / carrier).
        let frames = t.position_frames().unwrap();
        assert!((frames - got * f64::from(SR) / CARRIER).abs() < 1e-9);
    }

    #[test]
    fn acquisition_latency_is_a_few_window_lengths() {
        // Lock needs the 20-bit window + ACQUIRE_VERIFY consistent
        // lookups + the adaptive `ref_level` to settle (~REF_PEAKS_AVG
        // reads): well under 200 cycles (200 ms at unity), and never
        // before the window fills.
        let mut g = modulated_generator();
        let mut t = tracker();
        let mut buf = vec![0.0f32; 48 * 2]; // ~1 carrier cycle at unity
        let mut cycles = 0u32;
        while !t.locked() {
            g.render(&mut buf, 1.0, 0.5);
            feed(&mut t, &buf);
            cycles += 1;
            assert!(cycles <= 200, "no lock after {cycles} cycles");
        }
        assert!(cycles > 20, "locked before the window could fill?!");
    }

    /// Scale ch1 — the same ~2 dB cartridge imbalance the decoder tests
    /// use. It biases the carrier-phase rate by ≈ −2.3 %; the absolute
    /// tracker must not care.
    fn imbalance(buf: &mut [f32]) {
        for frame in buf.chunks_exact_mut(2) {
            frame[1] *= 1.26;
        }
    }

    fn run_imbalanced(g: &mut Generator, t: &mut AbsoluteTracker, rate: f64, frames: usize) {
        let mut buf = vec![0.0f32; frames * 2];
        g.render(&mut buf, rate, 0.5);
        imbalance(&mut buf);
        feed(t, &buf);
    }

    #[test]
    fn locked_velocity_is_exact_despite_channel_imbalance() {
        // The headline M6 property. A 2 dB channel imbalance biases the
        // carrier-phase rate estimate by ≈ −2.3 % (see decoder tests);
        // velocity from absolute-position deltas counts groove cycles
        // and is immune. ±8 % must read back exactly ±8 %.
        for pitch in [0.08_f64, -0.08] {
            let rate = 1.0 + pitch;
            let mut g = modulated_generator();
            let mut t = tracker();
            // Acquire at unity (as a DJ starts the record), then ride the
            // pitch fader to the extreme — the realistic path.
            run_imbalanced(&mut g, &mut t, 1.0, 24_000);
            assert!(t.locked(), "no lock at unity");
            run_imbalanced(&mut g, &mut t, rate, 24_000); // settle at pitch
            assert!(t.locked(), "lost lock riding to rate {rate}");
            let start = t.position_cycles().unwrap();
            let secs = 2.0;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let frames = (secs * f64::from(SR)) as usize;
            run_imbalanced(&mut g, &mut t, rate, frames);
            assert!(t.locked(), "lost lock at rate {rate}");
            let dp = t.position_cycles().unwrap() - start;
            let measured = dp / (CARRIER * secs);
            assert!(
                (measured - rate).abs() < 1e-3,
                "abs velocity {measured} should be exactly {rate}"
            );
        }
    }

    #[test]
    fn position_does_not_drift_over_a_long_run() {
        // The "sticker drift" regression: integrate for minutes and the
        // absolute position must still match ground truth to a fraction
        // of a cycle. The relative path would accumulate any rate bias.
        let mut g = modulated_generator();
        let mut t = tracker();
        for _ in 0..120 {
            run_imbalanced(&mut g, &mut t, 0.97, 48_000); // 1 s per chunk
        }
        assert!(t.locked(), "lost lock during long run");
        let got = t.position_cycles().unwrap();
        let truth = ground_truth(&g);
        assert!(
            (got - truth).abs() < 0.2,
            "drift after 2 min: got {got}, truth {truth}"
        );
    }

    #[test]
    fn reverse_motion_stays_locked_and_tracks_truth() {
        // The play direction is read from which secondary crossing (up vs
        // down) coincides with the primary's gated half-cycle, so reverse
        // play steps the LFSR backward and the position decrements — the
        // lock holds and tracks the groove truth in both directions.
        let mut g = modulated_generator();
        let mut t = tracker();
        run(&mut g, &mut t, 1.0, 48_000); // 1 s forward: lock
        assert!(t.locked());
        let before = t.position_cycles().unwrap();
        run(&mut g, &mut t, -1.0, 24_000); // 0.5 s reverse ≈ 500 cycles
        assert!(t.locked(), "reverse play must keep the lock");
        // The absolute position must *recede* by the groove travelled —
        // exact reverse velocity, the property the engine consumes (a
        // constant sub-cycle offset between fwd/rev conventions is
        // irrelevant to the delta).
        let delta = before - t.position_cycles().unwrap();
        assert!(
            (delta - 500.0).abs() < 2.0,
            "reverse moved {delta}, expected ~500"
        );
    }

    #[test]
    fn forward_jitter_keeps_lock_and_advances_monotonically() {
        // A steady forward hold with small per-block speed wobble (a DJ
        // riding the platter forward, not scratching back): the lock must
        // survive and the position only advance.
        let mut g = modulated_generator();
        let mut t = tracker();
        run(&mut g, &mut t, 1.0, 48_000);
        assert!(t.locked());
        let before = t.position_cycles().unwrap();
        let mut buf = vec![0.0f32; 48 * 2];
        for i in 0..200 {
            let rate = if i % 2 == 0 { 1.1 } else { 0.9 }; // forward wobble
            g.render(&mut buf, rate, 0.5);
            feed(&mut t, &buf);
        }
        assert!(t.locked(), "forward jitter must not break the lock");
        let after = t.position_cycles().unwrap();
        assert!(after > before, "position must advance: {before} -> {after}");
    }

    #[test]
    fn desync_unlocks_and_reacquires() {
        let mut g = modulated_generator();
        let mut t = tracker();
        run(&mut g, &mut t, 1.0, 24_000);
        assert!(t.locked());

        // A bare (unmodulated) carrier slices to a constant bit which
        // disagrees with the m-sequence prediction ~50 % of the time —
        // sustained disagreement must unlock. And while it persists, the
        // constant acquisition window never advances by one, so no
        // false re-lock either.
        let mut bare = Generator::new(Format::SeratoCv02, SR);
        let mut buf = vec![0.0f32; 24_000 * 2];
        bare.render(&mut buf, 1.0, 0.5);
        feed(&mut t, &buf);
        assert!(!t.locked(), "bare carrier must unlock the tracker");
        assert!(t.confidence() < f32::EPSILON);
        assert_eq!(t.position_cycles(), None);

        // Modulation returns → fresh acquisition, correct position.
        run(&mut g, &mut t, 1.0, 24_000);
        assert!(t.locked(), "tracker should re-acquire");
        let got = t.position_cycles().unwrap();
        let truth = ground_truth(&g);
        assert!(
            (got - truth).abs() < 0.1,
            "re-acquired: got {got}, truth {truth}"
        );
    }

    #[test]
    fn dropout_unlocks() {
        let mut g = modulated_generator();
        let mut t = tracker();
        run(&mut g, &mut t, 1.0, 24_000);
        assert!(t.locked());
        let silence = vec![0.0f32; 4_800 * 2]; // 100 ms of lift
        feed(&mut t, &silence);
        assert!(!t.locked(), "silence must unlock");
        assert_eq!(t.position_cycles(), None);
        // Carrier returns → re-acquires (the slicer levels survive).
        run(&mut g, &mut t, 1.0, 24_000);
        assert!(t.locked(), "should re-acquire after the drop");
    }

    #[test]
    fn mk2_has_no_tracker() {
        assert!(
            AbsoluteTracker::new(Format::TraktorMk2, 48_000.0).is_none(),
            "MK2 bitstream isn't decoded — relative fallback"
        );
    }

    #[test]
    fn on_sample_is_alloc_free() {
        // The per-sample feed runs inside the decoder's RT loop. The
        // LUT build in `new()` is the only allocation allowed.
        let mut g = modulated_generator();
        let mut t = tracker();
        run(&mut g, &mut t, 1.0, 24_000); // lock outside the assertion
        assert!(t.locked());
        let mut buf = vec![0.0f32; 64 * 2];
        assert_no_alloc::assert_no_alloc(|| {
            for _ in 0..200 {
                g.render(&mut buf, 1.0, 0.5);
                for frame in buf.chunks_exact(2) {
                    // Whitened-phasor order: re = ch1, im = ch0.
                    t.on_sample(f64::from(frame[1]), f64::from(frame[0]));
                }
            }
        });
        assert!(t.locked());
    }
}
