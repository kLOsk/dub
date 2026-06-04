# Dub Docs Routing Guide

Use this file to choose the smallest useful doc set for a task. The goal is to
avoid loading the whole `docs/` folder when one anchored section is enough.

## Folder layout

```
docs/
  README.md            this routing guide
  UI-BACKLOG.md        open UI/UX work (+ a closed archive)
  spec/                binding spec + reference (source of truth)
    PRD.md  PRD-BEATS.md  ARCHITECTURE.md
    LIBRARY-SCHEMA.md  LIBRARY-FORMATS.md  LICENSE-DEPENDENCIES.md
  history/             durable, backward-looking
    SHIPPED.md         one-line-per-milestone index (detail in git)
    LESSONS.md         pitfalls + load-bearing decisions
  investigations/      research / runbooks
    BPM-DETECTOR-V2-INVESTIGATION.md  WAVEFORM-JITTER-CAPTURE.md
  html/                read-only status dashboard (index / roadmap / backlog)
```

Doc names below are unique, so they're referenced by bare name; the folder map
above says where each lives.

## Source of truth — `spec/`

- `PRD.md` is the product source of truth: scope, non-goals, milestone plan,
  acceptance criteria, and user-facing behavior.
- `PRD-BEATS.md` is the sub-spec for tempo, beat grid, downbeat, tap-to-grid,
  and the waveform overlay contract. Replaces PRD §8.3.1 and is binding for
  any code under `crates/dub-bpm/`, the tap path in `crates/dub-ffi/`, and
  the grid renderer in `apple/Dub/Waveform/`. Round-by-round beat-grid
  hardening history lives here, not in `SHIPPED.md`.
- `LIBRARY-SCHEMA.md` is the public SQLite schema contract. Load it only for
  library, schema, migration, or FFI work.
- `LICENSE-DEPENDENCIES.md` is the source of truth for dependency licenses and
  binary attribution.

## Lessons and history — `history/`

- `LESSONS.md` is the distilled "don't repeat these mistakes" file: the
  hard-won pitfalls and load-bearing decisions (RT-safety, timecode lift,
  BPM octave ceiling, waveform sample-rate drift, FFI versioning, library
  reachability, …). **Read the relevant section before touching a subsystem.**
- `SHIPPED.md` is now a one-line-per-milestone **index** of what has shipped,
  in build order. The detailed per-milestone write-ups moved to git history —
  `git log` the crate, or read the landing commit, when you need the full
  archaeology. Durable rationale was lifted into `LESSONS.md`.

## Investigation notes and runbooks — `investigations/`

- `BPM-DETECTOR-V2-INVESTIGATION.md` records why a classical (non-ML) detector
  does not beat the tuned Classic `dub-bpm` octave logic, and the learned
  beat-tracker plan that can. Load before any "replace the BPM detector" work;
  the experimental code it describes was measured and removed.
- `WAVEFORM-JITTER-CAPTURE.md` is the `os_signpost` capture runbook for
  diagnosing waveform / beat-grid jitter regressions. The originally
  investigated jitter was fixed end to end; this remains the procedure if it
  recurs (the probes and `make trace-grid` targets are still wired).

## Human-facing HTML dashboard (read-only, optional)

`docs/html/` is a **status dashboard only** — three hand-kept pages, openable
from `file://` with no build step. It is deliberately *not* a mirror of the
reference docs (the old `architecture.html` / `schema.html` / `beats.html`
duplicated Markdown and rotted, so they were deleted). **AI agents should not
load these** — read the Markdown directly. See `html/README.md` for the rule.

| Page | Mirrors |
| --- | --- |
| `html/index.html` | Project landing + non-negotiables + doc routing |
| `html/roadmap.html` | `PRD.md` §12 + `SHIPPED.md` (milestone timeline, filter chips) |
| `html/backlog.html` | `UI-BACKLOG.md` (kanban with filter chips) |

## Load By Task

| Task | Read |
| --- | --- |
| Product scope, out-of-scope, milestone planning | Relevant `PRD.md` section |
| Pitfalls before touching a subsystem | Relevant `LESSONS.md` section |
| Why a past implementation looks this way | `LESSONS.md`, then `git log` the crate (`SHIPPED.md` indexes the milestone) |
| Crate/threading/FFI structure | `ARCHITECTURE.md` overview, then relevant section |
| Library DB, migrations, FTS, analysis cache | `LIBRARY-SCHEMA.md` |
| Serato/Traktor/rekordbox/iTunes import quirks | `LIBRARY-FORMATS.md` |
| SwiftUI/AppKit UI polish backlog | `UI-BACKLOG.md` |
| Beat-grid BPM octave / tap-to-grid / downbeat / waveform-overlay work | `PRD-BEATS.md` (source of truth); `BPM-DETECTOR-V2-INVESTIGATION.md` before any detector replacement |
| License review, release acknowledgements | `LICENSE-DEPENDENCIES.md` |

## Context Budget Rules

- `SHIPPED.md` is now a short index — fine to skim, but it has no detail; for
  "why did this land this way?" read the matching `LESSONS.md` section, then
  `git log` the crate.
- Do not read all of `PRD.md` for implementation work. Start with the relevant
  section, then follow links.
- Prefer code plus `ARCHITECTURE.md` for "how does this work today?" questions.
- Keep backlog files task-specific. `UI-BACKLOG.md` should not be loaded for
  engine, DSP, library schema, or license work.

## Maintenance

When adding a new doc, update this routing guide in the same change. When a
backlog item ships, move it to the `UI-BACKLOG.md` "Closed (archive)" list and,
if it carries a durable lesson, add that to `LESSONS.md`. When a milestone
ships, add its one-liner to `SHIPPED.md` and reflect the headline in
`html/roadmap.html`. Keep `docs/html/` to the three status pages — reference
material belongs in Markdown, never a new HTML page (see `html/README.md`).
