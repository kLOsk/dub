.DEFAULT_GOAL := help

# Pin the Apple build output to a predictable location inside the repo
# (apple/build/, which is gitignored) instead of Xcode's default
# DerivedData cave under ~/Library. Re-export so recipes see the value.
APP_BUILD_DIR ?= $(CURDIR)/apple/build
APP_CONFIG    ?= Debug
APP_BUNDLE     = $(APP_BUILD_DIR)/Build/Products/$(APP_CONFIG)/Dub.app

.PHONY: help fmt fmt-check clippy test smoke rt-audit cov fuzz-quick soak clean ci docs-check app app-release run-app open-app xcframework

help:
	@echo "Dub — common targets"
	@echo "  make test          run all tests (cargo nextest + clippy)"
	@echo "  make smoke         run the dub-cli smoke binary"
	@echo "  make rt-audit      run the RT-safety harness"
	@echo "  make fmt           cargo fmt"
	@echo "  make fmt-check     cargo fmt --check"
	@echo "  make clippy        cargo clippy --all-targets -- -D warnings"
	@echo "  make cov           coverage report (requires cargo-llvm-cov)"
	@echo "  make fuzz-quick    run fuzz targets for 60s each (placeholder)"
	@echo "  make soak          1-hour offline render soak (placeholder)"
	@echo "  make ci            run the full CI pipeline locally"
	@echo "  make docs-check    fail if README / docs/html drift from code constants"
	@echo "  make clean         cargo clean"
	@echo ""
	@echo "Apple shell"
	@echo "  make app           build Dub.app (Debug) -> apple/build/Build/Products/Debug/Dub.app"
	@echo "  make app-release   build Dub.app (Release)"
	@echo "  make run-app       build + launch Dub.app"
	@echo "  make open-app      open the apple/build/Build/Products/$(APP_CONFIG)/ folder in Finder"

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

clippy:
	cargo clippy --all-targets --workspace -- -D warnings

# Prefer nextest if installed; fall back to cargo test.
test: clippy
	@if command -v cargo-nextest >/dev/null 2>&1; then \
		cargo nextest run --workspace; \
	else \
		echo "[hint] install cargo-nextest for faster runs: cargo install cargo-nextest --locked"; \
		cargo test --workspace; \
	fi

smoke:
	cargo run -p dub-cli -- smoke

rt-audit:
	cargo run -p dub-cli -- rt-audit

cov:
	@command -v cargo-llvm-cov >/dev/null 2>&1 || { \
		echo "cargo-llvm-cov not installed. Install: cargo install cargo-llvm-cov --locked"; exit 1; }
	cargo llvm-cov --workspace --html --output-dir coverage

fuzz-quick:
	@echo "[placeholder] fuzz targets are added per parser as they land. See PRD §2.2.5."

soak:
	@echo "[placeholder] soak harness lands in M2. See PRD §2.2.0 phase B."

docs-check:
	./scripts/check-docs.sh

ci: docs-check fmt-check clippy test
	@echo "Local CI pipeline complete."

clean:
	cargo clean
	rm -rf $(APP_BUILD_DIR)

# ----- Apple shell -----------------------------------------------------
#
# All `app*` targets pin Xcode's DerivedData to $(APP_BUILD_DIR) so the
# built .app lives at a stable, repo-relative path (apple/build/Build/...).
# The first invocation also runs ./scripts/bootstrap.sh to ensure the
# Rust xcframework + UniFFI bindings + xcodegen project are present.
#
# Override the build config via APP_CONFIG=Release (or use `make
# app-release`); override the directory via APP_BUILD_DIR=/some/path.

# Internal: regenerate xcodeproj + xcframework. Re-runs are safe / fast
# (skipped work is a no-op).
$(CURDIR)/apple/Dub.xcodeproj/project.pbxproj: apple/project.yml
	./scripts/bootstrap.sh

# The Swift target links against this universal static lib. If any
# Rust FFI source is newer than it, the Apple build will silently
# pick up stale Rust behaviour (this exact trap shipped a "library
# looks empty" bug in M11d once already). Guard with shell `find`
# so an up-to-date tree pays only a few millisecond stat walk and
# skips the (slow) cargo + lipo + uniffi-bindgen pipeline.
XCFRAMEWORK_LIB := $(CURDIR)/apple/DubCore.xcframework/macos-arm64_x86_64/libdub_ffi.a

xcframework:
	@if [ ! -f "$(XCFRAMEWORK_LIB)" ] \
	    || [ -n "$$(find $(CURDIR)/crates -name '*.rs' -not -path '*/target/*' -newer $(XCFRAMEWORK_LIB) -print -quit 2>/dev/null)" ] \
	    || [ -n "$$(find $(CURDIR)/crates -name 'Cargo.toml' -newer $(XCFRAMEWORK_LIB) -print -quit 2>/dev/null)" ] \
	    || [ $(CURDIR)/Cargo.toml -nt $(XCFRAMEWORK_LIB) ] \
	    || [ $(CURDIR)/Cargo.lock -nt $(XCFRAMEWORK_LIB) ]; then \
	    echo "==> Rust FFI sources changed; rebuilding DubCore.xcframework"; \
	    ./scripts/build-xcframework.sh; \
	else \
	    echo "==> DubCore.xcframework up to date"; \
	fi

app: xcframework $(CURDIR)/apple/Dub.xcodeproj/project.pbxproj
	@mkdir -p $(APP_BUILD_DIR)
	xcodebuild build \
	    -project apple/Dub.xcodeproj \
	    -scheme Dub \
	    -configuration $(APP_CONFIG) \
	    -destination 'platform=macOS' \
	    -derivedDataPath $(APP_BUILD_DIR) \
	    | xcbeautify 2>/dev/null || \
	xcodebuild build \
	    -project apple/Dub.xcodeproj \
	    -scheme Dub \
	    -configuration $(APP_CONFIG) \
	    -destination 'platform=macOS' \
	    -derivedDataPath $(APP_BUILD_DIR)
	@echo ""
	@echo "Built: $(APP_BUNDLE)"

app-release:
	$(MAKE) app APP_CONFIG=Release

trace-grid:
	@# Capture every os_signpost from the waveform renderer into a
	@# plain-text file we can diff frame-to-frame to find the source
	@# of intermittent grid steps.
	@#
	@# IMPORTANT: `log stream` does not stream signpost events on
	@# macOS, so we use the marker-and-dump pattern: record a
	@# timestamp, you reproduce the symptom, press Enter, and we
	@# dump every signpost emitted between the marker and now.
	@#
	@# Usage:
	@#   make trace-grid             # writes /tmp/dub-grid-trace.log
	@#   make trace-grid OUT=foo.log
	@#
	@# Workflow:
	@#   1. In one terminal: make run-app
	@#   2. In another terminal: make trace-grid
	@#   3. When prompted, reproduce the jitter in the running app.
	@#   4. Press Enter in the trace-grid terminal.
	@#   5. The file path is printed; share it for analysis.
	@OUT_FILE="$(or $(OUT),/tmp/dub-grid-trace.log)"; \
	MARK="$$(date '+%Y-%m-%d %H:%M:%S')"; \
	echo "Marker: $$MARK"; \
	echo "Reproduce the grid jitter in Dub now."; \
	printf "Press Enter when you're done reproducing... "; \
	read _; \
	echo "Dumping signposts since $$MARK → $$OUT_FILE"; \
	/usr/bin/log show \
	    --predicate 'subsystem == "com.klos.dub.waveform"' \
	    --info --debug --signpost --style compact \
	    --start "$$MARK" \
	    > "$$OUT_FILE"; \
	LINES=$$(wc -l < "$$OUT_FILE"); \
	echo "Wrote $$LINES lines to $$OUT_FILE"; \
	if [ "$$LINES" -le 2 ]; then \
	    echo "  WARNING: file looks empty. Did the waveform actually draw?"; \
	    echo "  (Render thread only fires when a deck is loaded / visible.)"; \
	fi

trace-grid-last:
	@# Variant for "I forgot to mark the start": dump the last N
	@# minutes of waveform signposts. Useful when you've already
	@# reproduced and just want everything in recent memory.
	@#
	@# Usage:
	@#   make trace-grid-last                # last 5 minutes
	@#   make trace-grid-last LAST=10m       # last 10 minutes
	@#   make trace-grid-last OUT=foo.log
	@OUT_FILE="$(or $(OUT),/tmp/dub-grid-trace.log)"; \
	LAST_WINDOW="$(or $(LAST),5m)"; \
	echo "Dumping last $$LAST_WINDOW of subsystem=com.klos.dub.waveform → $$OUT_FILE"; \
	/usr/bin/log show \
	    --predicate 'subsystem == "com.klos.dub.waveform"' \
	    --info --debug --signpost --style compact \
	    --last "$$LAST_WINDOW" \
	    > "$$OUT_FILE"; \
	LINES=$$(wc -l < "$$OUT_FILE"); \
	echo "Wrote $$LINES lines to $$OUT_FILE"

run-app: app
	@# `open` only focuses an already-running instance — it will NOT
	@# relaunch with the freshly-built binary / metallib. Send a
	@# graceful AppleScript quit to any prior instance, then wait for
	@# it to exit, then launch. `osascript ... quit` is a no-op (exit
	@# code 0) if Dub isn't running, so this is safe on a cold start.
	@osascript -e 'tell application "Dub" to quit' >/dev/null 2>&1 || true
	@for i in 1 2 3 4 5 6 7 8; do \
	    if ! pgrep -x Dub >/dev/null 2>&1; then break; fi; \
	    sleep 0.25; \
	done
	@if pgrep -x Dub >/dev/null 2>&1; then \
	    echo "[run-app] Dub did not quit gracefully; force-killing"; \
	    pkill -x Dub || true; \
	    sleep 0.25; \
	fi
	@# Re-register with Launch Services so Finder/Dock drop stale icons
	@# cached against an older copy (Xcode DerivedData, pre-icon builds).
	@/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f -R "$(APP_BUNDLE)" >/dev/null 2>&1 || true
	@touch "$(APP_BUNDLE)"
	@echo "Launching $(APP_BUNDLE)"
	open "$(APP_BUNDLE)"

open-app:
	@if [ -d "$(APP_BUILD_DIR)/Build/Products/$(APP_CONFIG)" ]; then \
	    open "$(APP_BUILD_DIR)/Build/Products/$(APP_CONFIG)"; \
	else \
	    echo "No build at $(APP_BUILD_DIR)/Build/Products/$(APP_CONFIG)/ yet."; \
	    echo "Run: make app"; \
	    exit 1; \
	fi
