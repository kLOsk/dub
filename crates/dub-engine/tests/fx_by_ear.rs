//! The rig checklist's by-ear DUB FX items (section 6), measured offline.
//!
//! Those items were `hands` because a click or a zipper is heard, not seen,
//! and nothing on the rig's loop cable reaches the FX deck's input. Here the
//! channel is driven the way the app drives it — the engine's command
//! handle, a send arriving on deck A's input pair, the deck on DUB FX — and
//! each test measures what the ear was listening for:
//!
//! - a click is a corner sharper than anything the music itself has: the
//!   largest second difference of the output, against the same unit's
//!   steady state in and out;
//! - a blow-up is a peak past the level a unit can reach at its settings;
//! - a ghost is sound from before the DJ switched a unit out.
//!
//! The send is a low tone: its own corners are tiny (A·(2πf/sr)² ≈ 2e-4),
//! so any step the rack adds stands out by orders of magnitude.

use dub_engine::{ControlMode, Engine, EngineHandle, FxSlot, RealtimeContext, TimecodeInputConfig};
use ringbuf::traits::{Producer as _, Split as _};
use ringbuf::{HeapProd, HeapRb};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

/// The send: a 220 Hz tone at an aux send's level.
const TONE_HZ: f32 = 220.0;
const TONE_LEVEL: f32 = 0.25;

/// What the app pushes for each unit when the deck becomes DUB FX
/// (`FxRackControls` defaults), apart from where a test says otherwise.
const SPACE_ECHO_TRIPLE_REVERB: u8 = 6;

struct FxChannel {
    engine: Engine,
    handle: EngineHandle,
    send: HeapProd<f32>,
    rt_out: Vec<f32>,
    phase: f32,
    /// The mixer's send: the tone, or nothing.
    sending: bool,
    /// The last two samples rendered, so a window can start across a join.
    last: [f32; 2],
}

impl FxChannel {
    fn new() -> Self {
        let (mut engine, mut handle) = Engine::new_with_handle(SR, BLOCK);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ring = HeapRb::<f32>::new((SR as usize) * 2);
        let (send, rx) = ring.split();
        let cfg = TimecodeInputConfig {
            format: dub_timecode::Format::SeratoCv02,
            detect_vinyl: false,
            count_synthetic: false,
            input_sample_rate: SR,
            max_block_frames: BLOCK,
            confidence_threshold: 0.7,
            disengage_threshold: 0.5,
            sticky_blocks_to_disengage: 1,
            amplitude_threshold: 0.001,
            hold_until_calibrated: false,
        };
        engine
            .attach_timecode_input(0, rx, cfg)
            .expect("deck A takes the send");
        handle
            .deck(0)
            .set_control_mode(ControlMode::Fx)
            .expect("deck A to DUB FX");
        let mut ch = Self {
            engine,
            handle,
            send,
            rt_out: vec![0.0; BLOCK * 2],
            phase: 0.0,
            sending: true,
            last: [0.0; 2],
        };
        ch.default_controls();
        ch
    }

    /// The channel's starting controls, every unit out.
    fn default_controls(&mut self) {
        let h = &mut self.handle;
        h.deck(0).set_rack_big_knob(3, 0.7).unwrap();
        h.deck(0).set_rack_phaser(0.4, 0.7, 0.45, 0.5).unwrap();
        h.deck(0)
            .set_rack_space_echo(SPACE_ECHO_TRIPLE_REVERB, 420.0, 0.62, 0.6, 0.3, 0.3)
            .unwrap();
        h.deck(0).set_rack_spring(0.6, 0.5, 0.35).unwrap();
    }

    fn switch(&mut self, slot: FxSlot, on: bool) {
        self.handle.deck(0).set_rack_slot_active(slot, on).unwrap();
    }

    /// Render `secs`; the deck's left output, sample by sample.
    fn run(&mut self, secs: f32) -> Vec<f32> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let blocks = ((secs * SR) as usize).div_ceil(BLOCK);
        self.run_blocks(blocks, |_, _| {})
    }

    /// Render `secs` after a change, led by the two samples before it: a
    /// step at the join is a corner in the result.
    fn run_across(&mut self, secs: f32) -> Vec<f32> {
        let mut joined = self.last.to_vec();
        joined.extend(self.run(secs));
        joined
    }

    /// Render `blocks` blocks, calling `each(self, block)` before each.
    fn run_blocks(&mut self, blocks: usize, mut each: impl FnMut(&mut Self, usize)) -> Vec<f32> {
        let mut left = Vec::with_capacity(blocks * BLOCK);
        let mut input = [0.0_f32; BLOCK * 2];
        let step = std::f32::consts::TAU * TONE_HZ / SR;
        for b in 0..blocks {
            each(self, b);
            for frame in input.chunks_exact_mut(2) {
                let s = if self.sending {
                    self.phase.sin() * TONE_LEVEL
                } else {
                    0.0
                };
                self.phase = (self.phase + step) % std::f32::consts::TAU;
                frame[0] = s;
                frame[1] = s;
            }
            self.send.push_slice(&input);
            let mut rt = RealtimeContext::new();
            self.engine.render(&mut rt, &mut self.rt_out);
            left.extend(self.rt_out.chunks_exact(2).map(|f| f[0]));
        }
        if let [.., a, b] = left[..] {
            self.last = [a, b];
        }
        left
    }
}

/// A step under this is no click: −60 dBFS, under the hiss of any send a
/// sound system lets through. The tone's own corners sit far below it, so
/// where a unit's steady sound is quieter still, this is the bar.
const CLICK_FLOOR: f32 = 1e-3;

/// What a change may add: twice the steady sound's sharpest corner, and
/// never less than [`CLICK_FLOOR`].
fn click_bar(steady: f32) -> f32 {
    (2.0 * steady).max(CLICK_FLOOR)
}

/// The sharpest corner in `x`: its largest second difference.
fn sharpest(x: &[f32]) -> f32 {
    x.windows(3)
        .map(|w| (w[0] - 2.0 * w[1] + w[2]).abs())
        .fold(0.0, f32::max)
}

/// The harshest step in `x`: its largest fourth difference. A step of `j`
/// reads up to 3·j; a tone of amplitude `a` at `f` Hz reads a·(2πf/sr)⁴,
/// 4e-7 for the send and still under 1e-4 four times higher.
fn harshest(x: &[f32]) -> f32 {
    x.windows(5)
        .map(|w| (w[0] - 4.0 * w[1] + 6.0 * w[2] - 4.0 * w[3] + w[4]).abs())
        .fold(0.0, f32::max)
}

fn rms_db(x: &[f32]) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let mean = x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32;
    10.0 * mean.max(1e-20).log10()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
}

fn secs(n: f32) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (n * SR) as usize;
    frames
}

const SLOTS: [(FxSlot, &str); 4] = [
    (FxSlot::BigKnob, "Big Knob"),
    (FxSlot::Phaser, "phaser"),
    (FxSlot::SpaceEcho, "Space Echo"),
    (FxSlot::Spring, "Spring"),
];

/// Rig 6: "Each unit in and out (bat toggle and its key) → audible, no
/// click." Eight switches per unit, each landing at a different point of
/// the tone's cycle; the 30 ms after each may be no sharper than the
/// unit's own steady sound, in or out.
#[test]
fn each_unit_switches_in_and_out_without_a_click() {
    let mut report = Vec::new();
    for (slot, name) in SLOTS {
        let mut ch = FxChannel::new();
        // The Big Knob on a detent that takes the tone well down, so in
        // and out sound different (the default 240 Hz detent sits on it).
        ch.handle.deck(0).set_rack_big_knob(6, 0.7).unwrap();
        let out = ch.run(1.0);
        let steady_out = sharpest(&out[secs(0.5)..]);
        ch.switch(slot, true);
        let in_ = ch.run(3.0);
        let steady_in = sharpest(&in_[secs(1.5)..]);
        let steady = steady_out.max(steady_in);

        let mut worst = 0.0_f32;
        let mut on = true;
        for k in 0..8 {
            on = !on;
            ch.switch(slot, on);
            let after = ch.run_across(0.03);
            worst = worst.max(sharpest(&after));
            // Uneven gaps put every switch at a different phase.
            ch.run(0.4 + 0.0011 * k as f32);
        }
        report.push(format!(
            "{name}: steady {steady:.5} (in {steady_in:.5}, out {steady_out:.5}), worst switch {worst:.5} = {:.1}×",
            worst / steady
        ));
        if worst > click_bar(steady) {
            report.push(format!("  ^ {name} clicks"));
        }
    }
    let text = report.join("\n");
    println!("{text}");
    assert!(!text.contains("clicks"), "{text}");
}

/// A unit switched out and back in later starts from what it hears now:
/// the Space Echo does not play back a phrase from before it went out,
/// nor does the spring.
#[test]
fn a_unit_switched_back_in_has_no_ghost_of_what_it_held() {
    for (slot, name) in [
        (FxSlot::SpaceEcho, "Space Echo"),
        (FxSlot::Spring, "Spring"),
    ] {
        let mut ch = FxChannel::new();
        ch.switch(slot, true);
        ch.run(1.0);
        ch.switch(slot, false);
        ch.sending = false;
        ch.run(8.0);
        ch.switch(slot, true);
        let back = ch.run(1.0);
        let ghost = peak(&back);
        assert!(
            ghost < TONE_LEVEL * 0.01,
            "{name}: switched back in 8 s after the send stopped, it played {ghost:.4} \
             (the send was {TONE_LEVEL})"
        );
    }
}

/// KICK with the spring out is not saved up for later: switching the
/// spring in afterwards brings no thunder with it.
#[test]
fn a_kick_with_the_spring_out_does_not_wait_for_it() {
    let mut ch = FxChannel::new();
    ch.sending = false;
    ch.run(0.2);
    ch.handle.deck(0).kick_spring(1.0).unwrap();
    ch.run(5.0);
    ch.switch(FxSlot::Spring, true);
    let later = ch.run(1.0);
    let thunder = peak(&later);
    assert!(
        thunder < 0.001,
        "the kick from 5 s ago thundered when the spring went in: {thunder:.4}"
    );
}

/// Rig 6: "Spring: KICK → thunder, every time." Ten kicks, a second and a
/// half apart (the tail of one still ringing into the next): each one
/// lands within 3 dB of the first, and well clear of silence.
#[test]
fn the_spring_thunders_on_every_kick() {
    let mut ch = FxChannel::new();
    ch.sending = false;
    ch.switch(FxSlot::Spring, true);
    ch.run(0.2);
    let mut levels = Vec::new();
    for _ in 0..10 {
        ch.handle.deck(0).kick_spring(1.0).unwrap();
        let tail = ch.run(1.5);
        levels.push(rms_db(&tail[..secs(0.5)]));
    }
    let first = levels[0];
    println!("kicks, dB RMS over 0.5 s: {levels:.1?}");
    assert!(
        first > -60.0,
        "the first kick is barely there: {first:.1} dB"
    );
    for (i, l) in levels.iter().enumerate() {
        assert!(
            (l - first).abs() < 3.0,
            "kick {} at {l:.1} dB, the first at {first:.1} dB: {levels:?}",
            i + 1
        );
    }
}

/// Rig 6: "Big Knob through all 11 detents with Q up → no zipper, no
/// blow-up." Q at the knob's top (8), the detents clicked through up and
/// down at a quick hand's pace (80 ms each), twice. Blow-up: louder than
/// the filter can be — its gain at resonance is Q, reached when the
/// gliding cutoff crosses the tone (no detent sits on 220 Hz, so held
/// still it is never quite there). Zipper: a step, on the fourth
/// difference, so the resonance ringing as the cutoff moves (at −80 dB and
/// below) does not count.
#[test]
fn the_big_knob_steps_through_its_detents_at_full_q_without_zipper_or_blow_up() {
    let mut ch = FxChannel::new();
    ch.handle.deck(0).set_rack_big_knob(0, 8.0).unwrap();
    ch.switch(FxSlot::BigKnob, true);
    ch.run(1.0);
    let mut ring = 0.0_f32;
    let mut steady = 0.0_f32;
    for d in 0..=10_u8 {
        ch.handle.deck(0).set_rack_big_knob(d, 8.0).unwrap();
        let held = ch.run(1.0);
        ring = ring.max(peak(&held[secs(0.5)..]));
        steady = steady.max(harshest(&held[secs(0.5)..]));
    }

    let detents: Vec<u8> = (0..=10)
        .chain((0..=10).rev())
        .chain(0..=10)
        .chain((0..=10).rev())
        .collect();
    let per = secs(0.08).div_ceil(BLOCK);
    let swept = ch.run_blocks(detents.len() * per, |ch, b| {
        if b % per == 0 {
            ch.handle
                .deck(0)
                .set_rack_big_knob(detents[b / per], 8.0)
                .unwrap();
        }
    });
    assert!(swept.iter().all(|s| s.is_finite()));
    let top = peak(&swept);
    let corner = harshest(&swept);
    println!(
        "held: loudest {ring:.3}, harshest {steady:.6}; swept: peak {top:.3}, harshest {corner:.6}"
    );
    let most = 8.0 * TONE_LEVEL;
    assert!(
        top <= most,
        "the sweep peaked at {top:.3}, past the filter's own gain at Q 8 ({most:.3})"
    );
    assert!(
        corner <= click_bar(steady),
        "the sweep zippered: {corner:.6} against {steady:.6} held still"
    );
}

/// Rig 6: "Space Echo: every mode" — the selector turned through all seven
/// positions while the echoes ring, then back, and no turn clicks.
#[test]
fn the_space_echo_selector_turns_without_a_click() {
    let mut ch = FxChannel::new();
    ch.switch(FxSlot::SpaceEcho, true);
    let settled = ch.run(3.0);
    let mut steady = sharpest(&settled[secs(1.5)..]);
    let order: Vec<u8> = (0..7).chain((0..7).rev()).collect();
    let mut worst = 0.0_f32;
    for mode in order {
        ch.handle
            .deck(0)
            .set_rack_space_echo(mode, 420.0, 0.62, 0.6, 0.3, 0.3)
            .unwrap();
        let after = ch.run_across(0.03);
        worst = worst.max(sharpest(&after));
        let held = ch.run(1.5);
        steady = steady.max(sharpest(&held[secs(1.0)..]));
    }
    println!("steady {steady:.5}, worst turn {worst:.5}");
    assert!(
        worst <= click_bar(steady),
        "a selector turn clicked: {worst:.5} against {steady:.5} held still"
    );
}

/// Rig 6: "Repeat knob swept → the repeats bend like tape, no clicks."
/// The knob turned end to end and back over two seconds, a new value
/// every block, as a drag sends them. A bend changes the tone's pitch,
/// which the second difference reads as sharper corners; a click is a
/// step, which the fourth difference reads at its full size while a tone
/// even four times higher stays orders of magnitude under it.
#[test]
fn the_space_echo_repeat_bends_without_a_click() {
    let mut ch = FxChannel::new();
    ch.switch(FxSlot::SpaceEcho, true);
    let settled = ch.run(3.0);
    let steady = harshest(&settled[secs(1.5)..]);
    let blocks = secs(2.0).div_ceil(BLOCK);
    let swept = ch.run_blocks(blocks, |ch, b| {
        #[allow(clippy::cast_precision_loss)]
        let t = b as f32 / blocks as f32;
        let ms = 20.0 + 730.0 * (1.0 - (2.0 * t - 1.0).abs());
        ch.handle
            .deck(0)
            .set_rack_space_echo(SPACE_ECHO_TRIPLE_REVERB, ms, 0.62, 0.6, 0.3, 0.3)
            .unwrap();
    });
    let corner = harshest(&swept);
    println!("held {steady:.6}, swept {corner:.6}");
    assert!(
        corner <= steady * 4.0,
        "the repeat sweep clicked: {corner:.6} against {steady:.6} held still"
    );
}

/// Rig 6: "intensity past 1.0 → RUNAWAY, bounded." At the knob's top
/// (1.2), a phrase thrown in and the send stopped: the tape keeps itself
/// going and, after climbing for about forty seconds, holds a ceiling. One
/// head, two, and all three with the reverb (the loudest).
///
/// The ceilings sit past full scale at the default echo volume (Long
/// ≈ 1.0, Triple ≈ 1.5, Triple + reverb ≈ 1.9): bounded, but the
/// interface clips them. Left for the ear (`docs/history/CURRENT.md`).
#[test]
fn runaway_keeps_going_and_holds_a_ceiling() {
    let mut report = Vec::new();
    let mut failed = false;
    for mode in [1_u8, 2, 6] {
        let mut ch = FxChannel::new();
        ch.handle
            .deck(0)
            .set_rack_space_echo(mode, 420.0, 1.2, 0.6, 0.3, 0.3)
            .unwrap();
        ch.switch(FxSlot::SpaceEcho, true);
        ch.run(1.0);
        ch.sending = false;
        let tail = ch.run(60.0);
        let finite = tail.iter().all(|s| s.is_finite());
        let middle = peak(&tail[secs(40.0)..secs(50.0)]);
        let last = peak(&tail[secs(50.0)..]);
        let ok = finite && last > TONE_LEVEL * 0.1 && last <= middle * 1.05;
        failed |= !ok;
        report.push(format!(
            "mode {mode}: 40-50 s {middle:.3}, 50-60 s {last:.3}{}",
            if ok { "" } else { "  <- not a bounded runaway" }
        ));
    }
    let text = report.join("\n");
    println!("{text}");
    assert!(!failed, "{text}");
}

/// Every continuous knob on the rack dragged end to end and back over two
/// seconds, at the ~60 values a second a drag sends: no zipper. The bar is
/// the knob's own sound held still at five positions along its travel,
/// since a louder setting has sharper corners of its own. (Taken in
/// steps, nine of these read 4–134× over it; the Repeat knob and the Big
/// Knob's detents have tests of their own above.)
#[test]
fn every_knob_drags_without_a_zipper() {
    let per = secs(1.0 / 60.0).div_ceil(BLOCK).max(1);
    let knobs: [(&str, FxSlot); 12] = [
        ("echo volume", FxSlot::SpaceEcho),
        ("intensity", FxSlot::SpaceEcho),
        ("echo reverb", FxSlot::SpaceEcho),
        ("wow flutter", FxSlot::SpaceEcho),
        ("spring wet", FxSlot::Spring),
        ("spring decay", FxSlot::Spring),
        ("spring tone", FxSlot::Spring),
        ("phaser mix", FxSlot::Phaser),
        ("phaser depth", FxSlot::Phaser),
        ("phaser rate", FxSlot::Phaser),
        ("phaser feedback", FxSlot::Phaser),
        ("big knob q", FxSlot::BigKnob),
    ];
    let mut report = Vec::new();
    for (name, slot) in knobs {
        let set = |ch: &mut FxChannel, t: f32| {
            let d = ch.handle.deck(0);
            match name {
                "echo volume" => d.set_rack_space_echo(6, 420.0, 0.62, t, 0.3, 0.3).unwrap(),
                "intensity" => d.set_rack_space_echo(6, 420.0, t, 0.6, 0.3, 0.3).unwrap(),
                "echo reverb" => d.set_rack_space_echo(6, 420.0, 0.62, 0.6, t, 0.3).unwrap(),
                "wow flutter" => d
                    .set_rack_space_echo(6, 420.0, 0.62, 0.6, 0.3, 2.0 * t)
                    .unwrap(),
                "spring wet" => d.set_rack_spring(0.6, 0.5, t).unwrap(),
                "spring decay" => d.set_rack_spring(t, 0.5, 0.35).unwrap(),
                "spring tone" => d.set_rack_spring(0.6, t, 0.35).unwrap(),
                "phaser mix" => d.set_rack_phaser(0.4, 0.7, 0.45, t).unwrap(),
                "phaser depth" => d.set_rack_phaser(0.4, t, 0.45, 0.5).unwrap(),
                "phaser rate" => d.set_rack_phaser(0.05 + 9.95 * t, 0.7, 0.45, 0.5).unwrap(),
                "phaser feedback" => d.set_rack_phaser(0.4, 0.7, 0.95 * t, 0.5).unwrap(),
                _ => d.set_rack_big_knob(2, 0.5 + 7.5 * t).unwrap(),
            }
        };
        let mut steady = 0.0_f32;
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let mut ch = FxChannel::new();
            set(&mut ch, t);
            ch.switch(slot, true);
            let held = ch.run(4.0);
            steady = steady.max(sharpest(&held[secs(2.0)..]));
        }
        let mut ch = FxChannel::new();
        ch.switch(slot, true);
        ch.run(3.0);
        let blocks = secs(2.0).div_ceil(BLOCK);
        let swept = ch.run_blocks(blocks, |ch, b| {
            if b % per != 0 {
                return;
            }
            #[allow(clippy::cast_precision_loss)]
            let t = 1.0 - (2.0 * b as f32 / blocks as f32 - 1.0).abs();
            set(ch, t);
        });
        let corner = sharpest(&swept);
        let line = format!("{name}: held {steady:.5}, dragged {corner:.5}");
        report.push(if corner > click_bar(steady) {
            format!("{line}  <- zippers")
        } else {
            line
        });
    }
    let text = report.join("\n");
    println!("{text}");
    assert!(!text.contains("zippers"), "{text}");
}
