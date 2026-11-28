# Rig checklist

The hardware half of the release gate (PRD §2.2.10). CI proves the engine in
the simulator; this proves it with a needle on a record. **Every alpha cut is
signed off against this list** — any FAIL in sections 1–9 blocks the cut.

Each item is *do this → expect that*. Write PASS / FAIL / SKIP and, for a
FAIL, one line of what happened. A stall or a glitch you cannot reproduce is
still a FAIL until it is explained.

## Who runs what

Every item carries one tag. The tag says who *can* run it; nothing here has
been run by the agent yet (2026-11-27).

| Tag | Who | Needs |
|---|---|---|
| `robot` | The agent: launches Dub, clicks and types in it, screenshots its window, reads the stall logs. | Nobody at the Mac while it runs: its clicks land on whatever app is in front. |
| `loop` | The agent, playing a recorded take into the real app through the interface and recording what comes out. | The cable loop below, and nobody at the decks. |
| `hands` | The DJ. | A needle, a platter, a plug or an ear. |

Where one item had a part for each, it is two checkboxes now.

**The cable loop (SL 3).** Two RCA pairs, with every input in the loop set
to LINE:

1. **AUX OUT → LEFT DECK IN.** The agent plays a recorded take out of the
   aux pair, and deck A hears a turntable.
2. **LEFT DECK OUT → AUX IN.** The agent records what deck A plays.

The takes are in `testdata/timecode/full/` (both control records, every
fader step from −8 to +8 %, scratching, a needle drop) and
`testdata/rip-baselines/` (a reggae 12″, a soul sampler, a dnb 12″ for THRU
and rip items). They were recorded after the SL 3's own input stage, which is
why the loop wants LINE and not PHONO.

**A loop PASS covers the app and the interface, not a cartridge on vinyl.**
It stands for the item on a cut that didn't touch the decoder or
calibration (`dub-timecode`, the deck's calibration). On a cut that did, run
sections 2–4 with needles as well.

**The scratch library.** The agent's runs never use the DJ's library:
loading a track writes set history, and a finished rip imports into it.
Launch Dub on a folder of its own instead:

    open --env DUB_DATA_DIR="$HOME/Library/Application Support/Dub Scratch" \
        apple/build/Build/Products/Release/Dub.app

The library, its waveforms, rip sessions and the grid-calibration log then
live in that folder, and the library footer reads **SCRATCH LIBRARY**. The
first launch imports the enabled sources (Serato, Traktor, Apple Music) by
reading them, which is what the import items time. `dub import` and
`dub diagnose` follow the same variable. Launched any other way, Dub opens
the DJ's library as always. Delete the folder to start the next run from
nothing. (`DUB_DATA_DIR`, 2026-11-27.)

**Checking the loop.** Run this after cabling, with Dub closed:

    dub loopback testdata/timecode/serato-cv02-steady-nominal.flac \
        --device "SL 3" --input-channels 3,4 --record /tmp/loop-check.wav

Expect *the take came back as it went out*, at a level within a few dB of
0. Tens of dB up, or a peak pinned at 0 dBFS, means LEFT DECK IN is on
PHONO. On 2026-11-27 it came back at −0.1 dB (correlation 0.999) and
decoded like the original: locked on every window, the same position code.
Beside a running Dub the same take locked deck A (the SIGNAL panel read
LOCKED · Timecode · forward, calibrated and slid away), and a siren shot
routed to deck A arrived on AUX IN, so Dub and the player share the SL 3
both ways. Not checked yet: a track playing on a deck while the player runs.

**The agent's tools.** `make rig-drive` builds the click, drag and window
helpers (`tools/rig-drive/`); `scripts/rig/lib.sh` plays a take, logs each
press on the recording's own clock, and refuses to click unless Dub is in
front and the screen is unlocked. The measurements are `dub` commands:

- `dub loop-check` — a loop's length in beats, how exactly it replays, each
  seam against the same drum hit played straight, and the exit.
- `dub echo-check` — ECHO OUT against an untouched pass of the same take
  from the same start: when the dry muted, the repeats' levels, where the
  tune came back.
- `dub thru-check` — a THRU deck's output against what it received (one
  4-channel recording, `--input-channels 1,2,3,4`): the delay through the
  deck and any dropout.
- `dub dropout-check` — holes in a recording of a deck playing.
- `dub thru-tempo` — a recording replayed through the THRU deck's own
  tempo listener (the engine's settings, fed in step with the clock):
  what the deck's BPM would have shown each second, with `--raw` for the
  estimator's every reading underneath. A tempo seen on the rig can be
  reproduced, and fixed, at the desk.

`scripts/rig/` runs whole items: `loops.sh` (4.1), `echo-out.sh` (0.10,
5.1), `soak.sh` (3.1, 9.3).

**Order of a run.**

1. `robot`, interface unplugged, on the scratch library: section 10.
2. Plug the interface in and cable the loop.
3. `robot` and `loop`, on the scratch library: the agent works through them
   and fills its columns of the sign-off table. It puts back every
   preference and key it changed.
4. Uncable. `hands`: the DJ's session, with the coach, on the DJ's own
   library.

**The coach.** `make rig` reads the `hands` items aloud, one at a time, so
the list doesn't have to be followed on a screen from the decks. Do the
item, then answer with one key:

| Key | |
|---|---|
| `p` | Pass |
| `f` | Fail. It asks for one line of what happened. |
| `s` | Skip |
| `r` | Say it again |
| `b` | Back one item |
| `q` | Stop. Run it again and it picks up where you stopped. |

It writes the run to `docs/rig-runs/` after every answer, with the commit,
the machine and the interface filled in, and it names any stall dump that
lands while an item is up. `make rig RIG='--section 2'` runs one section,
for after a fix; `RIG=--list` prints what it would read; `RIG=--mute` shows
the items without the voice.

Each item is also recorded: every input of the interface, from the moment
the item comes up to its answer, into `testdata/rig-takes/<run>/<item>.wav`
(gitignored), and the run file names the take beside the verdict. A lift, a
stop or a scratch done once here can then be played back through the loop
cable as often as a check needs it, without hands (`dub loopback <take>
--take-channels 3,4` plays deck A's pair on the SL 3). With no
interface plugged in the coach says so and carries on; `RIG='--device NAME'`
picks one when two are plugged in.

## Before you start

- **Build:** a **Release** build — `make app APP_CONFIG=Release`. Debug hides
  real timing and once hid a Release build that did not compile at all.
  Note the commit (`git log -1 --oneline`) and the FFI number (README).
- **Rig:** the interface (SL 3 is the reference; Audio 6 for the Traktor
  items), two turntables with control vinyl, the external mixer with an aux
  send, a mic, a real record or three (a reggae, a hip-hop, a dnb 12″).
- **Watching:** stalls land in `~/Library/Logs/Dub/stall-*.txt` and on
  `log stream --predicate 'subsystem == "com.dub.app"'`. Empty the folder
  first so anything new is from this run.
- **Bindings:** Dub ships a keymap (left hand deck A, right hand deck B).
  Rebind in map mode (MAP on the status strip) before section 5 if you play
  another.

| Run | |
|---|---|
| Date | |
| Commit · FFI | |
| Machine · macOS | |
| Interface · vinyl | |
| Tester | |

---

## 0. Owed now — shipped, never confirmed on the rig

Clear these first; they are the open items in `history/CURRENT.md`.

- [ ] `hands` **One-turntable swap.** DUB FX on (Preferences ▸ Alpha: *Alpha
      features*, then *Dub FX channel*). Play a record in THRU on deck A,
      ECHO OUT on the last phrase, **HOLD**, lift the needle, change the
      record, drop it, **HOLD** off → the echo carries the whole swap
      (10–20 s) at nearly its level, never grows, fades when released.
- [ ] `hands` **Space Echo HOLD.** Patch first: the mixer's aux send into deck B's
      input pair on the SL 3 (line, in place of turntable 2) — the channel
      has no software routing, it processes whatever is on its pair. Deck B
      on DUB FX, deck A's channel sent on the mixer. Throw a
      phrase into the Space Echo, **HOLD**, lift and drop the needle → the
      loop sustains, and the lift, crackle and drop thump are *not* in it.
      Listen for whether the held loop sounds like tape or turns boomy.
- [ ] `hands` **SYNC on a timecode deck.** Space Echo SYNC at ¾ (the keys on the
      Space Echo's top row in the DUB FX deck), deck A on timecode
      with an analysed track. The line under REPEAT reads `♩ A · <bpm>`;
      the repeats sit on the beat. Ride the pitch fader −8 → +8 % → the
      echoes stay in time, bend smoothly, never click.
- [ ] `loop` **TAP on a real record.** Deck A in THRU: the deck's BPM shows the
      record's tempo once it is heard, and SYNC reads `♩ A · <bpm>`. Tap the
      deck's BPM four times → `BPM · TAP`, echoes on your beat. (The
      Space Echo's own TAP is the fallback while nothing is heard yet.)
- [ ] `robot` **Following beat grid (Alpha).** Preferences ▸ Alpha: *Alpha
      features* and *Following beat grid* on. Load a live-band track (roots
      reggae, soul) → the beat lines are unevenly spaced. Load a
      drum-machine track → an even grid, as before. Switch it off, load the
      live track again → the even grid is back.
- [ ] `hands` **Following beat grid, by eye.** On your own live-band tracks
      the lines sit on the band's beats through the track. Note any track
      where they wobble or hop.
- [ ] `robot` **Deck HOLD only with DUB FX.** Preferences ▸ Alpha: either switch
      off → the echo is the single ECHO OUT, no HOLD, and the source switch
      has no DUB FX. Both on → OUT · HOLD.
- [ ] `robot` **FX rack responsiveness.** Switch a deck to DUB FX and back five
      times; fold and unfold the rack bar. The rack appears in well under
      a second, knobs answer at once, no stall dumps appear.
- [ ] `hands` **SL 3 connect.** Quit Dub, unplug the SL 3, launch, plug it in →
      no long freeze; the decks come up.
- [ ] `loop` **Echo Out on the next beat.** Press ECHO OUT on a beat → that beat
      plays, then mutes and repeats, the first repeat already quieter than
      the beat. Press again inside the beat → nothing is muted.
- [ ] `robot` **Vertical waveform drag.** Drag the strip up → the track moves
      forward.
- [ ] `loop` **A rip end to end** (section 7) with the new file names.
- [ ] `loop` **REC → PREP parks a take** (section 7).

## 1. Launch and library

- [ ] `robot` Cold launch with no library configured → the first-run experience, no crash.
- [ ] `robot` Import a large Serato library (≥ 50 k tracks if you have one) → done
      within 60 s, metadata intact, no crash.
- [ ] `robot` Import Traktor, rekordbox and iTunes one after another → no
      conflicts, no duplicates beyond what the dedupe reports.
- [ ] `robot` Launch with a deliberately corrupted Serato file in place → a clear
      error, the app carries on.
- [ ] `robot` Search, filter boxes, sort: every interaction answers
      immediately with the library at full size. (`-dubLibraryBench 1` times
      the lists; it only reads the library.)

## 2. Interface and timecode

- [ ] `loop` A recorded take into deck A on TC → it locks; the
      calibration bar shows while calibrating — also with **no track
      loaded**.
- [ ] `hands` SL 3, control vinyl on inputs 1/2 and 3/4, both decks on TC →
      both lock from the records themselves.
- [ ] `hands` Audio 6 with Traktor MK2 vinyl (and MK1 if you have it), both
      decks on TC → both lock.
- [ ] `hands` Lift the needle mid-play, drop it elsewhere → re-locks, no click or
      runaway.
- [ ] `hands` Stop the platter by hand, start it → the waveform keeps its zoom
      through stop and start.
- [ ] `loop` Pitch −8 → +8 % (the fader-step take) on a programmed track
      (drum machine, locked grid; a live band drifts and hides the answer)
      → the waveform stretches with the pitch the whole way (no jump, no new
      zoom at the end); BPM reads the track's BPM × the pitch.
- [ ] `hands` Scratch hard → the waveform keeps its zoom; the deck follows the hand.
- [ ] `loop` The motions without hands: `dub timecode-gen motions.wav` (a
      lift and drop 60 s on, a hand stop, hard scratching, −8 / +8 %) into
      deck A on TC with a programmed track, Dub started with
      `DUB_DRAW_LOG=<file>` (Debug), then `dub draw-check <file>` → the deck
      holds through each lift and stop and carries on at unity; no step in
      steady play; the zoom holds through stop and scratch and follows the
      fader; the playhead traces the scratch strokes. Real vinyl's noise is
      not in it: the `hands` items above stay.

## 3. Playback, waveform, phase meter

- [ ] `loop` Play a track on TC for 30 minutes with key lock on and
      occasional scratching (the takes, back to back) → no xruns, no click
      or dropout in the recorded output, no stall dumps.
- [ ] `hands` Two decks beatmatched → the strips scroll at the same speed and the
      beat spacing matches; the phase meter reads late above the line.
- [ ] `hands` Waveforms never jump left/right while a deck plays steadily.
- [ ] `loop` Key lock at a pitched platter (the −8 % and +8 % steps, a
      test tone on the deck) → the tone comes out at its own pitch.
- [ ] `hands` Key lock at a pitched platter, on music → no warble.

## 4. Loops and hot cues

- [ ] `loop` Loop 1 · 2 · 4 · 8 on a timecode deck → no click at the
      seam, each loop its length in beats.
- [ ] `hands` Lift the needle mid-loop, drop it → loop and lock behave.
- [ ] `robot` Set, fire and clear hot cues on both decks; reload the track → cues
      survive.

## 5. Echo Out, siren, sampler

- [ ] `loop` Echo Out 50 times in a row → no degradation; each tail decays
      cleanly; the deck comes back at its slipped position.
- [ ] `loop` Echo Out on a THRU deck (a recorded side) → the record
      **mutes**, the echo plays out; off → the record is back. The side goes
      silent while it rings → the echo finishes before it turns itself off.
- [ ] `loop` Siren: all five shots, the DUB knob end to end →
      level-matched, no clicks, the VU throws on each press.
- [ ] `hands` Siren, by ear: the Laser is a zap with no low growl at its
      end; the Rifle Gun's crack at the end of each round does not click
      (both 2026-11-14).
- [ ] `robot` Sampler pads on both decks → fire, swap, release.
- [ ] `hands` Quick Scratch on both decks → the sample is under the hand;
      release gives the track back.
- [ ] `robot` Every mapped key fires its control; map mode rebinds and
      unbinds.

## 6. DUB FX channel

Parked behind Preferences ▸ Alpha. The loop above doesn't reach it: the FX
deck processes the mixer's send. A second patch (deck A's output into the FX
deck's input) would let the agent take the click and level checks; it isn't
set up. Offline, `dub-engine/tests/fx_by_ear.rs` measures the clicks,
zippers and bounds below on every build; the ear still judges the sound.

- [ ] `hands` Mixer aux send into the FX deck's pair, SEND selected → the VU
      moves with the send; push the trim until HOT lights.
- [ ] `hands` MIC: a mic on one side of the pair → heard in both channels.
- [ ] `hands` Each unit in and out (bat toggle and its key) → audible, no click.
- [ ] `hands` Big Knob through all 11 detents with Q up → no zipper, no blow-up.
- [ ] `hands` Space Echo: every mode; Repeat knob swept → the repeats bend like
      tape, no clicks; intensity past 1.0 → RUNAWAY, bounded.
- [ ] `hands` Spring: KICK → thunder, every time.
- [ ] `robot` Switch the FX deck back to INT / TC / THRU → the rack is bypassed,
      the deck plays normally.

## 7. Vinyl rip

The loop plays a recorded side (`testdata/rip-baselines/`), whose track
count and side times are known. On the scratch library the rip, its FLACs
and its import all stay inside the scratch folder.

- [ ] `loop` PREP · REC · PERF: REC arms; the side starts → recording
      starts on the music (needle-drop auto-start); run-out → it stops.
- [ ] `hands` REC armed, drop a real needle → recording starts on the
      music; run-out → it stops. Once per cut.
- [ ] `loop` While recording, dragging a library track onto a deck is refused.
- [ ] `loop` Review: the tracklist splits sensibly; edit titles, artist, genre
      (suggestions from the library, a new genre accepted) → fields
      neither blink nor lose input.
- [ ] `loop` Finish → files named `Artist - Title.flac` (or `Track N.flac`),
      tagged with BPM and key, imported into the library.
- [ ] `loop` REC → PREP with a reviewed take → it is parked; the recovery banner
      offers it back.
- [ ] `loop` Recognition (needs network, and Dub's AcoustID application key
      in the repo's `.env` for a Debug build): identifies a known pressing,
      or fails quietly — the rip never depends on it.

## 8. Thru — real records

- [ ] `robot` THRU on a deck, needle up → the title reads *Waiting for the needle*,
      the clock `--:--`, INPUT `○ NO SIGNAL`; no cues, loop or scratch pads.
- [ ] `loop` A recorded side starts → it passes through the engine, the
      delay through the deck measured in milliseconds; the title reads *Live record*, the clock counts from `00:00`, both
      INPUT meters move (one dead meter = a dead channel), and the strip's
      waveform keeps building past the first ten seconds (the 2026-11-14
      bug: black after ~11 s).
- [ ] `hands` Drop a real record → no latency you can hear against the
      needle.
- [ ] `loop` *The side so far* grows left to right against minute marks;
      the side ends → it stays drawn; the next side → it starts over.
- [ ] `hands` Once the BPM is solid → dashed beat lines below the playhead, faint
      ones above sitting on the kicks; the TEMPO lamp flashes on the beat.
      Note if the lines sit early or late against the kicks. With the BPM
      solid the lines **hold still** — no hopping half a beat and back (the
      2026-11-14 reggae 12″), no catching up in jumps on a live band.
- [ ] `loop` Auto-BPM: the reggae and dnb sides in THRU (no hip-hop side
      is recorded yet: that one is `hands`) → the deck's BPM
      shows a dim number, then a solid one within 15 s, on the right tempo —
      and **stays solid** through the record (it flickered solid/dim on
      2026-11-14).
      **Known:** dnb reads two-thirds of its tempo (~116 for ~175) — note it.
- [ ] `loop` Tap override: tap the deck's BPM four times on the beat → it reads
      `BPM · TAP` with your tempo; the Space Echo's SYNC follows it. Lift the
      needle, put the next record down → the tap is gone and the deck listens
      again.
- [ ] `loop` Right-click the BPM → ×2, ÷2, ×1.5 and ×⅔: the label reads
      `BPM · ×2` etc., the number follows the tracker scaled, the beat lines
      re-space. On the dnb record ×1.5 → ~175. *Back to the heard tempo*
      clears it; the next record clears it too.

## 9. Robustness

- [ ] `hands` Unplug the interface mid-playback → the engine stops cleanly and the
      app says so; plug it back → playback can resume. Note exactly what
      the app showed.
- [ ] `hands` Sleep and wake the Mac with Dub running → audio comes back without
      artefacts.
- [ ] `loop` 60 minutes of the takes and scripted presses, then quit →
      no crash, no orphan processes (`pgrep -l Dub` is empty), no stall
      dumps.
- [ ] `hands` After your own session, quit → no crash, no orphan
      processes (`pgrep -l Dub` is empty), no stall dumps.

## 10. Prep mode (built-in output, no interface)

- [ ] `robot` Launch without the interface → Prep: one deck, horizontal waveform.
- [ ] `robot` Load, play, seek on the overview, set the grid (tap the BPM to set
      the 1), set cues — every control mouse-clickable.

---

## Sign-off

A dash means the section has no item with that tag.

| Section | Robot | Loop | Hands | Notes |
|---|---|---|---|---|
| 0 Owed now | | | | |
| 1 Launch and library | | — | — | |
| 2 Interface and timecode | — | | | |
| 3 Playback | — | | | |
| 4 Loops and cues | | | | |
| 5 Echo, siren, sampler | | | | |
| 6 DUB FX | | — | | |
| 7 Rip | — | | | |
| 8 Thru | | | | |
| 9 Robustness | — | | | |
| 10 Prep | | — | — | |

Signed off by: ______ on ______ — alpha cut: yes / no
