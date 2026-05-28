## Waveform jitter capture procedure

The waveform renderer emits Instruments-friendly signposts so the grid-jitter symptom can be diagnosed without sprinkling `print` calls in the hot path. The probes are zero-cost when no Instruments subscriber is attached.

### What gets emitted

`WaveformRenderer.drawImpl` (one per draw, on the render thread):

| Signpost                                                          | Type    | Meaning                                                                                                                                  |
|-------------------------------------------------------------------|---------|------------------------------------------------------------------------------------------------------------------------------------------|
| `draw` (subsystem `com.klos.dub.waveform`, category `GridTrace`)  | interval | Begin / end markers around every draw call. Carries `deck`, drawable `w`/`h`.                                                            |
| `grid` (same log, inside the `draw` interval)                     | event   | All grid-math inputs in one record: `playhead`, `peakDur`, `contChunkF`, `snap`, `eps`, `ndcPast`, `ndcFuture`, `drawnAbove`, `drawnBelow`, `peaksLen`, `hasTrack`. |

`WaveformRenderThread.drawIfNeeded` (one per frame; subsystem `com.klos.dub.waveform`, category `RenderThread`):

| Signpost                  | Type  | Meaning                                                                                       |
|---------------------------|-------|-----------------------------------------------------------------------------------------------|
| `drawIfNeeded.thread`     | event | Reports `isMain=0` when the draw runs on the dedicated render thread (expected). `isMain=1` ever means we regressed. |

### Capturing a trace — from the command line (preferred)

`os_signpost` records are written to macOS unified logging but **`log stream` does not stream signpost events** (this is a quirk of the `log` CLI). You have to use `log show` to dump them. Two make targets wrap this so you don't have to think about it:

* **Marker + dump** (recommended):

  ```
  # Terminal 1
  make run-app

  # Terminal 2
  make trace-grid
  # Prints a marker timestamp, then waits.
  # Reproduce the jitter in Dub.
  # Press Enter in the trace-grid terminal.
  # /tmp/dub-grid-trace.log now contains every signpost emitted
  # between the marker and "Enter". Override path with OUT=foo.log.
  ```

* **Window dump** (when you forgot to mark the start):

  ```
  make run-app
  # ... reproduce the jitter, then quit Dub ...
  make trace-grid-last LAST=5m
  # Override window with LAST=10m, output path with OUT=path.
  ```

Both targets filter on `subsystem == "com.klos.dub.waveform"`, so they capture every `draw` interval, every `grid` event, and every `drawIfNeeded.thread` event. The output is the macOS unified-logging `compact` style:

```
TIMESTAMP               Sp Dub[PID:TID] [SUBSYSTEM:CATEGORY] [spid …, process, KIND] NAME: KEY=VALUE …
```

One frame is one `draw` begin / `grid` / `draw` end triple, with a sibling `drawIfNeeded.thread` event marking the render thread tick. Diff frame-to-frame to find the bad frame.

### Capturing a trace — from Instruments (optional)

1. Launch Dub from Xcode (Debug or Release; signposts are not stripped).
2. Open Instruments → choose the `Logging` template (or `System Trace` if you also want CPU thread activity).
3. In the Logging instrument's filter, set the subsystem to `com.klos.dub.waveform`.
4. Press record. Load a track on deck A, hit play, wait for the grid step to appear. Stop the recording.
5. Inspect the `draw` interval lane. Each interval has an inline `grid` event with the math inputs as readable text.

### Reading the trace

Look at the frame immediately before the visible grid step and the frame after. The cause is one of:

* `drawnAbove` or `drawnBelow` differ between the two frames. The drawable size changed mid-playback (layout pass fired). Fix at the source: stabilise the layer's drawable size during steady playback.
* `peakDur` differs. The engine reported a new peaks-chunk cadence. Track-load races or source-swap not getting masked by the `framesSinceSourceSwap > 20` gate.
* `snap` advances by more than 2 in a single frame, or backwards. Audio thread is publishing a non-monotonic playhead. Inspect `Deck::store_position_secs` ordering and the rate logic.
* `eps` jumps without `snap` advancing. The continuous playhead made a step that did not cross the snap boundary; suggests a coarse-grained position write on the audio thread.
* All values transition smoothly across the visible step. The math is correct frame-to-frame and the wobble is elsewhere (Metal MSAA sample positions, layer contentsGravity, fractional NSView positioning under live resize). Move to a `System Trace` capture with `Display` instruments active.

### Disabling the probes

`os_signpost` is essentially free without a subscriber. If you want to strip the emit calls entirely (e.g. a paranoid release build), wrap each call in `#if DUB_TRACE_GRID … #endif`. For day-to-day work just leave them on.
