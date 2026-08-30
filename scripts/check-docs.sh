#!/usr/bin/env bash
#
# check-docs.sh — fail when human-facing docs drift from the
# source-of-truth constants in the code.
#
# Docs quote numbers that live in code — the FFI contract version, the
# SQLite schema version, the crate count, and the vinyl-rip gate
# constants. Nothing else keeps them in sync, so this guard greps the
# code for the canonical value and fails the build if a doc still shows
# the old one. It also checks that every relative link between docs
# resolves, which is what rots after a folder reorg.
#
# The rule this encodes: a doc statement that *can* be checked mechanically
# should be, because the ones that were checked stayed right for months
# while the ones that were not drifted ten milestones.
#
# Wire into `make docs-check` and CI.
#
# This is intentionally narrow: it checks the few numbers that have
# actually rotted in the past, not prose. Add a check here whenever a
# new "magic number" starts appearing in both code and docs.

set -euo pipefail

cd "$(dirname "$0")/.."

FAILURES=0

fail() {
    echo "  FAIL: $1"
    FAILURES=$((FAILURES + 1))
}

ok() {
    echo "  ok:   $1"
}

# Extract a `pub const NAME: TYPE = N;` integer literal from a file.
extract_const() {
    local name="$1" file="$2"
    grep -Eo "pub const ${name}: u32 = [0-9]+" "$file" \
        | grep -Eo '[0-9]+$' \
        | head -n1
}

# require_match <description> <file> <grep-flags...> <pattern>
# Fails when the pattern is absent from the file.
require_match() {
    local desc="$1"; shift
    local file="$1"; shift
    if grep "$@" -- "$file" >/dev/null 2>&1; then
        ok "$desc"
    else
        fail "$desc — expected pattern not found in $file"
    fi
}

echo "Source-of-truth constants:"

FFI_VERSION="$(extract_const FFI_VERSION crates/dub-ffi/src/lib.rs)"
SCHEMA_VERSION="$(extract_const SCHEMA_VERSION crates/dub-library/src/schema.rs)"
CRATE_COUNT="$(find crates -mindepth 2 -maxdepth 2 -name Cargo.toml | wc -l | tr -d ' ')"

if [ -z "$FFI_VERSION" ]; then fail "could not read FFI_VERSION from crates/dub-ffi/src/lib.rs"; fi
if [ -z "$SCHEMA_VERSION" ]; then fail "could not read SCHEMA_VERSION from crates/dub-library/src/schema.rs"; fi
if [ -z "$CRATE_COUNT" ] || [ "$CRATE_COUNT" = "0" ]; then fail "could not count crates/*/Cargo.toml"; fi

echo "  FFI_VERSION    = ${FFI_VERSION}"
echo "  SCHEMA_VERSION = ${SCHEMA_VERSION}"
echo "  crate count    = ${CRATE_COUNT}"
echo ""
echo "Doc checks:"

# README milestone note quotes the FFI version.
require_match "README.md FFI version" \
    README.md -E "FFI.*\*\*${FFI_VERSION}\*\*"

# LIBRARY-SCHEMA.md states the current version in prose + version history.
require_match "LIBRARY-SCHEMA.md current-version prose" \
    docs/spec/LIBRARY-SCHEMA.md -F "current applied version is **${SCHEMA_VERSION}**"
require_match "LIBRARY-SCHEMA.md version-history row" \
    docs/spec/LIBRARY-SCHEMA.md -E "^\| ${SCHEMA_VERSION} +\|"

# ---------------------------------------------------------------------
# Vinyl-rip gate constants (PRD §5.2.7 quotes them; dub-rip owns them).
#
# These were refitted twice against real records and the prose lagged
# both times — a live example of the class of drift this script exists
# to stop.
# ---------------------------------------------------------------------
extract_f32() {
    local name="$1" file="$2"
    grep -Eo "${name}: [0-9]+\.[0-9]+" "$file" | grep -Eo '[0-9]+\.[0-9]+' | head -n1
}

GAPS=crates/dub-rip/src/gaps.rs
SESSION=crates/dub-rip/src/session.rs
MARGIN_DB="$(extract_f32 margin_db "$GAPS")"
CONTRAST_DB="$(extract_f32 min_contrast_db "$GAPS")"
LEADIN_DB="$(extract_f32 lead_in_margin_db "$GAPS")"
# Read from the `Default` impl specifically — `AutoCapture::manual()`
# carries different values and appears first in the file.
autocapture_default() {
    awk '/impl Default for AutoCapture/,/^}/' "$SESSION" \
        | grep -Eo "$1: (Some[(])?[0-9]+[.][0-9]+" \
        | grep -Eo '[0-9]+[.][0-9]+' | head -n1
}
STOP_SECS="$(autocapture_default silence_stop_secs)"
STOP_DB="$(autocapture_default silence_drop_db)"

echo ""
echo "Vinyl-rip gates (PRD §5.2.7 must quote these):"
echo "  margin_db          = ${MARGIN_DB}"
echo "  min_contrast_db    = ${CONTRAST_DB}"
echo "  lead_in_margin_db  = ${LEADIN_DB}"
echo "  auto-stop          = ${STOP_SECS} s at ${STOP_DB} dB"
echo ""
require_match "PRD gap line (floor + margin_db)" \
    docs/spec/PRD.md -F "floor + ${MARGIN_DB%.0} dB"
require_match "PRD contrast refusal (min_contrast_db)" \
    docs/spec/PRD.md -F "under ${CONTRAST_DB%.0} dB above its own floor"
require_match "PRD auto-stop (secs at dB)" \
    docs/spec/PRD.md -F "${STOP_SECS%.0} s at ${STOP_DB%.0} dB down"

# ---------------------------------------------------------------------
# Relative links between docs must resolve. Catches the reorg rot that
# leaves a doc pointing at where a file used to live.
# ---------------------------------------------------------------------
echo ""
echo "Doc cross-links:"
BROKEN_LINKS=0
# Ours only — vendored SPM checkouts under apple/build carry their own
# broken links and are none of our business.
for md in $(find . -name '*.md' \
        -not -path './target/*' -not -path './.git/*' \
        -not -path './apple/build/*' -not -path './apple/DerivedData/*' | sort); do
    dir="$(dirname "$md")"
    targets="$(grep -Eo '\]\([^)#]+\.md' "$md" 2>/dev/null | sed -E 's/^\]\(//' || true)"
    for target in $targets; do
        case "$target" in
            http*) continue ;;
        esac
        if [ ! -e "$dir/$target" ]; then
            fail "$md -> $target (no such file)"
            BROKEN_LINKS=$((BROKEN_LINKS + 1))
        fi
    done
done
if [ "$BROKEN_LINKS" -eq 0 ]; then
    ok "every relative .md link resolves"
fi

echo ""
if [ "$FAILURES" -ne 0 ]; then
    echo "docs-check: ${FAILURES} drift(s) found. Update the doc(s) above to match the code, then re-run."
    exit 1
fi

echo "docs-check: docs are in sync with code."
