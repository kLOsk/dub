//! One-shot sampler voices (M17, PRD §7.1).
//!
//! Eight slots, each holding a pre-decoded sample. A press plays the
//! sample through and ends: air horns, vocal stabs, dub-siren
//! one-shots, "rewind!" FX, drops.
//!
//! **Additive.** §7.1 is explicit that the sampler plays *over* the
//! decks rather than replacing them, so a voice sums onto the deck's
//! output bus and never writes it. It renders after the FX chain for
//! the same reason the siren does — the music FX are for the music.
//!
//! **The cursor is an integer.** Slots are resampled to the engine rate
//! when they are bound, off the audio thread, so a voice is a read and
//! an add with no rate conversion in it. That is the whole reason for
//! the bind-time conversion: a trigger has to sound on the frame the
//! key goes down, so the expensive part cannot happen here.
//!
//! **The deck is chosen at the press.** A voice sums onto whichever
//! deck bus the trigger names — the shell passes the master deck, so a
//! horn lands on the channel the crowd is hearing — and keeps that bus
//! for the life of the take. A master switch mid-horn does not hop the
//! sound across the mixer; the next press goes to the new master.

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;

use dub_io::Track;

/// Slots in the rack. Eight: the Prep sample shelf is a two-row pad
/// bank of four, laid out like a controller's (5–8 over 1–4), and the
/// shelf *is* the sampler — a slot loaded there is the pad fired on the
/// Performance surface. §7.1 shipped with four and a separate binding
/// layer in Preferences; the two collapsed into this one table.
pub const SAMPLER_SLOTS: usize = 8;

/// Frames of linear ramp applied at the start and end of a voice, and
/// across a retrigger. ~1.3 ms at 48 kHz — long enough to kill the edge
/// of a hard start, short enough that a stab still sounds instant.
const RAMP_FRAMES: usize = 64;

/// One one-shot voice.
///
/// Holds its own outgoing tail so a retrigger crossfades rather than
/// cutting: mashing the same pad is the normal way this gets used, and
/// a hard restart clicks every time.
#[derive(Default)]
pub struct SamplerVoice {
    source: Option<Arc<Track>>,
    /// Playhead in frames. Integer because the source is already at the
    /// engine rate.
    frame: usize,
    playing: bool,
    /// Linear gain. Set at bind time from the clip's measured loudness
    /// (auto-gain, the same −14 LUFS target as a track), not by hand.
    gain: f32,
    /// Deck bus this take sums onto, recorded at the trigger.
    output_deck: u8,
    /// Ramp position at the head of a trigger, counted up to
    /// [`RAMP_FRAMES`].
    attack: usize,
    /// Read position of the previous take, still ramping out after a
    /// retrigger. `None` when nothing is fading.
    tail_frame: Option<usize>,
    /// Ramp position of the outgoing tail, counted down from
    /// [`RAMP_FRAMES`].
    tail_remaining: usize,
}

impl SamplerVoice {
    /// Bind a sample to this slot, returning the one it displaced so
    /// the caller can bounce it through the trash channel. **The audio
    /// thread never drops an `Arc<Track>`.**
    #[must_use]
    pub fn set_source(&mut self, source: Arc<Track>) -> Option<Arc<Track>> {
        let previous = self.source.replace(source);
        self.playing = false;
        self.frame = 0;
        self.tail_frame = None;
        previous
    }

    /// Unbind, returning the sample for disposal off the audio thread.
    #[must_use]
    pub fn clear_source(&mut self) -> Option<Arc<Track>> {
        self.playing = false;
        self.frame = 0;
        self.tail_frame = None;
        self.source.take()
    }

    /// `true` when a sample is bound.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        self.source.is_some()
    }

    /// `true` while the one-shot is sounding.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Per-slot linear gain (§7.1).
    #[must_use]
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Set the slot's gain. Clamped to `[0, 4]`: a negative value
    /// would invert the phase against the deck it sums into, and the
    /// ceiling bounds what auto-gain may ask of a near-silent clip.
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain.clamp(0.0, 4.0);
    }

    /// Deck output bus the current take sums onto.
    #[must_use]
    pub fn output_deck(&self) -> u8 {
        self.output_deck
    }

    /// How far through the sample the take is, `0.0..=1.0`. `0.0` when
    /// idle, so the UI's progress sweep rests at the left edge.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn progress(&self) -> f32 {
        match self.source.as_ref() {
            Some(source) if self.playing && source.frames() > 0 => {
                (self.frame as f32 / source.frames() as f32).min(1.0)
            }
            _ => 0.0,
        }
    }

    /// Fire the one-shot from the top onto `output_deck`'s bus.
    /// Retriggering a sounding voice hands the current take to the tail
    /// so it ramps out under the new one instead of cutting. The tail
    /// follows the new take's bus: a horn that hops decks across a
    /// retrigger is one press, not two sounds.
    pub fn trigger(&mut self, output_deck: u8) {
        if self.source.is_none() {
            return;
        }
        if self.playing {
            self.tail_frame = Some(self.frame);
            self.tail_remaining = RAMP_FRAMES;
        }
        self.output_deck = output_deck;
        self.frame = 0;
        self.attack = 0;
        self.playing = true;
    }

    /// Stop without a click: hand the take to the tail and let it ramp.
    pub fn stop(&mut self) {
        if self.playing {
            self.tail_frame = Some(self.frame);
            self.tail_remaining = RAMP_FRAMES;
        }
        self.playing = false;
        self.frame = 0;
    }

    /// Sum this voice into `out`'s stereo pair starting at `first`.
    ///
    /// Adds; never writes. RT-safe: no allocation, no locks, and the
    /// source read goes through [`Track::frame`], which returns silence
    /// past the decode watermark rather than blocking.
    pub fn render_add(&mut self, out: &mut [f32], num_channels: usize, first: usize) {
        if !self.playing && self.tail_frame.is_none() {
            return;
        }
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let frames = out.len() / num_channels;
        let total = source.frames();

        // Cursors are lifted into locals and written back at the end so
        // the loop can hold `&self.source` without cloning the `Arc`.
        // An `Arc::clone` here would be two atomic refcount writes per
        // voice per block — not an allocation, so not a violation, but
        // avoidable traffic on the one thread that cannot afford any.
        let mut frame = self.frame;
        let mut playing = self.playing;
        let mut attack = self.attack;
        let mut tail_frame = self.tail_frame;
        let mut tail_remaining = self.tail_remaining;

        for f in 0..frames {
            let mut left = 0.0f32;
            let mut right = 0.0f32;

            if playing {
                if frame >= total {
                    playing = false;
                } else {
                    let s = source.frame(frame);
                    // Release the last RAMP_FRAMES so the tail of a
                    // stab does not end on a step.
                    let remaining = total - frame;
                    let env = ramp_in(attack) * ramp_out(remaining);
                    left += s[0] * env;
                    right += s[1] * env;
                    frame += 1;
                    attack = attack.saturating_add(1).min(RAMP_FRAMES);
                    // Clear the flag on the frame the sample runs out,
                    // not on the next block: a one-shot that has
                    // finished sounding must not still read as playing
                    // to the UI poll for another buffer.
                    if frame >= total {
                        playing = false;
                    }
                }
            }

            if let Some(tail) = tail_frame {
                if tail_remaining == 0 || tail >= total {
                    tail_frame = None;
                } else {
                    let s = source.frame(tail);
                    #[allow(clippy::cast_precision_loss)]
                    let env = tail_remaining as f32 / RAMP_FRAMES as f32;
                    left += s[0] * env;
                    right += s[1] * env;
                    tail_frame = Some(tail + 1);
                    tail_remaining -= 1;
                }
            }

            let o = f * num_channels + first;
            out[o] += left * self.gain;
            out[o + 1] += right * self.gain;
        }

        self.frame = frame;
        self.playing = playing;
        self.attack = attack;
        self.tail_frame = tail_frame;
        self.tail_remaining = tail_remaining;
    }
}

/// What the UI can see of the rack: per slot, whether the take is
/// sounding and how far through it is.
///
/// Written by the audio thread once per block, read by the shell's
/// poll — the same shape as the deck's `siren_state`. Relaxed atomics:
/// the pad lights and the sweep moves; nothing sequences on them.
#[derive(Debug)]
pub struct SamplerSharedState {
    playing: [AtomicBool; SAMPLER_SLOTS],
    /// Progress in 1/[`PROGRESS_SCALE`]ths, so the whole rack fits in a
    /// cache line and the reader needs no float atomics.
    progress: [AtomicU16; SAMPLER_SLOTS],
}

/// Resolution of the published progress. 10 000 steps is finer than any
/// pad sweep will draw and still fits a `u16`.
const PROGRESS_SCALE: f32 = 10_000.0;

impl Default for SamplerSharedState {
    fn default() -> Self {
        Self::new()
    }
}

impl SamplerSharedState {
    /// Every slot idle.
    #[must_use]
    pub fn new() -> Self {
        Self {
            playing: std::array::from_fn(|_| AtomicBool::new(false)),
            progress: std::array::from_fn(|_| AtomicU16::new(0)),
        }
    }

    /// Publish every voice's state. Audio thread; stores only.
    pub(crate) fn publish(&self, voices: &[SamplerVoice; SAMPLER_SLOTS]) {
        for (i, voice) in voices.iter().enumerate() {
            self.playing[i].store(voice.is_playing(), Ordering::Relaxed);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let permyriad = (voice.progress() * PROGRESS_SCALE) as u16;
            self.progress[i].store(permyriad, Ordering::Relaxed);
        }
    }

    /// Whether slot `slot` is sounding. Out of range reads idle.
    #[must_use]
    pub fn is_playing(&self, slot: usize) -> bool {
        self.playing
            .get(slot)
            .is_some_and(|p| p.load(Ordering::Relaxed))
    }

    /// Slot `slot`'s progress, `0.0..=1.0`. Out of range reads `0.0`.
    #[must_use]
    pub fn progress(&self, slot: usize) -> f32 {
        self.progress.get(slot).map_or(0.0, |p| {
            f32::from(p.load(Ordering::Relaxed)) / PROGRESS_SCALE
        })
    }

    /// Back to idle — a fresh engine starts with an empty rack.
    pub fn reset(&self) {
        for i in 0..SAMPLER_SLOTS {
            self.playing[i].store(false, Ordering::Relaxed);
            self.progress[i].store(0, Ordering::Relaxed);
        }
    }
}

/// Linear fade-in over [`RAMP_FRAMES`].
#[allow(clippy::cast_precision_loss)]
fn ramp_in(attack: usize) -> f32 {
    if attack >= RAMP_FRAMES {
        1.0
    } else {
        attack as f32 / RAMP_FRAMES as f32
    }
}

/// Linear fade-out over the final [`RAMP_FRAMES`] frames.
#[allow(clippy::cast_precision_loss)]
fn ramp_out(remaining: usize) -> f32 {
    if remaining >= RAMP_FRAMES {
        1.0
    } else {
        remaining as f32 / RAMP_FRAMES as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(value: f32, frames: usize) -> Arc<Track> {
        Arc::new(Track::from_interleaved(vec![value; frames * 2], 48_000, 2).unwrap())
    }

    fn render(voice: &mut SamplerVoice, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        voice.render_add(&mut out, 2, 0);
        out
    }

    #[test]
    fn an_unloaded_voice_is_silent() {
        let mut voice = SamplerVoice::default();
        voice.trigger(0);
        assert!(!voice.is_playing(), "nothing to trigger");
        assert!(render(&mut voice, 32).iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_trigger_plays_the_sample_through_and_ends() {
        let mut voice = SamplerVoice::default();
        assert!(voice.set_source(track(0.5, 128)).is_none());
        voice.set_gain(1.0);
        voice.trigger(0);
        assert!(voice.is_playing());

        let out = render(&mut voice, 128);
        assert!(out.iter().any(|s| *s > 0.1), "the sample sounded");
        assert!(!voice.is_playing(), "a one-shot ends by itself (§7.1)");

        // And stays ended: no loop, no retrigger.
        assert!(render(&mut voice, 64).iter().all(|s| *s == 0.0));
    }

    /// §7.1: "Output is additive — sample plays *over* whatever Deck
    /// A/B are currently playing."
    #[test]
    fn output_is_added_to_the_bus_not_written_over_it() {
        let mut voice = SamplerVoice::default();
        let _ = voice.set_source(track(0.25, 256));
        voice.set_gain(1.0);
        voice.trigger(0);

        let mut out = vec![0.4f32; 64 * 2];
        voice.render_add(&mut out, 2, 0);
        assert!(
            out.iter().all(|s| *s >= 0.4),
            "the deck's audio must survive underneath"
        );
    }

    #[test]
    fn gain_scales_the_voice_and_clamps() {
        let mut voice = SamplerVoice::default();
        let _ = voice.set_source(track(0.5, 512));
        voice.set_gain(0.5);
        voice.trigger(0);
        // Skip the attack ramp before measuring.
        let _ = render(&mut voice, RAMP_FRAMES);
        let out = render(&mut voice, 16);
        assert!(
            (out[0] - 0.25).abs() < 1e-6,
            "0.5 sample × 0.5 gain, got {}",
            out[0]
        );

        voice.set_gain(-1.0);
        assert!(voice.gain() >= 0.0, "gain cannot invert the phase");
        voice.set_gain(99.0);
        assert!(voice.gain() <= 4.0, "clamped rather than deafening");
    }

    /// Mashing the same pad is how this gets used; a hard restart puts
    /// a step in the output every time.
    #[test]
    fn a_retrigger_crossfades_instead_of_cutting() {
        let mut voice = SamplerVoice::default();
        let _ = voice.set_source(track(0.5, 4096));
        voice.set_gain(1.0);
        voice.trigger(0);
        let _ = render(&mut voice, 512); // well past the attack

        voice.trigger(0);
        let out = render(&mut voice, RAMP_FRAMES);
        // Across the retrigger the envelope is the outgoing tail
        // ramping down plus the new take ramping up — never a jump to
        // silence.
        assert!(
            out.chunks_exact(2).all(|f| f[0] > 0.2),
            "output dipped to a step across the retrigger: {:?}",
            &out[..8]
        );
        assert!(voice.is_playing());
    }

    #[test]
    fn the_head_and_tail_are_ramped() {
        let mut voice = SamplerVoice::default();
        let _ = voice.set_source(track(1.0, RAMP_FRAMES * 4));
        voice.set_gain(1.0);
        voice.trigger(0);

        let out = render(&mut voice, RAMP_FRAMES * 4);
        assert!(out[0].abs() < 0.05, "starts from silence, got {}", out[0]);
        let last = out[out.len() - 2];
        assert!(last.abs() < 0.05, "ends into silence, got {last}");
        let middle = out[RAMP_FRAMES * 2 * 2];
        assert!(
            (middle - 1.0).abs() < 1e-6,
            "unity in the body, got {middle}"
        );
    }

    #[test]
    fn setting_a_new_sample_hands_back_the_old_one_for_disposal() {
        let mut voice = SamplerVoice::default();
        let first = track(0.5, 64);
        let second = track(0.25, 64);
        assert!(voice.set_source(Arc::clone(&first)).is_none());

        let displaced = voice.set_source(second).expect("the old sample comes back");
        assert!(
            Arc::ptr_eq(&displaced, &first),
            "the audio thread must never drop an Arc<Track>"
        );
        assert!(voice.clear_source().is_some());
        assert!(!voice.is_loaded());
    }

    #[test]
    fn stop_ramps_out_rather_than_cutting() {
        let mut voice = SamplerVoice::default();
        let _ = voice.set_source(track(0.8, 4096));
        voice.set_gain(1.0);
        voice.trigger(0);
        let _ = render(&mut voice, 256);

        voice.stop();
        assert!(!voice.is_playing());
        let out = render(&mut voice, RAMP_FRAMES);
        assert!(out[0].abs() > 0.0, "the tail keeps sounding while it fades");
        let last = out[out.len() - 2];
        assert!(last.abs() < 0.05, "and lands on silence, got {last}");
    }

    /// The pad's sweep: idle rests at 0, a take walks to 1 and the
    /// end of the sample puts it back at 0 rather than parking at 1.
    #[test]
    fn progress_walks_the_take_and_rests_at_zero() {
        let mut voice = SamplerVoice::default();
        let _ = voice.set_source(track(0.5, 1024));
        voice.set_gain(1.0);
        assert!(voice.progress().abs() < f32::EPSILON);
        voice.trigger(0);
        let _ = render(&mut voice, 512);
        assert!(
            (voice.progress() - 0.5).abs() < 1e-3,
            "{}",
            voice.progress()
        );
        let _ = render(&mut voice, 512);
        assert!(!voice.is_playing());
        assert!(
            voice.progress().abs() < f32::EPSILON,
            "a finished take is idle, not 100 %"
        );
    }

    /// The master switching decks mid-horn must not hop the sound
    /// across the mixer; the deck is fixed at the press.
    #[test]
    fn a_take_keeps_its_deck_until_retriggered() {
        let mut voice = SamplerVoice::default();
        let _ = voice.set_source(track(0.5, 4096));
        voice.trigger(0);
        assert_eq!(voice.output_deck(), 0);
        voice.trigger(1);
        assert_eq!(voice.output_deck(), 1);
    }

    #[test]
    fn shared_state_publishes_every_voice_and_resets() {
        let mut voices: [SamplerVoice; SAMPLER_SLOTS] = Default::default();
        let _ = voices[2].set_source(track(0.5, 1024));
        voices[2].set_gain(1.0);
        voices[2].trigger(0);
        let _ = render(&mut voices[2], 256);

        let shared = SamplerSharedState::new();
        shared.publish(&voices);
        assert!(shared.is_playing(2));
        assert!(
            (shared.progress(2) - 0.25).abs() < 1e-3,
            "{}",
            shared.progress(2)
        );
        assert!(!shared.is_playing(0));
        assert!(shared.progress(0).abs() < f32::EPSILON);
        assert!(!shared.is_playing(SAMPLER_SLOTS), "out of range reads idle");

        shared.reset();
        assert!(!shared.is_playing(2));
        assert!(shared.progress(2).abs() < f32::EPSILON);
    }

    #[test]
    fn a_voice_renders_into_its_assigned_channel_pair_only() {
        let mut voice = SamplerVoice::default();
        let _ = voice.set_source(track(0.5, 256));
        voice.set_gain(1.0);
        voice.trigger(1);
        assert_eq!(voice.output_deck(), 1);

        // Four channels, deck B's pair at offset 2.
        let mut out = vec![0.0f32; 64 * 4];
        voice.render_add(&mut out, 4, 2);
        assert!(
            out.chunks_exact(4).all(|f| f[0] == 0.0 && f[1] == 0.0),
            "deck A's pair must stay untouched"
        );
        assert!(out.chunks_exact(4).any(|f| f[2] > 0.0));
    }
}
