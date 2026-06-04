# Dub macOS app — `apple/`

This directory hosts the AppKit/SwiftUI shell that consumes the Rust core
through UniFFI.

## Status: Performance + Prep shell, library browser, beat grid

The app opens a performance window with two deck columns, a library browser,
Metal waveforms with a beat-grid overlay, tap-to-grid editing from the deck
header, and a status strip. When no multi-channel interface is connected it
boots the Track Preparation Mode shell instead (file playback at horizontal
waveform resolution). On cold boot a brief launch splash fades out once the
engine and library are ready. Click **DUB** in the top-left status strip, or
choose **Dub → About Dub**, for the About sheet (splash artwork, version info,
links to the repo and PRD). App icon and splash live in `Dub/Assets.xcassets/`.

## One-shot bootstrap

```bash
brew install xcodegen                  # one-time
./scripts/bootstrap.sh                 # xcframework + Swift bindings + Xcode project
make app                               # build Dub.app into apple/build/
make run-app                           # build + launch
# Or: open apple/Dub.xcodeproj and ⌘R
```

Re-run `./scripts/bootstrap.sh` whenever:

- `apple/project.yml` changes — regenerates `Dub.xcodeproj`.
- `crates/dub-ffi/src/lib.rs` changes — rebuilds the xcframework and
  regenerates the Swift bindings.
- `apple/Dub/Waveform/Shaders.metal` changes — Xcode rebuilds the
  `default.metallib` automatically on the next build.

## Layout

```
apple/
├── project.yml                    XcodeGen manifest (source of truth)
├── Dub.xcodeproj/                 Generated (gitignored)
├── DubCore.xcframework/           Generated (gitignored) — universal Rust static lib
├── Dub/
│   ├── DubAppDelegate.swift       AppKit @main lifecycle
│   ├── MainWindowController.swift NSWindow holding an NSHostingController(MainView)
│   ├── MainView.swift             SwiftUI shell + app model (engine, library, decks)
│   ├── Performance/               two-deck surface: PerformanceView, DeckHeader,
│   │                              LibraryView, TapToGridController,
│   │                              TrackOverviewView, StatusStrip, BeatgridCalibrationLog
│   ├── Waveform/
│   │   ├── Shaders.metal          Vertex (instanced quads) + fragment shaders
│   │   ├── WaveformRenderer.swift Metal renderer (chunks ring, triple-buffered uniforms, beat grid)
│   │   ├── WaveformRenderThread.swift  Dedicated off-main render thread (CVDisplayLink-driven)
│   │   ├── WaveformMetalHostView.swift / WaveformView.swift  NSViewRepresentable wrapping MTKView
│   │   ├── PlayheadMarker.swift   Shared playhead source for envelope + grid
│   │   └── EngineHostTimeMapping.swift  Audio-clock → host-time extrapolation
│   ├── Preferences/PreferencesSheet.swift   Device / channel / key-remap settings
│   ├── DesignSystem/Tokens.swift  Spacing / colour / type tokens
│   ├── About/                     AboutSheet + LaunchSplashOverlay
│   ├── App/AppNotifications.swift Menu-bar → SwiftUI notification bridge
│   ├── Assets.xcassets/           AppIcon + AboutSplash image sets
│   ├── Info.plist                 Placeholder — keys overridden by XcodeGen
│   └── Dub.entitlements           Sandbox off (local-only signing)
└── DubShared/
    ├── Package.swift              Swift Package wrapping DubCore.xcframework
    └── Sources/DubCore/
        ├── Placeholder.swift
        └── Generated/             UniFFI Swift bindings (gitignored)
```

## Why a hybrid AppKit + SwiftUI app?

- **AppKit owns the lifecycle.** The M10 waveform needs `MTKView`
  through `NSViewRepresentable`, plus future `NSEvent` hooks that
  SwiftUI either doesn't expose or re-exposes through awkward bridges.
  AppKit gives us the lowest-overhead path.
- **SwiftUI owns non-realtime sub-views.** `MainView`, the library
  browser, the deck header, the palette picker, and Preferences /
  About sheets are pure forms; SwiftUI's declarative model is faster
  to iterate on than AppKit's view-by-view layout.
- **The waveform itself is `MTKView` (Metal) wrapped in
  `NSViewRepresentable`.** A dedicated render thread (driven by
  `CVDisplayLink`, not the main run loop) polls `DubEngine.peaksLen` +
  `peaksExtend` each frame and draws off the main thread; no callback
  path from the audio thread ever reaches Swift.

## Signing

The app uses **"Sign to Run Locally"** only. No Apple Developer
account, no notarisation. Distribution signing + notarisation land as
their own milestone in v1.1 (PRD §12.2, M23).

## See also

- `docs/spec/PRD.md` §10.1 — Workspace layout
- `docs/spec/PRD.md` §10.3 — Apple frontend stack
- `.cursor/rules/swift.mdc` — Swift conventions
- `.cursor/rules/ffi.mdc` — Rust ↔ Swift contract
