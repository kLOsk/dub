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

/// Which slot in the per-deck vintage-FX rack a [`Command::DeckSetRackFx`]
/// targets. `repr(u8)` so the FFI maps a Swift enum onto it 1:1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FxSlot {
    /// Spring reverb (the dub tank). Additive send.
    Spring = 0,
    /// Roland RE-201 Space Echo (tape echo + onboard spring). Additive send.
    SpaceEcho = 1,
    /// King Tubby "Big Knob" high-pass filter. In-place insert.
    BigKnob = 2,
    /// Mu-Tron Bi-Phase phaser. In-place insert.
    Phaser = 3,
}

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

    /// Fire dub-siren shot `preset_id` on deck `idx` (PRD §6.3). The siren
    /// is a *generator* summed onto the deck's output bus (additive — it
    /// sounds with or without a track loaded, and survives the deck's
    /// echo-out dry-mute). The bank ([`crate::SIREN_BANK`]) names which chip
    /// each shot plays on; every patch is **precomputed off-RT** at engine
    /// construction, so this only carries the index and the audio thread
    /// copies the resolved patch and re-triggers that voice as a tap
    /// one-shot. Out-of-range ids are ignored.
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

    /// Set the vintage-FX rack slot `slot` on deck `idx`: toggle it `active`
    /// and apply its Advanced super-knob (`macro_value`, 0..1). The macro is
    /// RT-safe to apply (each effect precomputes its transcendental
    /// coefficients off-RT into a LUT/constant), so the audio thread just fans
    /// it across the effect's params. Spring + Space Echo are additive sends;
    /// Big Knob + Phaser are in-place inserts.
    DeckSetRackFx {
        idx: u8,
        slot: FxSlot,
        active: bool,
        macro_value: f32,
    },

    /// The DUB FX channel's input kind on deck `idx` (F-38 stage 3):
    /// `mono` sums the input pair to both channels (a MIC on one side of
    /// the pair), `false` passes the pair as it comes (the mixer's SEND).
    DeckSetFxInputMono { idx: u8, mono: bool },

    /// Engage or bypass rack slot `slot` on deck `idx` *without* touching its
    /// controls — the DUB FX channel's IN/OUT toggle. [`Self::DeckSetRackFx`]
    /// re-applies the Advanced macro on every call, which would stomp the
    /// Expert knobs; this one is the switch alone.
    DeckSetRackSlotActive { idx: u8, slot: FxSlot, active: bool },

    /// Expert Big Knob on deck `idx` (the DUB FX channel, PRD §6.3): snap the
    /// cutoff to detent `step` (0..=10 over [`dub_dsp::BIG_KNOB_STEPS`]) and set
    /// the resonance `q` (0.5..8). Both pure on the audio thread — the detent
    /// coefficients are a table built at construction.
    DeckSetRackBigKnob { idx: u8, step: u8, q: f32 },

    /// Expert phaser on deck `idx`: LFO `rate_hz`, sweep `depth`, resonance
    /// `feedback` and dry/wet `mix` (all 0..1 except the rate). Pure setters.
    DeckSetRackPhaser {
        idx: u8,
        rate_hz: f32,
        depth: f32,
        feedback: f32,
        mix: f32,
    },

    /// Expert Space Echo on deck `idx`: the `mode` selector position (dial
    /// index into [`dub_dsp::Re201Mode::ALL`]), the longest head's
    /// `repeat_ms`, `intensity` (feedback; > ~1 self-oscillates),
    /// `echo_volume`, the onboard `reverb` wet and `wow_flutter` (tape age).
    /// All pure on the audio thread — ms→samples is a multiply and the
    /// reverb's coefficients are fixed at construction.
    DeckSetRackSpaceEcho {
        idx: u8,
        mode: u8,
        repeat_ms: f32,
        intensity: f32,
        echo_volume: f32,
        reverb: f32,
        wow_flutter: f32,
    },

    /// Expert spring on deck `idx`: `decay` (tail feedback), `tone` (0 dark →
    /// 1 bright, from the tank's tone table) and `wet`. Pure setters.
    DeckSetRackSpring {
        idx: u8,
        decay: f32,
        tone: f32,
        wet: f32,
    },

    /// Kick the spring tank on deck `idx` — Tubby's thunder: a short impulse
    /// into the springs at `level` (0..1). Pure (arms a sample counter).
    DeckKickSpring { idx: u8, level: f32 },

    /// Set the dub-siren's live controls on deck `idx` (PRD §6.3). The siren is
    /// a self-contained instrument: `speed` is the HK628 chip-clock (pitch +
    /// timing); the rest drive its onboard PT2399 echo — `delay_ms`/`feedback`/
    /// `mix`, the DS01E **FILTER** (`filter`, 0 = dark LP · 0.5 open · 1 thin HP)
    /// and **ECHO CUT** (`echo_cut` momentarily mutes the wet, loop keeps
    /// running); `volume` is the siren output level. All applied with pure
    /// setters on the audio thread (the FILTER cutoff comes from a LUT). The
    /// echo render is skipped when `mix` is ~0. Both the Advanced super-knob
    /// (FFI fans one macro into these) and Expert (individual knobs) route here.
    DeckSetSirenControls {
        idx: u8,
        speed: f32,
        delay_ms: f32,
        feedback: f32,
        mix: f32,
        volume: f32,
        filter: f32,
        echo_cut: bool,
    },

    /// Set the Benidub DS01E voice controls on deck `idx`: `pitch_factor`
    /// (PITCH selector — multiplies the MODE's base frequency), `rate_hz` (RATE
    /// selector — the LFO modulation rate), and `continuous` (TRIGGER down =
    /// latched sustain vs up = momentary one-shot). Applied to the precomputed
    /// MODE patch at fire time; pure field tweaks, RT-safe.
    DeckSetSirenVoice {
        idx: u8,
        pitch_factor: f32,
        rate_hz: f32,
        continuous: bool,
    },

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

    /// Bind a sample to sampler slot `slot` (M17, PRD §7.1).
    ///
    /// Carries an `Arc<Track>` for the same reason [`Self::DeckLoad`]
    /// does, and with the same contract: the audio thread installs it
    /// and bounces whatever it displaced back through the trash
    /// channel. The sample arrives already at the engine rate — the
    /// conversion happens off-RT at bind time (`dub_io::resample_track`),
    /// so the voice reads it with an integer cursor.
    SamplerLoad { slot: u8, source: Arc<Track> },

    /// Unbind sampler slot `slot`, returning its sample for disposal
    /// off the audio thread.
    SamplerClear { slot: u8 },

    /// Fire sampler slot `slot`'s one-shot (§7.1) onto deck `deck`'s
    /// output bus — the master deck, chosen by the shell at the press,
    /// or [`crate::sampler::SAMPLER_OUTPUT_BOTH`] for both. Retriggering
    /// a sounding voice crossfades rather than cutting.
    SamplerTrigger { slot: u8, deck: u8 },

    /// Stop sampler slot `slot` before its sample ends, ramping out.
    SamplerStop { slot: u8 },

    /// Slot `slot`'s linear gain — the auto-gain the shell measured at
    /// bind time (§7.1).
    SamplerSetGain { slot: u8, gain: f32 },

    /// Quick Scratch (PRD §7.2): put sampler slot `slot`'s sample on
    /// deck `deck` at 0 and park what the deck was playing — doubling
    /// it onto `double_to` first, when the shell asks (the tune was on
    /// air and the other deck idle), so it keeps playing there. Engaging
    /// on a deck already engaged swaps the sample and leaves the park
    /// alone. A slot with nothing loaded does nothing.
    DeckQuickScratchEngage {
        deck: u8,
        slot: u8,
        double_to: Option<u8>,
    },

    /// Quick Scratch release: put the parked track back — in time if
    /// the deck was playing when it was parked, at the parked frame if
    /// it was not. No-op when nothing is engaged.
    DeckQuickScratchRelease { deck: u8 },

    /// Instant Doubles (M17, PRD §7.3): put the track loaded on deck
    /// `from` onto deck `to` at `from`'s current playhead, for
    /// juggling.
    ///
    /// Carries no payload because it needs none — the source is
    /// already an `Arc<Track>` on the audio thread, so duplicating it
    /// is a refcount bump rather than a decode, and reading both decks
    /// inside one command application is what makes the alignment
    /// sample-accurate. Routing it through the load path off-RT would
    /// re-decode the file and land the playhead wherever the deck had
    /// drifted to by the time it finished.
    ///
    /// The displaced source on `to` leaves through the same de-click →
    /// trash path as [`Self::DeckLoad`]; the audio thread never drops
    /// an `Arc<Track>`.
    DeckInstantDouble { from: u8, to: u8 },

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
            Self::SamplerLoad { slot, .. } => {
                f.debug_struct("SamplerLoad").field("slot", slot).finish()
            }
            Self::SamplerClear { slot } => {
                f.debug_struct("SamplerClear").field("slot", slot).finish()
            }
            Self::SamplerTrigger { slot, deck } => f
                .debug_struct("SamplerTrigger")
                .field("slot", slot)
                .field("deck", deck)
                .finish(),
            Self::SamplerStop { slot } => {
                f.debug_struct("SamplerStop").field("slot", slot).finish()
            }
            Self::SamplerSetGain { slot, gain } => f
                .debug_struct("SamplerSetGain")
                .field("slot", slot)
                .field("gain", gain)
                .finish(),
            Self::DeckQuickScratchEngage {
                deck,
                slot,
                double_to,
            } => f
                .debug_struct("DeckQuickScratchEngage")
                .field("deck", deck)
                .field("slot", slot)
                .field("double_to", double_to)
                .finish(),
            Self::DeckQuickScratchRelease { deck } => f
                .debug_struct("DeckQuickScratchRelease")
                .field("deck", deck)
                .finish(),
            Self::DeckInstantDouble { from, to } => f
                .debug_struct("DeckInstantDouble")
                .field("from", from)
                .field("to", to)
                .finish(),
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
            Self::DeckSetRackFx {
                idx,
                slot,
                active,
                macro_value,
            } => f
                .debug_struct("DeckSetRackFx")
                .field("idx", idx)
                .field("slot", slot)
                .field("active", active)
                .field("macro_value", macro_value)
                .finish(),
            Self::DeckSetFxInputMono { idx, mono } => f
                .debug_struct("DeckSetFxInputMono")
                .field("idx", idx)
                .field("mono", mono)
                .finish(),
            Self::DeckSetRackSlotActive { idx, slot, active } => f
                .debug_struct("DeckSetRackSlotActive")
                .field("idx", idx)
                .field("slot", slot)
                .field("active", active)
                .finish(),
            Self::DeckSetRackBigKnob { idx, step, q } => f
                .debug_struct("DeckSetRackBigKnob")
                .field("idx", idx)
                .field("step", step)
                .field("q", q)
                .finish(),
            Self::DeckSetRackPhaser {
                idx,
                rate_hz,
                depth,
                feedback,
                mix,
            } => f
                .debug_struct("DeckSetRackPhaser")
                .field("idx", idx)
                .field("rate_hz", rate_hz)
                .field("depth", depth)
                .field("feedback", feedback)
                .field("mix", mix)
                .finish(),
            Self::DeckSetRackSpaceEcho {
                idx,
                mode,
                repeat_ms,
                intensity,
                echo_volume,
                reverb,
                wow_flutter,
            } => f
                .debug_struct("DeckSetRackSpaceEcho")
                .field("idx", idx)
                .field("mode", mode)
                .field("repeat_ms", repeat_ms)
                .field("intensity", intensity)
                .field("echo_volume", echo_volume)
                .field("reverb", reverb)
                .field("wow_flutter", wow_flutter)
                .finish(),
            Self::DeckSetRackSpring {
                idx,
                decay,
                tone,
                wet,
            } => f
                .debug_struct("DeckSetRackSpring")
                .field("idx", idx)
                .field("decay", decay)
                .field("tone", tone)
                .field("wet", wet)
                .finish(),
            Self::DeckKickSpring { idx, level } => f
                .debug_struct("DeckKickSpring")
                .field("idx", idx)
                .field("level", level)
                .finish(),
            Self::DeckSetSirenControls {
                idx,
                speed,
                delay_ms,
                feedback,
                mix,
                volume,
                filter,
                echo_cut,
            } => f
                .debug_struct("DeckSetSirenControls")
                .field("idx", idx)
                .field("speed", speed)
                .field("delay_ms", delay_ms)
                .field("feedback", feedback)
                .field("mix", mix)
                .field("volume", volume)
                .field("filter", filter)
                .field("echo_cut", echo_cut)
                .finish(),
            Self::DeckSetSirenVoice {
                idx,
                pitch_factor,
                rate_hz,
                continuous,
            } => f
                .debug_struct("DeckSetSirenVoice")
                .field("idx", idx)
                .field("pitch_factor", pitch_factor)
                .field("rate_hz", rate_hz)
                .field("continuous", continuous)
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
