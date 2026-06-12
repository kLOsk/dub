# Stillpoint — beatmatch aid, round 3 (design proposal)

Status: **proposal for review** — supersedes the round-2 gutter candidates
(Pendulum / Tug Bars / Beat Ladder, `BeatmatchViz.swift`) and, if adopted,
replaces PRD §9.4's Phase-Drift Trail as the centre-gutter design. The PRD
edit is deliberately left to the design owner; this doc carries the full spec.

Round-2 verdict from the rig: "none of them feels how DJs think — more like a
programmer." This round started from a diagnosis of *why* (below), ran a
multi-agent design study (3 research tracks, 7 design lenses, 21 concepts,
3 adversarial judges per concept, synthesis + completeness audit), and lands
on one primary design with two fallbacks.

---

## 1. Why round 2 failed (diagnosis, treat as established)

1. **Symmetric where the DJ is asymmetric.** All candidates show "A vs B"
   diffs. A blend has a *master* (owns the room — untouchable) and an
   *incoming* deck (the only deck the DJ corrects). The only question is
   "what do I do to MY record." Forcing the DJ to resolve "which deck leads →
   which deck is mine → so speed up or slow down" burns the exact cognitive
   frame the product exists to eliminate.
2. **State, not action.** Δ quantities on abstract axes the DJ must mentally
   differentiate and translate into hand motion.
3. **Horizontal axes fight the product's own law.** The whole UI maps platter
   motion to vertical (forward = up, PRD §9.1). The candidates speak
   left/right = deck A/B — an axis that means nothing to the hand.
4. **Position-coded tempo.** Tempo error is a *rate*. Eyes are poor at small
   static offsets, superb at motion — that's how turntablists read Technics
   strobe dots (stationary = on speed). Render tempo as drift; matched = the
   world freezes. Phase is a *position*: render as displacement.
5. **No workflow staging.** The DJ matches tempo *before* the drop and phase
   *after* it. Equal-weight twin meters serve neither moment.
6. **Numbers as crutch.** BPM digits already live in the deck headers. The
   gutter must do what digits can't: direction-to-action, drift-over-time,
   honesty.
7. **The PRD trail's one great idea — memory — was dropped.** Ears detect
   drift over bars; the display needs short memory to answer "did my nudge
   hold?"
8. **Missing ride-stage killer feature:** if the DJ keeps nudging the same
   way, their pitch is off by a computable amount. Nothing on the market
   coaches this.

Field research that anchors the design: DJs can hear the flam easily but
can't *attribute* it ("I get confused which track is going faster" is the
canonical beginner wall) — attribution is precisely what a display can supply.
Nudge→drift→pitch-trim→repeat is "the classic way of beatmatching" (Digital
DJ Tips), and the expert heuristic "if you need to drag continuously, slow
the incoming track down" appears verbatim in tutorials. The Technics strobe
is the one visual instrument vinyl DJs already trust, and its perceptual
channel is motion-nulling. No shipping product (Serato, Traktor, rekordbox,
Mixxx, VirtualDJ, djay, Phase) frames its display as an *action on the
incoming deck*; all are symmetric state meters, and the 1990s Numark
Beatkeeper got closer than any of them.

---

## 2. PRIMARY — Stillpoint

### Soul

One incoming-tinted band on the playhead line. It **drifts** when your tempo
is off; it **freezes** when matched; it **sits on the line** when you're in;
the line grows green beneath it for every beat you hold. Governing law:
**nothing in the gutter ever moves at beat rate — only at error rate.**
Because phase integrates tempo, one object carries the whole workflow as two
perceptual reads a vinyl DJ already owns: first *make it stop moving* (pitch
fader — the strobe null), then *make it sit on the line* (hand — seat a
thing on a mark). The master is never drawn as a contestant — the master
**is the line**, untouchable, exactly the booth's social reality. Matched is
stillness. Locked is a sleeping band on a slowly growing green line. A false
green has no render path.

### The four moments

- **Tempo (pre-drop).** Master owns the room; you ride the incoming pitch
  fader, record running in your phones. The gutter shows a belt of dots in
  your deck's tint scrolling against fixed grey hairlines — climbing = your
  record is slow (pitch `+`), sinking = fast. You don't read anything; you ride the
  fader until the dots petrify. Grab the record to re-cue: the dots freeze
  to grey ghosts (remembering, not measuring). Release: they re-ink.
- **Drop.** You release on the phrase. Within ~150 ms the Beat Band
  materializes at your true landing — above the line: late; below: early —
  and the belt dissolves. Wherever the band sits, the fix is already chosen.
- **Phase.** Band above the line → the room got ahead of you. You push the
  label; the band settles down into the grey pocket and turns greener the
  closer it rides to the line. Band seated, all gates holding: the line
  ignites green and grows outward.
- **Ride.** Minutes in: dark gutter, green line nearly full width — beats
  held, readable in the corner of the eye. The band slips → the line
  **shatters from the edges** (a one-shot brightening — lock-loss is an
  event, never a slow fade). You nudge it home; the pocket re-greens; the
  line regrows. Third same-direction nudge with the inter-nudge drift agreeing:
  a small `+` glyph breathes at the gutter foot with `0.2` in fine print —
  the app quoting your own hands. One hair of fader; it dissolves.

### Visual spec (120 px × full waveform height; lock line at y = 25 %)

| # | Element | Spec |
|---|---|---|
| 1 | **Lock line** | 2 px full-width hairline at the *same y as the waveform playheads* (collinear across all three columns). Neutral grey at 60 %; brightens to 85 % whenever the band is off-line (the datum must never be dimmer than the indicator). In certified lock it becomes the **hold-line**: locked-green, growing outward from centre 2 px/beat per side → full width ≈ 30 beats; on revoke it shatters from the edges (200 ms) with a 150 ms brightening pulse; regrows from centre on re-grant. |
| 2 | **Beat Band** | 64 × 10 px rounded bar (48 px wide at 80 px gutter), incoming tint, 90 % opacity, horizontally centred, no halo (cut in round 5). `y = lineY + 0.75 px/ms × φms` (inverted axis, final: late = above; gain halved in round 4 — at 1.5 px/ms inaudible near-match wobble read as alarming motion), linear to ±40 ms (±30 px), arcsinh-compressed to ±50 px at ±half-beat; past clamp the band grows a chevron tip on its moving edge — never silently saturate. Display position median-filtered with a motion floor (sub-threshold jitter renders as stillness). The pill's colour is the **proximity gradient**: it blends from deck tint toward green as it approaches the line — fully green on a perfect hit, wearing off continuously to pure tint by ~1.6× the pocket. Honest by construction: it claims only "the measured offset is ≈ 0 right now"; certification stays the line's statement. |
| 2b | **Pocket** | ±10 ms zone around the lock line (≈ the audibility threshold of a kick flam), drawn as a quiet full-width grey strip — tolerance made visible, nothing more. The "how close" colour lives on the pill, not the zone. The band keeps its true position — the zone communicates tolerance, it never snaps. |
| 3 | **Deposits (memory + honesty)** | **Hidden since round 5** (operator: distracting, no felt purpose). The engine still stamps them (machinery + tests retained) pending a subtler memory form; the hold-line's growth is currently the only visible memory, so R10 is partially deferred. Original spec: one tick per master beat at the band's y, pile = held, ladder = leaking, gap = abstained. |
| 4 | **Tempo belt (pre-drop face)** | 8 dots Ø10 px at 56 px pitch, incoming tint 70 %, scrolling over fixed 1 px hairlines at the same pitch. Velocity `v = sign(Δ)·(3 + 55·|ΔBPM|) px/s`, capped 110 px/s with dim+blur beyond (coarse matching belongs to the header digits). **Deadband ±0.015 BPM renders as dead stop; sign flips require |Δ| > 0.03** (kills zero-crossing flicker). Hand on record: dots hollow to grey outlines, frozen at the last honest verdict. |
| 5 | **Coach glyph** | 14 px `+` / `−` disc at the gutter foot, incoming tint, suggested trim beneath in 9 pt mono at 35 % (`0.2` = +0.2 %). Pulses once on arming, 0.25 Hz breath while active; never loops. |
| 6 | **Fine print** | Bottom 24 px: Δms / ΔBPM, 9 pt mono, 40 % grey. Hidden entirely in RIDE unless drift resumes. |
| 7 | **Transitions** | Faces crossfade 250–400 ms; minimum dwell 1.2 s; nothing snaps; nothing anywhere animates at beat rate. |

### Channel → action

| You see | It means | You do |
|---|---|---|
| Band above the line | The room got ahead of you — late | Push the label; the band settles down onto the line |
| Band below the line | Early | Drag the platter edge; the band lifts home |
| Pill turning green | Within earshot of perfect (≤ ~10 ms) | Leave it; tend the drift, not the offset |
| Band/belt creeping up | You're running slow | Pitch fader **faster** (`+`) |
| Band/belt creeping down | Running fast | Pitch **slower** (`−`) |
| Ticks piled on the line | It's holding | Hands off |
| Ticks in a ladder | Tempo leak | Trim toward the creep's `+`/`−` |
| Green line growing | Certified lock; length = beats held | Work the room |
| Line shatters | Lock just broke | Glance, nudge |
| Frozen grey anything | Your hand owns the record / no data | Trust your ears |
| Chevron tip on band | Off-scale / near half-beat | Re-drop or big correction |
| `+0.2` at the foot | Three same-way nudges, quoted back | Kiss the fader |

**Sign convention (round 6, FINAL — operator-confirmed):** the
**inverted axis** — late = **above** the line, the gap the room has
opened ahead of you; push and the band settles down onto the line, drag
and it lifts back up. (History: round 3 inverted on rig feel, round 5
flipped back on a miscommunication, round 6 confirmed inverted. Closed.)
The belt matches: climbing = slow = pitch `+`. Honest trade-off, accepted
by the operator: during a correction the band moves *against* the platter
direction — the DJ's read outranks the doctrine.

**Pitch advice is always `+`/`−` symbols** — never a vertical fader picture
(the Technics fader physically runs down-toward-you for faster).

### Roles — who is incoming? (state machine)

**Bootstrap rules** (apply while no roles are assigned; they need no
timecode evidence, so internal/file decks frame correctly too):

- Exactly one deck playing, the other has a track loaded → the idle
  deck is the incoming one (sustained 1 s).
- Both playing with play-start separation ≥ 2 s → the **later starter**
  is the incoming one (sustained 1 s).

Beyond bootstrap, per deck, an **incoming score**, updated at poll rate:

- +1.0 per hand-on-record edge (lockState → 3), decaying τ = 10 s
- +0.8 per backward playhead jump > 1 beat (re-cue), decaying τ = 15 s
- +0.5 while play started < 30 s ago (decaying)
- −0.1/s of continuous clean play (tenure)

**Master-tenure prior:** once a deck has held master ≥ 64 beats, its hand
events count ×0.25; > 3 disengage events in 10 s on a tenured master is
performance (scratching), not cueing — scores nothing.

Roles assign when the score margin > 0.6 sustained 2 s. Sticky: a swap needs
the opposite margin for 5 s **with no hand currently down**, or the master
stopping ≥ 3 s. **Roles never decay while the stage is PHASE or RIDE.**
**NEUTRAL** (ambiguous) = both scores < 0.2 and both decks clean-playing
> 60 s: band desaturates to grey, no glyphs, no coach, no tinted deposits;
green requires the full two-sided certification. The first hand event
re-frames instantly (an ambiguous-state hand event *is* the resolving
evidence).

Known v0 limitation (documented, accepted): engine lockState 3 covers
lifted/scratching *and* signal dropout; a dropout on a young master can score
as cueing evidence. The tenure prior bounds the damage; the real fix is an
engine-side carrier-health qualifier on disengage edges (future, optional).

### Stages (transition table)

States: `HIDDEN`, `NEUTRAL`, `TEMPO`, `PHASE`, `RIDE`. (DROP is an edge
animation on TEMPO→PHASE, not a state.) All transitions respect 1.2 s
minimum dwell except →HIDDEN.

| From | To | Guard |
|---|---|---|
| any | HIDDEN | either deck missing grid/BPM (conf < 0.15) or no usable phase — band retracts into the line over 250 ms; belt-only if ΔBPM still computable |
| any (except PHASE/RIDE) | NEUTRAL | role margin invalid (defn above) |
| NEUTRAL | TEMPO | first hand event / role margin resolves |
| TEMPO | PHASE | **drop edge**: hand release (3→1, debounced 250 ms) into play while master plays — fire materialize animation, clear deposits, band live ≤ 150 ms. **Internal drop edge**: a file deck has no needle — its play press while the master plays IS the drop (same animation). Fallback: incoming clean play ≥ 4 beats → enter PHASE without the animation |
| PHASE | RIDE | \|φ\| < 12 ms sustained ≥ 8 master beats |
| RIDE | PHASE | \|φ\| > 25 ms sustained 2 beats (12-in / 25-out) |
| PHASE/RIDE | TEMPO | hold > 600 ms **and** backward jump > 1 beat (a re-cue). A nudge touch (< 600 ms, no jump) can never flip the face. A 3→1 edge in PHASE/RIDE is a nudge release — never re-fires the drop. Internal decks: any seek back > 1 beat is the re-cue (no hold concept without a needle) |

RIDE arms at the 12 ms window; **green certification** needs 8 ms — a stage
is not a certification; the two thresholds are deliberately distinct.

### Honesty gating (two degraded styles, not three)

Element alpha = `min(bpmConf_inc, bpmConf_mas, carrierQuality)`, floor 0.15
(the shipped PhaseClockView floor). Degraded carrier (lockState 2) just
lowers alpha on the live band.

1. **Stale-frozen** — hand on record (3) or carrier lost (0) or pitch nil:
   the band freezes as a 25 % grey ghost **at its last honest offset — it
   never slides toward the line** (degraded data may imitate a problem,
   never success); deposits halt (the gap in the chain shows it).
2. **Withdrawn** — grid/BPM missing on either side: band retracts into the
   line; belt-only if ΔBPM computable; else empty grey.

**Green is a conjunction** (all numeric, none discretionary):

- |φ| < 8 ms sustained 4 master beats, AND
- |ΔBPM (octave-folded)| < 0.02, AND
- bpmConfidence ≥ 0.5 on both decks, AND
- pitchSettled on both driven decks (engine `pitch_settled` semantics), AND
- timecodeLockState == 1 on both timecode-driven decks
- 600 ms attack; revoke < 120 ms.

**Grid-era clause (pre-ODF):** while phase is grid-derived, green
additionally requires grid-trust on both decks. v0 producer: grid-trust :=
`bpmConfidence ≥ 0.8` (proxy) — a per-deck "tap-confirmed grid" flag from
the prep/tap-to-grid flow is the intended producer once it's published;
confirmation lives in prep UI, never in the gutter. Until trust:
a fully seated band renders **bright white, never green**. White says
"seated, as true as my inputs"; green says "certified". The ODF swap makes
the clause vanish.

### Math (today's data; the R8 contract)

Per deck: `liveBPM = trackBPM × (1 + pitch/100)`;
`beatPhase = frac((playhead − anchor) × trackBPM / 60)`.

**Half-beat wrap (sticky rail):** crossing zero flips the shown side
immediately, but crossing the ±half-beat rail pins the band dimmed
beyond the rail (chevron-tipped, non-actionable) until the phase has
come 0.15 beat past it — no teleport, no mid-wrap filter garbage, and
the release discontinuity clears the smoothing history.

**Tempo error comes from measured slip, not the pitch readout.** The
engine's `pitch%` is display-filtered for steady header digits (median
prefilter + persistence-gated pole) — it lags the fader and moves in
steps, which makes a strobe null impossible to ride. The belt, the
green-gate ΔBPM check, and the Δ fine print therefore use the
least-squares dφ/dt over the last ~0.8 s of playhead-derived phase
(groove truth: continuous, instant under the fader, immune to the
display filter). Slip exists only while **both** grooves are moving; a
paused deck falls back to the pitch/grid *prediction* ("if you drop it
now"). Pitch-derived live BPM survives only there and in the coarse
octave-fold ratio choice.

**Octave folding (required for 87↔174 blends):** `k = round(log₂(liveBPM_inc
/ liveBPM_mas))`, clamped to ±2, adopted only when the folded ratio is
within 6 % of 1 and stable ≥ 1 s (sticky — its own hysteresis so the band
never jumps on an estimate flip). Phases are folded into the **faster**
deck's beat domain before wrapping:
`φ_beats = wrap±½(beatPhase_fast − frac(beatPhase_slow × 2^|k|))`,
`φms = φ_beats × beatMs(fast domain)`, sign normalized to incoming−master.
ΔBPM compared in the folded domain. Both decks sampled **at the same frame
timestamp** (kills frame-quantization error). Drift: 1.0 BPM @ 120 ≈
8.3 ms/s ≈ 12.5 px/s on the band scale.

**Input contract** (so the grid→ODF swap is invisible):

| Consumer | Signals |
|---|---|
| Renderer | φms, fold k, ΔBPM_folded, confidence α, stage, roles, certification, deposits[], holdBeats, coach |
| State machine | per-deck lockState, isPlaying, raw playhead (backjump detection — frac-wrapped beatPhase can't express > 1-beat jumps), pitchPercent *value* (coach clearing), pitchSettled, bpm, bpmConfidence, gridAnchor |

All state-machine inputs are transport facts, analysis-independent — the
swap replaces only the φ/Δ producers.

### Ride coaching (dual estimator)

- **Nudge detection requires a needle.** Only a timecode deck has a
  platter to push or drag; internal-deck seeks are jumps, not
  corrections, and never feed the coach.
- **A nudge** = a phase step ≥ 15 ms toward zero within 500 ms cutting the
  error ≥ 40 %, or a hand blip < 1.2 s followed by phase reduction; 2-beat
  debounce. A hold > 1.2 s is not a nudge and suspends deposits (scratch
  cuts can't pollute the tally).
- **Residual tempo** = EMA (τ = 16 master beats) of dφ/dt measured
  **strictly between corrections** — nudges are phase steps; inter-nudge
  drift is uncontaminated tempo truth; never fold nudges into BPM (a nudge
  is ambiguous between phase fix and tempo evidence — the research's
  explicit warning, and why a *deliberate groove offset* — displaced but
  still — never triggers coaching).
- **Arm** when ≥ 3 same-sign corrections inside 64 beats AND sign(EMA)
  agrees AND |EMA| ≥ 0.4 ms/s — the 0.05 trim floor must never quote
  measurement noise. EMA samples only well inside the rails (|φ| < 0.3
  beat); a pinned band carries no usable drift information. Trim =
  |EMA drift ms/s| / 10 → %, rounded to 0.05, clamped 0.05–0.8.
  (0.83 ms/s @ 120 BPM = 0.1 BPM ≈ 0.08 %.)
- **Clear** when settled pitch moves ≥ half the suggestion in the called
  direction (silent fade — no green tick; green stays sacred), or the
  drift sign flips.

### Pre-build gates (non-negotiable before rig time)

1. Offline scrub-harness calibration of drift gain + jitter floor
   (reproduce-offline-before-rig discipline).
2. Property/unit tests on the role + face machine against synthetic juggling
   and double-drop traces.
3. 2 m dark-room legibility pass on band, ticks, hold-line; if the single
   band fails peripheral pickup, the Strobe Veil (alternate B) is the
   designed exit, criteria below.

### Why a DJ reads it on first contact

It is the Technics strobe generalized to two decks. The only skills demanded
are the two a vinyl DJ already owns — null a drift, seat a thing on a line.
The line is the room and cannot move; the band wears your tint and answers
your hand within a frame, in the same direction your record moves, on the
same axis the waveforms scroll. The flam your ear hears is the gap your eye
sees, at the same "now". And it has a caddie's manners: silent while your
hand is down, dark when you're right, and it only suggests a club after
watching three of your swings.

---

## 3. Alternates (designed exits, not also-rans)

**A · LAMP — master-beat stroboscope.** Every master beat *photographs*
where your beat is; discs pile when matched, ladder when drifting; nothing
interpolates, ever. If Stillpoint fails on the rig because continuous 60 Hz
motion amplifies decode jitter into a nervous near-lock band, LAMP is immune
by construction: between flashes nothing moves, and a false green would need
four consecutive false *measurements*, not one optimistic frame. One
mechanism from cue to ride — no faces to mis-flip. Sacrifices: up to one
beat of verdict latency (0.86 s at 70 BPM dub — painful for exactly our
core genre), near-blindness below 0.05 BPM, slower tempo null than the belt.

**B · STROBE VEIL — full-height beat curtain.** A curtain of dim
incoming-tinted bands over fixed reference dots, full gutter height:
*streams* when tempo is off, *sits displaced* when phase is off, fades to
near-black at lock. If Stillpoint's single band proves too small for true
peripheral pickup at 2 m — its one unvalidated bet — the Veil is the
redundancy answer: the read is a property of the whole texture, no fixation
point, total light output proportional to wrongness. Sacrifices: endgame
precision, a positive lock confirmation, and ink. Swap criteria: band or
ladder unreadable at 2 m in the dark-room pass after one sizing iteration.

## 4. Graveyard (one line each — 21 concepts judged)

Vernier fringe (illusory optical gain; wrap-aliased false lock) · The Call
(sub-threshold at 2 m; its three-nudge trim call survives in the coach) ·
Strobe Coach (fusion choreography invisible at distance; its green
hold-line survives) · Corner Man (ledger needs a legend) · WELD (half-beat
pairing flips; its inter-nudge estimator survives) · FLAM (fusion erases
direction at the endgame; white-before-green survives) · GATES (tolerance
geometry collapses at 2 m) · Flamline (post-drop blind window; its
bar-on-line geometry + deposits survive) · Kick Strobe (true-scale dead
zone) · Kick Lens (stepped zoom = false motion) · Firefly (two of four
moments unreachable) · Vernier Strobe (ride magnification unreadable as a
mode) · Fuse (clips early drops) · Ember (freezes its own hero moment) ·
Sparkfall (locked = empty air, indistinguishable from dead) · THE SEAM
(one 3 px stroke, no peripheral fallback; its required-travel doctrine and
re-zero-at-release survive) · Master Rails (waveform-renderer annexation) ·
Phantom (locked-as-absence is forgeable by frozen data).

## 5. What we deliberately do NOT show

- **No numbers in the primary read.** Δms/ΔBPM are 9 pt fine print, hidden
  in RIDE. If the digits beat the picture, delete the picture.
- **No symmetric A/B meter, ever.** The master is the line, not a contestant.
- **No horizontal/left-right axes.** Vertical or nothing.
- **No downbeat flags or bar counters.** Beats only — bar emphasis lies on
  one-drop reggae and halftime; the deleted feature is the honest feature.
- **No vertical fader pictograms.** `+`/`−` only — the Technics fader
  physically inverts.
- **No beat-rate animation.** Motion is reserved for error, so any motion is
  a true alarm and lock is literal stillness.
- **No trophies/checkmarks.** Lock is a quiet line that grows; "done!"
  teaches letting go at exactly the wrong moment.
- **No on-air claims.** We have no mixer data; we frame "the record I'm
  correcting", never "the record the room hears".
- **No advice while the hand is on the record; no interpolation across data
  gaps.** Stale must look stale; degraded data may imitate a problem, never
  success.
- **No legends or tooltips.** Any element that needs one in the dark-room
  test dies, by this doc's own rule.

## 6. Implementation map (v0 prototype, this round)

- `apple/Dub/Performance/StillpointModel.swift` — pure logic: role scores,
  stage FSM, octave fold, φ/Δ math, sustain windows, deposits, certification,
  nudge/coach estimators. No UI imports; deterministic per `(now, inputs)`;
  unit-tested.
- `apple/Dub/Performance/StillpointView.swift` — TimelineView + Canvas
  renderer over a `StillpointFrame`; fixed-frame init for previews and
  snapshots.
- Mounted in the centre gutter by `PerformanceView` (one-line swap with the
  retired `BeatmatchStackView`, which stays in-tree until the rig verdict).
- Tests: `DubTests/StillpointModelTests.swift` (FSM transitions, fold,
  false-green impossibility, coach arming) +
  `DubTests/StillpointSnapshotTests.swift` (five canonical frames).
