//! Commands sent from the UI thread to the audio engine.
//!
//! Every transport/state mutation that needs to happen mid-playback flows
//! through this enum. The producer side lives in [`crate::EngineHandle`]
//! (main thread); the consumer side is drained by [`crate::Engine::render`]
//! at the start of each block.
//!
//! Per PRD §4.2: lock-free SPSC, no allocation on send/receive. Adding a
//! new command means adding an enum variant and its match arm in
//! [`crate::Engine::apply_command`] — that's all.
//!
//! **Heap-bearing variants.** Most commands are tiny `Copy` values
//! (≤ 24 bytes). Three variants carry heap pointers because the work
//! being commanded is fundamentally about handing a heap-allocated
//! resource to the audio thread:
//!
//! - [`Command::DeckLoad`] carries an `Arc<Track>` (PCM samples + metadata).
//! - [`Command::AttachTimecodeInput`] carries a `Box<TimecodeInput>` (M5.4.5;
//!   the box owns the SPSC consumer end of the input ringbuffer plus the
//!   pre-allocated decoder + lift policy).
//! - [`Command::AttachThruSource`] carries a `Box<ThruSource>` (M7;
//!   the box owns the SPSC consumer end of the input ringbuffer plus
//!   the scratch buffer for the Thru render path).
//!
//! The audio thread *never* drops these allocations — when it swaps
//! any onto its slot, any displaced predecessor is bounced back
//! through a corresponding trash channel for disposal on the main thread
//! (see crate-level docs in `lib.rs`). Track trash, TimecodeInput trash,
//! and ThruSource trash are separate ringbufs because their item types
//! differ; all three follow the same overflow-counter "leak rather than
//! drop" pattern.

use std::sync::Arc;

use dub_io::Track;

use crate::thru::ThruSource;
use crate::timecode::TimecodeInput;

/// One mutation request to the engine. Variants name the deck index where
/// applicable; engine-wide commands use no index.
///
/// Field naming is uniform across variants: `idx` is the deck index,
/// other fields name the property being set.
///
/// **Not [`Clone`].** Two variants ([`Self::DeckLoad`],
/// [`Self::AttachTimecodeInput`]) carry uniquely-owned heap resources
/// (`Arc<Track>` and `Box<TimecodeInput>`). Cloning a command would
/// bump the `Arc` refcount silently or fail outright on the `Box`, so
/// we don't derive [`Clone`]; consumers move commands through the
/// SPSC channel as the only path.
///
/// [`Debug`] is hand-written (rather than derived) for the same
/// reason: [`TimecodeInput`] is not [`Debug`] (it owns a decoder + a
/// scratch buffer with no useful Debug rendering); we render a
/// placeholder so the `unreachable!` and trace formatting in
/// [`crate::EngineHandle`] stay usable.
//
// `Deck` prefix on per-deck variants is load-bearing namespacing
// (engine-wide commands such as `SetMasterGain` distinguish themselves
// by *not* having it). Allow the `enum_variant_names` lint accordingly.
#[allow(missing_docs, clippy::enum_variant_names)]
pub enum Command {
    /// Start playback on deck `idx`.
    DeckPlay { idx: u8 },

    /// Pause deck `idx` (playhead does not advance, but the source remains
    /// loaded).
    DeckPause { idx: u8 },

    /// Move deck `idx`'s playhead to the given position in track frames.
    DeckSeek { idx: u8, position_frames: f64 },

    /// Set deck `idx`'s playback rate. `1.0` = normal forward; `-1.0` =
    /// reverse at unity speed; `0.0` = paused without resetting state.
    DeckSetRate { idx: u8, rate: f64 },

    /// Enable / disable key lock (master tempo) on deck `idx` (M14). When on,
    /// the engaged stretcher holds pitch while tempo follows the platter; the
    /// engine auto-bypasses during scratch / reverse / extreme rates.
    DeckSetKeyLock { idx: u8, on: bool },

    /// Select which time-stretch engine deck `idx` uses when key lock engages
    /// (M14 live A/B). `ResamplerOnly` = no key lock (pitch shifts with rate).
    DeckSetStretchBackend {
        idx: u8,
        backend: dub_stretch::StretchBackend,
    },

    /// Engage Panic-Play (M10.6b, PRD §6.1.2) on deck `idx`. The
    /// engine captures the deck's current "last known good"
    /// velocity (preferring `LiftPolicy::last_locked_rate()` if a
    /// timecode input is attached, falling back to the deck's
    /// commanded rate otherwise), forces the policy into a
    /// disengaged state so the next `LiftIntent::Locked` is a
    /// fresh re-engagement, and starts the deck playing at the
    /// captured rate. From this point on the deck ignores
    /// timecode-driven rate / play-state updates (the `Locked` /
    /// `DropoutHoldRate` branches of `apply_timecode_intents`)
    /// until either:
    ///
    /// - the policy reports a clean `LiftIntent::Locked` (carrier
    ///   alive + confidence above the engage threshold), at which
    ///   point panic auto-cancels and normal timecode handling
    ///   resumes — the held playhead position becomes the new zero
    ///   reference for the LFSR's relative motion; or
    /// - the user issues [`Self::DeckCancelPanicPlay`], which
    ///   hands transport authority back to the timecode driver.
    ///
    /// `Locked`-with-cached-rate sticky-window samples don't count
    /// as a clean re-lock because the policy stays disengaged
    /// until it sees an above-engage-threshold confidence sample.
    DeckPanicPlay { idx: u8 },

    /// Cancel Panic-Play on deck `idx` (PRD §6.1.2 / M10.6d). The
    /// engine clears its panic-play flag and hands transport
    /// authority back to the timecode driver: a clean carrier
    /// keeps the deck playing at the platter rate (Serato INT→ABS
    /// path); a silent carrier pauses it on the next block via the
    /// existing `DropoutHoldRate` arm. Crucially this command does
    /// **not** flip `is_playing` itself — the driver does, on the
    /// next render block. Idempotent on decks not in panic mode.
    DeckCancelPanicPlay { idx: u8 },

    /// Set deck `idx`'s linear gain. `1.0` = unity, `0.0` = silence.
    DeckSetGain { idx: u8, gain: f32 },

    /// Engage a loop on deck `idx` over `[in_frames, out_frames)`
    /// (track frames). The reverse-loop region is computed off-RT
    /// (grid-snapped) and sent here; the deck jumps the playhead into
    /// the region if it's outside (the "repeat the bar just heard"
    /// jump-back) and wraps per-block with a seam crossfade. No-op on
    /// an empty / non-finite region.
    DeckSetLoop {
        idx: u8,
        in_frames: f64,
        out_frames: f64,
    },

    /// Disengage any loop on deck `idx`. Playback continues forward
    /// from the current position. Idempotent.
    DeckClearLoop { idx: u8 },

    /// Engage (or re-trigger) the M15 echo-out FX on deck `idx`
    /// (PRD §6.3). All parameters are resolved **off-RT** by the FFI:
    /// `delay_frames` is the echo length on the output bus (engine
    /// sample rate) = `division_beats × 60/bpm × engine_sr`; `feedback`
    /// is the per-lap decay; `lp_coeff` is the one-pole low-pass
    /// coefficient for the feedback path (computed from the cutoff Hz —
    /// the audio thread never calls `exp`). The deck's output is already
    /// warm-captured continuously, so engaging freezes the last beat and
    /// recirculates it with decay. While engaged the dry is muted (100 % wet)
    /// on every deck, Thru included.
    DeckEngageEcho {
        idx: u8,
        delay_frames: u32,
        feedback: f32,
        lp_coeff: f32,
    },

    /// Toggle echo-out off on deck `idx`: restore the (muted) dry signal —
    /// the deck has kept playing underneath, so it resumes at its slipped
    /// position — and fade the wet out. Idempotent on a deck that isn't
    /// engaged.
    DeckReleaseEcho { idx: u8 },

    /// Live-update echo-out `feedback` and feedback low-pass (`lp_coeff`,
    /// resolved off-RT from the cutoff) on deck `idx` while held or idle
    /// (UI sliders). The echo *length* only changes on a fresh
    /// [`Self::DeckEngageEcho`] — changing the beat division re-triggers.
    DeckSetEchoParams {
        idx: u8,
        feedback: f32,
        lp_coeff: f32,
    },

    /// Fire M16 dub-siren preset `preset_id` on deck `idx` (Simple mode,
    /// PRD §6.3). The siren is a *generator* summed onto the deck's output bus
    /// (additive — it sounds with or without a track loaded, and survives the
    /// deck's echo-out dry-mute). The preset bank (siren / alarm / laser / bomb
    /// / gun …) is **precomputed off-RT** at engine construction, so this only
    /// carries the index; the audio thread copies the resolved patch and
    /// re-triggers the voice as a tap one-shot. Out-of-range ids are ignored.
    ///
    /// `delay_frames_override` (`0` = use the preset's own slap-back time)
    /// replaces the echo length when the user has beat-matched the siren echo;
    /// it's resolved off-RT from the deck's tempo by the FFI.
    DeckFireSirenPreset {
        idx: u8,
        preset_id: u8,
        delay_frames_override: u32,
    },

    /// Stop the dub-siren on deck `idx`: the oscillator fades out (release ramp)
    /// while the slap-back tail rings on. Idempotent on a deck whose siren isn't
    /// sounding.
    DeckReleaseSiren { idx: u8 },

    /// Pin deck `idx`'s control mode (the deck-header Internal/Timecode
    /// switch). Sets the user override so auto source-detection won't
    /// change it until [`Self::DeckAutoControlMode`].
    DeckSetControlMode { idx: u8, mode: crate::ControlMode },

    /// Release deck `idx` back to automatic source detection.
    DeckAutoControlMode { idx: u8 },

    /// Start a channel-whitening calibration capture on deck `idx`'s
    /// timecode input (the manual "Recalibrate" affordance). No-op if no
    /// timecode input is attached.
    DeckCalibrateTimecode { idx: u8 },

    /// Load a new track on deck `idx`. The `Arc<Track>` is sent by value;
    /// the engine swaps it onto the deck on the audio thread without
    /// dropping the previous `Arc` — that goes back through the trash
    /// channel.
    ///
    /// `gain` is the linear deck gain to apply atomically with the
    /// swap (auto-gain / loudness normalization, M16). It is the
    /// value resolved **once at load time** from the library's stored
    /// loudness and held for the life of this load — the deck never
    /// re-reads it. `1.0` (unity) is passed for unanalyzed tracks and
    /// the Finder-drag path, so a load always sets a definite gain and
    /// never inherits the previous track's normalization.
    DeckLoad {
        idx: u8,
        source: Arc<Track>,
        gain: f32,
    },

    /// Set the engine-wide master gain applied after deck summing in the
    /// debug internal mixer. `1.0` = unity. PRD §5.3 calls for this only
    /// in the debug/internal mixer mode; external-mixer mode (M5+) bypasses
    /// the master and routes each deck to its own output pair raw.
    SetMasterGain { gain: f32 },

    /// Mid-stream attach of a [`TimecodeInput`] to deck `idx` (M5.4.5).
    /// The box is constructed on the main thread — `TimecodeInput::new`
    /// allocates a scratch buffer and a decoder — and handed across the
    /// command channel as a single 8-byte pointer.
    ///
    /// On the audio thread, [`crate::Engine::apply_command`] takes the
    /// box out of the variant and slots it into
    /// `engine.timecode_inputs[idx]`. If the slot was already occupied
    /// (mid-stream re-calibration after a cartridge swap, M5.4.5+
    /// extension), the *displaced* `Box<TimecodeInput>` is sent back
    /// through the timecode-input trash channel for main-thread
    /// disposal — never dropped on the audio thread.
    ///
    /// Why command-channel attach (vs. the existing `&mut Engine`
    /// [`crate::Engine::attach_timecode_input`]): once the engine has
    /// been moved into `dub_audio::AudioOutput`, no `&mut` access from
    /// the main thread is possible. M5.4.5 needs to attach decks
    /// *while audio is running* (the DJ-takeover use case), so the
    /// attach must route through the SPSC channel like every other
    /// runtime mutation.
    AttachTimecodeInput { idx: u8, input: Box<TimecodeInput> },

    /// Mid-stream attach of a [`ThruSource`] to deck `idx` (M7). The
    /// box is constructed on the main thread —
    /// [`crate::ThruSource::new`] allocates a scratch buffer — and
    /// handed across the command channel as a single 8-byte pointer.
    ///
    /// On the audio thread, [`crate::Engine::apply_command`] takes the
    /// box out of the variant and slots it into
    /// `engine.thru_sources[idx]`. If the slot was already occupied
    /// (the operator re-attaches mid-set, e.g. swaps cartridges or
    /// switches inputs), the *displaced* `Box<ThruSource>` is sent
    /// back through the thru-source trash channel for main-thread
    /// disposal — never dropped on the audio thread.
    ///
    /// Mirrors [`Self::AttachTimecodeInput`]'s shape exactly because
    /// the constraint (heap-bearing payload that the audio thread
    /// can't drop) is the same; M5.4.5's trash-channel pattern
    /// generalises trivially to a third channel.
    ///
    /// Thru Mode in Dub is a single always-on passthrough — there
    /// are no mode commands. FX engagement is handled inside the
    /// per-deck signal chain by individual FX modules (M15+), not
    /// by switching the Thru source between paths. See
    /// `crate::thru` module docs for the design rationale.
    AttachThruSource { idx: u8, source: Box<ThruSource> },
}

impl std::fmt::Debug for Command {
    // One arm per variant: the match is exhaustive and intentionally flat.
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeckPlay { idx } => f.debug_struct("DeckPlay").field("idx", idx).finish(),
            Self::DeckPause { idx } => f.debug_struct("DeckPause").field("idx", idx).finish(),
            Self::DeckSeek {
                idx,
                position_frames,
            } => f
                .debug_struct("DeckSeek")
                .field("idx", idx)
                .field("position_frames", position_frames)
                .finish(),
            Self::DeckSetRate { idx, rate } => f
                .debug_struct("DeckSetRate")
                .field("idx", idx)
                .field("rate", rate)
                .finish(),
            Self::DeckSetKeyLock { idx, on } => f
                .debug_struct("DeckSetKeyLock")
                .field("idx", idx)
                .field("on", on)
                .finish(),
            Self::DeckSetStretchBackend { idx, backend } => f
                .debug_struct("DeckSetStretchBackend")
                .field("idx", idx)
                .field("backend", backend)
                .finish(),
            Self::DeckPanicPlay { idx } => {
                f.debug_struct("DeckPanicPlay").field("idx", idx).finish()
            }
            Self::DeckCancelPanicPlay { idx } => f
                .debug_struct("DeckCancelPanicPlay")
                .field("idx", idx)
                .finish(),
            Self::DeckSetGain { idx, gain } => f
                .debug_struct("DeckSetGain")
                .field("idx", idx)
                .field("gain", gain)
                .finish(),
            Self::DeckSetLoop {
                idx,
                in_frames,
                out_frames,
            } => f
                .debug_struct("DeckSetLoop")
                .field("idx", idx)
                .field("in_frames", in_frames)
                .field("out_frames", out_frames)
                .finish(),
            Self::DeckClearLoop { idx } => {
                f.debug_struct("DeckClearLoop").field("idx", idx).finish()
            }
            Self::DeckEngageEcho {
                idx,
                delay_frames,
                feedback,
                lp_coeff,
            } => f
                .debug_struct("DeckEngageEcho")
                .field("idx", idx)
                .field("delay_frames", delay_frames)
                .field("feedback", feedback)
                .field("lp_coeff", lp_coeff)
                .finish(),
            Self::DeckReleaseEcho { idx } => {
                f.debug_struct("DeckReleaseEcho").field("idx", idx).finish()
            }
            Self::DeckSetEchoParams {
                idx,
                feedback,
                lp_coeff,
            } => f
                .debug_struct("DeckSetEchoParams")
                .field("idx", idx)
                .field("feedback", feedback)
                .field("lp_coeff", lp_coeff)
                .finish(),
            Self::DeckFireSirenPreset {
                idx,
                preset_id,
                delay_frames_override,
            } => f
                .debug_struct("DeckFireSirenPreset")
                .field("idx", idx)
                .field("preset_id", preset_id)
                .field("delay_frames_override", delay_frames_override)
                .finish(),
            Self::DeckReleaseSiren { idx } => f
                .debug_struct("DeckReleaseSiren")
                .field("idx", idx)
                .finish(),
            Self::DeckSetControlMode { idx, mode } => f
                .debug_struct("DeckSetControlMode")
                .field("idx", idx)
                .field("mode", mode)
                .finish(),
            Self::DeckAutoControlMode { idx } => f
                .debug_struct("DeckAutoControlMode")
                .field("idx", idx)
                .finish(),
            Self::DeckCalibrateTimecode { idx } => f
                .debug_struct("DeckCalibrateTimecode")
                .field("idx", idx)
                .finish(),
            Self::DeckLoad { idx, gain, .. } => f
                .debug_struct("DeckLoad")
                .field("idx", idx)
                .field("source", &"<Arc<Track>>")
                .field("gain", gain)
                .finish(),
            Self::SetMasterGain { gain } => {
                f.debug_struct("SetMasterGain").field("gain", gain).finish()
            }
            Self::AttachTimecodeInput { idx, .. } => f
                .debug_struct("AttachTimecodeInput")
                .field("idx", idx)
                .field("input", &"<Box<TimecodeInput>>")
                .finish(),
            Self::AttachThruSource { idx, .. } => f
                .debug_struct("AttachThruSource")
                .field("idx", idx)
                .field("source", &"<Box<ThruSource>>")
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_is_send_and_bounded() {
        // The non-Copy variants (DeckLoad's Arc<Track>, M5.4.5's
        // AttachTimecodeInput's Box<TimecodeInput>) make the enum
        // non-Copy, but each only adds an 8-byte pointer to the
        // payload.
        //
        // We require `Send` only — the SPSC channel moves a command
        // from the producer thread to the consumer thread by value,
        // never sharing &Command across threads. We dropped the
        // `Sync` bound when M5.4.5 added `Box<TimecodeInput>`: the
        // inner `HeapCons<f32>` is intentionally `!Sync` (an SPSC
        // consumer is unsound to access from two threads at once),
        // and `Sync` was never actually needed by the channel
        // contract.
        const _: fn() = || {
            fn assert_send<T: Send>() {}
            assert_send::<Command>();
        };
        // ~32 bytes today. The heap-bearing variants (DeckLoad's Arc<Track>,
        // AttachTimecodeInput / AttachThruSource's Box) each add only an 8-byte
        // pointer; the siren fires by a 1-byte preset id (the resolved patch
        // bank lives in the engine, not the command). Cap at 64 to catch
        // accidental bloat — push anything larger through indirection.
        assert!(
            std::mem::size_of::<Command>() <= 64,
            "Command grew to {} bytes; consider redesigning",
            std::mem::size_of::<Command>()
        );
    }
}
