# Dub Docs Routing Guide

Use this file to choose the smallest useful doc set for a task. The goal is to
avoid loading the whole `docs/` folder when one anchored section is enough.

## Source Of Truth

- `PRD.md` is the product source of truth: scope, non-goals, milestone plan,
  acceptance criteria, and user-facing behavior.
- `LIBRARY-SCHEMA.md` is the public SQLite schema contract. Load it only for
  library, schema, migration, or FFI work.
- `LICENSE-DEPENDENCIES.md` is the source of truth for dependency licenses and
  binary attribution.

## Human-facing HTML views (read-only, optional)

`docs/html/` holds an HTML dashboard generated from the Markdown sources for
human reading and sharing. Open `docs/html/index.html` in a browser. **These
are not the source of truth and AI agents should not load them**: agents read
the Markdown directly (cheaper, anchorable, single source). Refresh manually
when the Markdown diverges.

| View | Backs the Markdown |
| --- | --- |
| `html/index.html` | Project landing + non-negotiables + doc routing card |
| `html/architecture.html` | `ARCHITECTURE.md` (visual crate graph, RT-safety layers, messaging) |
| `html/roadmap.html` | `PRD.md` §12 + `SHIPPED.md` TOC (milestone timeline) |
| `html/schema.html` | `LIBRARY-SCHEMA.md` (interactive ER diagram + table reference) |
| `html/backlog.html` | `UI-BACKLOG.md` (kanban with filter chips) |

## Load By Task

| Task | Read |
| --- | --- |
| Product scope, out-of-scope, milestone planning | Relevant `PRD.md` section |
| Why a past implementation looks this way | Anchored section in `SHIPPED.md` |
| Crate/threading/FFI structure | `ARCHITECTURE.md` overview, then relevant section |
| Library DB, migrations, FTS, analysis cache | `LIBRARY-SCHEMA.md` |
| Serato/Traktor/rekordbox/iTunes import quirks | `LIBRARY-FORMATS.md` |
| SwiftUI/AppKit UI polish backlog | `UI-BACKLOG.md` |
| Beat-grid BPM octave / tap-to-grid work | `UI-BACKLOG.md` B-11, U-19; PRD §8.3.1 |
| License review, release acknowledgements | `LICENSE-DEPENDENCIES.md` |

## Context Budget Rules

- Do not read all of `SHIPPED.md` by default. It is historical archaeology;
  load an anchor from a PRD or code reference.
- Do not read all of `PRD.md` for implementation work. Start with the relevant
  section, then follow links.
- Prefer code plus `ARCHITECTURE.md` for "how does this work today?" questions.
  Use `SHIPPED.md` only when the task asks "why did this land this way?"
- Keep backlog files task-specific. `UI-BACKLOG.md` should not be loaded for
  engine, DSP, library schema, or license work.

## Maintenance

When adding a new doc, update this routing guide in the same change. When a
backlog item ships, either remove it or mark it fixed with the shipped anchor.
When the Markdown source of an HTML view in `docs/html/` changes in a way that
materially alters the view (new milestone, new schema table, retired backlog
item, new architecture diagram), refresh the corresponding HTML by re-reading
the source and updating the static page. HTML refresh is manual; the Markdown
remains canonical.
