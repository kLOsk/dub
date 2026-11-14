# Manual rig checklist

The hardware half of the release gate (PRD §2.2.10). CI proves the engine in
the simulator; this proves it with a needle on a record. **Every alpha cut is
signed off against this list** — any FAIL in sections 1–9 blocks the cut.

Work top to bottom; each item is *do this → expect that*. Write PASS / FAIL /
SKIP and, for a FAIL, one line of what happened. A stall or a glitch you
cannot reproduce is still a FAIL until it is explained.

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
- **Bindings:** nothing played ships with a key — bind the ones you want in
  map mode (MAP on the status strip) before section 5.

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

- [ ] **One-turntable swap.** DUB FX on (Preferences ▸ Alpha: *Alpha
      features*, then *Dub FX channel*). Play a record in THRU on deck A,
      ECHO OUT on the last phrase, **HOLD**, lift the needle, change the
      record, drop it, **HOLD** off → the echo carries the whole swap
      (10–20 s) at nearly its level, never grows, fades when released.
- [ ] **Space Echo HOLD.** Patch first: the mixer's aux send into deck B's
      input pair on the SL 3 (line, in place of turntable 2) — the channel
      has no software routing, it processes whatever is on its pair. Deck B
      on DUB FX, deck A's channel sent on the mixer. Throw a
      phrase into the Space Echo, **HOLD**, lift and drop the needle → the
      loop sustains, and the lift, crackle and drop thump are *not* in it.
      Listen for whether the held loop sounds like tape or turns boomy.
- [ ] **SYNC on a timecode deck.** Space Echo SYNC at ¾ (the keys on the
      Space Echo's top row in the DUB FX deck), deck A on timecode
      with an analysed track. The line under REPEAT reads `♩ A · <bpm>`;
      the repeats sit on the beat. Ride the pitch fader −8 → +8 % → the
      echoes stay in time, bend smoothly, never click.
- [ ] **TAP on a real record.** Deck A in THRU: the deck's BPM shows the
      record's tempo once it is heard, and SYNC reads `♩ A · <bpm>`. Tap the
      deck's BPM four times → `BPM · TAP`, echoes on your beat. (The
      Space Echo's own TAP is the fallback while nothing is heard yet.)
- [ ] **Following beat grid (Alpha).** Preferences ▸ Alpha: *Alpha
      features* and *Following beat grid* on. Load a live-band track (roots
      reggae, soul) → the beat lines sit on the band's beats through the
      track, unevenly spaced. Load a drum-machine track → an even grid, as
      before. Switch it off, load the live track again → the even grid is
      back. Note any track where the followed lines wobble or hop.
- [ ] **Deck HOLD only with DUB FX.** Preferences ▸ Alpha: either switch
      off → the echo is the single ECHO OUT, no HOLD, and the source switch
      has no DUB FX. Both on → OUT · HOLD.
- [ ] **FX rack responsiveness.** Switch a deck to DUB FX and back five
      times; fold and unfold the rack bar. The rack appears in well under
      a second, knobs answer at once, no stall dumps appear.
- [ ] **SL 3 connect.** Quit Dub, unplug the SL 3, launch, plug it in →
      no long freeze; the decks come up.
- [ ] **Echo Out on the next beat.** Press ECHO OUT on a beat → that beat
      plays, then mutes and repeats, the first repeat already quieter than
      the beat. Press again inside the beat → nothing is muted.
- [ ] **Vertical waveform drag.** Drag the strip up → the track moves
      forward.
- [ ] **A rip end to end** (section 7) with the new file names.
- [ ] **REC → PREP parks a take** (section 7).

## 1. Launch and library

- [ ] Cold launch with no library configured → the first-run experience, no crash.
- [ ] Import a large Serato library (≥ 50 k tracks if you have one) → done
      within 60 s, metadata intact, no crash.
- [ ] Import Traktor, rekordbox and iTunes one after another → no
      conflicts, no duplicates beyond what the dedupe reports.
- [ ] Launch with a deliberately corrupted Serato file in place → a clear
      error, the app carries on.
- [ ] Search, filter boxes, sort: every interaction answers immediately
      with the library at full size.

## 2. Interface and timecode

- [ ] SL 3, control vinyl on inputs 1/2 and 3/4, both decks on TC → both
      lock; the calibration bar shows while calibrating — also with **no
      track loaded**.
- [ ] Same on the Audio 6 with Traktor MK2 vinyl (and MK1 if you have it).
- [ ] Lift the needle mid-play, drop it elsewhere → re-locks, no click or
      runaway.
- [ ] Stop the platter by hand, start it → the waveform keeps its zoom
      through stop and start.
- [ ] Pitch −8 → +8 % slowly → the waveform stretches with the pitch the
      whole way (no jump, no new zoom at the end); BPM follows the fader.
- [ ] Scratch hard → the waveform keeps its zoom; the deck follows the hand.

## 3. Playback, waveform, phase meter

- [ ] Play a track on TC for 30 minutes with key lock on and occasional
      scratching → no xruns, no audible glitch, no stall dumps.
- [ ] Two decks beatmatched → the strips scroll at the same speed and the
      beat spacing matches; the phase meter reads late above the line.
- [ ] Waveforms never jump left/right while a deck plays steadily.
- [ ] Key lock at a pitched platter → the key holds, no warble.

## 4. Loops and hot cues

- [ ] Loop 1 · 2 · 4 · 8 under a real needle → seamless, on the beat.
- [ ] Lift the needle mid-loop, drop it → loop and lock behave.
- [ ] Set, fire and clear hot cues on both decks; reload the track → cues
      survive.

## 5. Echo Out, siren, sampler

- [ ] Echo Out 50 times in a row → no degradation; each tail decays
      cleanly; the deck comes back at its slipped position.
- [ ] Echo Out on a THRU deck → the record **mutes**, the echo plays out;
      off → the record is back. Lift the needle while it rings → the echo
      finishes before it turns itself off.
- [ ] Siren: all five shots, the DUB knob end to end → level-matched,
      no clicks, the VU throws on each press. The Laser is a zap with no low
      growl at its end; the Rifle Gun's crack at the end of each round does
      not click (both 2026-11-14).
- [ ] Sampler pads and Quick Scratch on both decks → fire, swap, release.
- [ ] Every mapped key fires its control; map mode rebinds and unbinds.

## 6. DUB FX channel

- [ ] Mixer aux send into the FX deck's pair, SEND selected → the VU
      moves with the send; push the trim until HOT lights.
- [ ] MIC: a mic on one side of the pair → heard in both channels.
- [ ] Each unit in and out (bat toggle and its key) → audible, no click.
- [ ] Big Knob through all 11 detents with Q up → no zipper, no blow-up.
- [ ] Space Echo: every mode; Repeat knob swept → the repeats bend like
      tape, no clicks; intensity past 1.0 → RUNAWAY, bounded.
- [ ] Spring: KICK → thunder, every time.
- [ ] Switch the FX deck back to INT / TC / THRU → the rack is bypassed,
      the deck plays normally.

## 7. Vinyl rip

- [ ] PREP · REC · PERF: REC arms; drop the needle → recording starts on
      the music (needle-drop auto-start); run-out → it stops.
- [ ] While recording, dragging a library track onto a deck is refused.
- [ ] Review: the tracklist splits sensibly; edit titles, artist, genre
      (suggestions from the library, a new genre accepted) → fields
      neither blink nor lose input.
- [ ] Finish → files named `Artist - Title.flac` (or `Track N.flac`),
      tagged with BPM and key, imported into the library.
- [ ] REC → PREP with a reviewed take → it is parked; the recovery banner
      offers it back.
- [ ] Recognition (needs network): identifies a known pressing, or fails
      quietly — the rip never depends on it.

## 8. Thru — real records

- [ ] THRU on a deck, needle up → the title reads *Waiting for the needle*,
      the clock `--:--`, INPUT `○ NO SIGNAL`; no cues, loop or scratch pads.
- [ ] Drop a record → it passes through the engine with no audible latency;
      the title reads *Live record*, the clock counts from `00:00`, both
      INPUT meters move (one dead meter = a dead channel), and the strip's
      waveform keeps building past the first ten seconds (the 2026-11-14
      bug: black after ~11 s).
- [ ] *The side so far* grows left to right against minute marks; lift the
      needle → it stays drawn; the next record → it starts over.
- [ ] Once the BPM is solid → dashed beat lines below the playhead, faint
      ones above sitting on the kicks; the TEMPO lamp flashes on the beat.
      Note if the lines sit early or late against the kicks. With the BPM
      solid the lines **hold still** — no hopping half a beat and back (the
      2026-11-14 reggae 12″), no catching up in jumps on a live band.
- [ ] Auto-BPM: a reggae, a hip-hop and a dnb record in THRU → the deck's BPM
      shows a dim number, then a solid one within 15 s, on the right tempo —
      and **stays solid** through the record (it flickered solid/dim on
      2026-11-14).
      **Known:** dnb reads two-thirds of its tempo (~116 for ~175) — note it.
- [ ] Tap override: tap the deck's BPM four times on the beat → it reads
      `BPM · TAP` with your tempo; the Space Echo's SYNC follows it. Lift the
      needle, put the next record down → the tap is gone and the deck listens
      again.
- [ ] Right-click the BPM → ×2, ÷2, ×1.5 and ×⅔: the label reads
      `BPM · ×2` etc., the number follows the tracker scaled, the beat lines
      re-space. On the dnb record ×1.5 → ~175. *Back to the heard tempo*
      clears it; the next record clears it too.

## 9. Robustness

- [ ] Unplug the interface mid-playback → the engine stops cleanly and the
      app says so; plug it back → playback can resume. Note exactly what
      the app showed.
- [ ] Sleep and wake the Mac with Dub running → audio comes back without
      artefacts.
- [ ] 60 minutes of mixed use, then quit → no crash, no orphan processes
      (`pgrep -l Dub` is empty), no stall dumps.

## 10. Prep mode (built-in output, no interface)

- [ ] Launch without the interface → Prep: one deck, horizontal waveform.
- [ ] Load, play, seek on the overview, set the grid (tap the BPM to set
      the 1), set cues — every control mouse-clickable.

---

## Sign-off

| Section | Result | Notes |
|---|---|---|
| 0 Owed now | | |
| 1 Launch and library | | |
| 2 Interface and timecode | | |
| 3 Playback | | |
| 4 Loops and cues | | |
| 5 Echo, siren, sampler | | |
| 6 DUB FX | | |
| 7 Rip | | |
| 8 Thru | | |
| 9 Robustness | | |
| 10 Prep | | |

Signed off by: ______ on ______ — alpha cut: yes / no
