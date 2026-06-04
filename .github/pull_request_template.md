<!-- Thanks for contributing! See AGENTS.md and docs/spec/PRD.md for context. -->

## Summary

<!-- What does this PR do? Keep it to 1–3 sentences. -->

## Why

<!-- The "why" matters more than the "what". Reference PRD sections by number. -->

## Test plan

<!-- How did you verify this works? Mandatory section. -->

- [ ] Unit tests added or updated
- [ ] `cargo nextest run --workspace` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] `make rt-audit` passes (if this PR touches `dub-engine` or `dub-dsp`)
- [ ] If touching the audio thread: confirmed RT-safety with `assert_no_alloc`
- [ ] If touching parsers: fuzz target updated / fuzzed for ≥ 60s

## PRD impact

<!-- Does this change anything in the PRD? Update docs/spec/PRD.md if so. -->

## Out-of-scope check

<!-- Confirm this PR does not silently expand v1 scope into PRD §15 territory.
     If it does, explain why. -->

## Notes for reviewers

<!-- Anything specific to look at, follow-up work, etc. -->
