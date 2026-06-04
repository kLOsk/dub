# CLAUDE.md

Claude Code loads this file automatically. It wires Dub's existing agent context
(shared with Cursor) into Claude Code so both tools behave the same way.

## Project context

@AGENTS.md

## Always-on rule

@.cursor/rules/prd-discipline.mdc

## Path-scoped rules

These mirror the `globs` in `.cursor/rules/*.mdc`. Nested `CLAUDE.md` files load
the matching rule automatically when you read or edit files in that subtree, so
you normally don't need to act on this table — it's the map:

| When working in…                                      | Rule loaded                  |
|-------------------------------------------------------|------------------------------|
| any Rust under `crates/` or `tools/`                  | `rust-general` + `testing`   |
| `crates/dub-engine`, `dub-dsp`, `dub-stretch`         | `audio-rt` + `dsp`           |
| `crates/dub-stretch`, `dub-ffi`, `dub-fingerprint`    | `ffi`                        |
| `apple/**/*.swift`                                     | `swift`                      |

The rule bodies live in `.cursor/rules/` and are the single source of truth for
both tools — edit them there, not here.

## Tool-name mapping (Cursor → Claude Code)

`AGENTS.md` refers to Cursor's tool names. The Claude Code equivalents:

- `StrReplace` / `Write` → `Edit` / `Write`
- `SemanticSearch` → the `Explore` agent (or `Grep` for exact symbols)
- `Read` and `Grep` are the same in both

## Formatting hook

`.claude/settings.json` runs `.claude/hooks/rust-fmt.sh` on `PostToolUse` after
every `Write`/`Edit`/`MultiEdit`, formatting any `.rs` file with `rustfmt`. This
mirrors Cursor's `afterFileEdit` hook (`.cursor/hooks/rust-fmt.sh`). It fails
open, so it never blocks edits.
