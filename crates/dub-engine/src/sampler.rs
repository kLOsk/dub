//! One-shot sampler voices (M17, PRD §7.1).
//!
//! Four slots (`A S D F`), each holding a pre-decoded sample. A press
//! plays the sample through and ends: air horns, vocal stabs, dub-siren
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

use std::sync::Arc;

use dub_io::Track;

/// Slots in the rack. §7.1 chose four over Serato's six so the keymap
/// stays symmetric with Quick Scratch's four (`Q W E R`), and because
/// four has covered the target user's drop / siren / horn / vocal-stab
/// workflow. Widening stays on the table for v1.x.
pub const SAMPLER_SLOTS: usize = 4;

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
    /// Linear gain, §7.1 per-slot.
    gain: f32,
    /// Deck bus this voice sums onto (§7.1 "output assignment").
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

    /// Set the per-slot gain. Clamped to `[0, 4]`: a negative value
    /// would invert the phase against the deck it sums into, and the
    /// ceiling keeps a mis-dragged slider off the DJ's ears.
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain.clamp(0.0, 4.0);
    }

    /// Deck output bus this voice sums onto.
    #[must_use]
    pub fn output_deck(&self) -> u8 {
        self.output_deck
    }

    /// Choose the deck bus this voice sums onto (§7.1 "output
    /// assignment"; default deck A).
    pub fn set_output_deck(&mut self, deck: u8) {
        self.output_deck = deck;
    }

    /// Fire the one-shot from the top. Retriggering a sounding voice
    /// hands the current take to the tail so it ramps out under the new
    /// one instead of cutting.
    pub fn trigger(&mut self) {
        if self.source.is_none() {
            return;
        }
        if self.playing {
            self.tail_frame = Some(self.frame);
            self.tail_remaining = RAMP_FRAMES;
        }
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
        voice.trigger();
        assert!(!voice.is_playing(), "nothing to trigger");
        assert!(render(&mut voice, 32).iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_trigger_plays_the_sample_through_and_ends() {
        let mut voice = SamplerVoice::default();
        assert!(voice.set_source(track(0.5, 128)).is_none());
        voice.set_gain(1.0);
        voice.trigger();
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
        voice.trigger();

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
        voice.trigger();
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
        voice.trigger();
        let _ = render(&mut voice, 512); // well past the attack

        voice.trigger();
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
        voice.trigger();

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
        voice.trigger();
        let _ = render(&mut voice, 256);

        voice.stop();
        assert!(!voice.is_playing());
        let out = render(&mut voice, RAMP_FRAMES);
        assert!(out[0].abs() > 0.0, "the tail keeps sounding while it fades");
        let last = out[out.len() - 2];
        assert!(last.abs() < 0.05, "and lands on silence, got {last}");
    }

    #[test]
    fn a_voice_renders_into_its_assigned_channel_pair_only() {
        let mut voice = SamplerVoice::default();
        let _ = voice.set_source(track(0.5, 256));
        voice.set_gain(1.0);
        voice.set_output_deck(1);
        assert_eq!(voice.output_deck(), 1);
        voice.trigger();

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
