# Dub macOS app — `apple/`

This directory will host the SwiftUI/AppKit shell that consumes the Rust core.

## Status: M0.5 (placeholder)

The Apple-side scaffold lands as **M0.5** — immediately after the Rust workspace
M0 is in place. M0 (this commit) ships only the Rust core + CI; the Xcode
project follows once we run XcodeGen or generate one via Xcode template +
check it in.

## Planned layout

```
apple/
├── DubCore.xcframework/           Generated artifact from `dub-ffi` (gitignored)
├── Dub.xcodeproj/                 SwiftUI macOS app project
├── Dub/
│   ├── DubApp.swift               @main entry point
│   ├── ContentView.swift          Root view
│   ├── Decks/                     Deck UI (SwiftUI + Metal)
│   ├── Library/                   Library browser
│   ├── Waveform/                  MTKView-based waveform renderer
│   └── Resources/
└── DubShared/                     Swift package wrapping the xcframework
    ├── Package.swift
    └── Sources/
        └── DubCore/               UniFFI-generated bindings + thin helpers
```

## M0.5 plan

1. **Generate `DubCore.xcframework`** from `crates/dub-ffi`:
   ```bash
   ./scripts/build-xcframework.sh
   ```
   (Script written in M0.5; uses `cargo build --target aarch64-apple-darwin`,
   `--target x86_64-apple-darwin`, `lipo`, `xcodebuild -create-xcframework`,
   and UniFFI's bindgen.)
2. **Create `Dub.xcodeproj`** via Xcode template ("macOS App", SwiftUI lifecycle).
3. **Add `DubShared` Swift package** referencing the xcframework.
4. **Implement smoke screen**: app launches, calls `DubCore.greeting()` from
   the Rust core, displays "Dub engine OK · v0.0.1".
5. **Verify `cargo build --release` + `xcodebuild test` both green in CI**.

## Why this isn't in M0

Generating a working `.xcodeproj` purely from text editing is brittle. Doing
it via Xcode + checking in the result is more reliable, but requires Xcode
on the developer's machine. M0 ships the parts that are pure-text-stable
(Rust workspace, CI, AGENTS.md, hooks, rules); M0.5 lands the Apple side
once the developer is at a Mac with Xcode and can run the bootstrap script.

## See also

- `docs/PRD.md` §10.1 — Workspace layout
- `docs/PRD.md` §10.3 — Apple frontend stack
- `.cursor/rules/swift.mdc` — Swift conventions
