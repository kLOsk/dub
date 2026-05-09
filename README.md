# Dub

> A timecode-vinyl DJ application built for scratch DJs and vinyl enthusiasts.
> Mac-first. Rust-cored. GPLv3.

Dub is the spiritual successor to Serato Scratch Live for the urban music scene
(hip hop, reggae, dnb, dubstep, scratch). Two decks, external mixer, real records
**through** the software, smart utility FX, fast sample throws.

This is **pre-alpha software**. There is no release. There is `main`.

## Status

- **Phase A — pre-alpha development.** No external users. Author dogfoods on `main`.
- See `docs/PRD.md` for the full product spec.
- See `docs/ARCHITECTURE.md` for engineering notes.
- Reliability staging is described in PRD §2.2.0 — pragmatism before users, rigor before stable.

## Repo layout

```
dubjay/                              repo root (workspace)
├── Cargo.toml                       Rust workspace
├── crates/
│   ├── dub-engine/                  audio graph, transport, RT-safety
│   ├── dub-dsp/                     resamplers, filters, FX
│   ├── dub-stretch/                 Rubber Band FFI wrapper (placeholder)
│   ├── dub-io/                      symphonia-based decoders (placeholder)
│   ├── dub-timecode/                Serato + Traktor timecode decoder (placeholder)
│   ├── dub-thru/                    Thru-mode pipeline + auto-detection (placeholder)
│   ├── dub-fingerprint/             Chromaprint FFI (v1.1, placeholder)
│   ├── dub-library/                 SQLite + library imports (placeholder)
│   ├── dub-controller/              HID/MIDI abstractions (placeholder)
│   ├── dub-ffi/                     UniFFI Swift bindings (placeholder)
│   └── dub-cli/                     headless smoke test
├── apple/                           SwiftUI/AppKit shell (M0.5)
├── tools/
│   └── rt-audit/                    RT-thread allocation auditor
├── docs/                            PRD, architecture, ADRs
├── scripts/                         build helpers
├── .cursor/                         Cursor rules + hooks for AI-assisted dev
└── AGENTS.md                        always-loaded project context for AI
```

## Quickstart

```bash
make test          # cargo nextest run + clippy
make smoke         # run the CLI smoke test
make rt-audit      # run the RT-safety harness
```

See the `Makefile` for more targets.

## License

GPLv3 — see `LICENSE`.

This means: if you distribute a binary based on this code, you must release the
source under GPLv3 too. We chose GPL deliberately so that the engine improvements
made by anyone in the community come back to the community.

## Contributing

This is currently a single-developer project. Contributions are welcome but
expect reviews to be opinionated about reliability and the No-Mouse-DJ-Ever
philosophy. Read `docs/PRD.md` first.
