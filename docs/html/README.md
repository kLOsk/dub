# docs/html — project status dashboard

This folder has **one job**: an at-a-glance, filterable status surface you can
open from `file://` with no build step. It is *not* a mirror of the reference
docs.

**Pages (the only three that should exist here):**

- `index.html` — landing / overview + the non-negotiable design principles.
- `roadmap.html` — where we are and what's next (filter chips for
  shipped / in-flight / planned). Mirrors `../spec/PRD.md §12` + `../history/SHIPPED.md`.
- `backlog.html` — open UI/UX work as a kanban. Mirrors `../UI-BACKLOG.md`.

**The rule:** reference and spec live in markdown (`../spec/PRD.md`,
`../spec/ARCHITECTURE.md`, `../spec/LIBRARY-SCHEMA.md`, `../spec/PRD-BEATS.md`,
`../history/LESSONS.md`). Do **not** add HTML pages that restate them — that's what
created the drift we just deleted (`architecture.html`, `schema.html`,
`beats.html` duplicated markdown and rotted). Link out to the `.md` instead.

**Maintenance:** these three are hand-kept and intentionally shallow, so
updating them is cheap. When a milestone ships or the backlog changes, touch
the matching markdown first (source of truth), then reflect the headline in the
dashboard. Don't deep-link into `SHIPPED.md` anchors — it's a short index now;
link the file.

Shared styling/behaviour is in `_assets/dub.css` + `_assets/dub.js` (vanilla,
zero-dependency; `dub.js` only attaches a feature if its DOM hook is present).
