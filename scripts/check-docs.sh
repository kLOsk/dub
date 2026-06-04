#!/usr/bin/env bash
#
# check-docs.sh — fail when human-facing docs drift from the
# source-of-truth constants in the code.
#
# The README milestone note and the docs/html dashboard quote three
# numbers that live in code: the FFI contract version, the SQLite
# schema version, and the crate count. Nothing else keeps them in
# sync, so this guard greps the code for the canonical value and
# fails the build if a doc still shows the old one. Wire into
# `make docs-check` and CI.
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

# docs/html dashboard quotes schema version + crate count.
require_match "index.html schema version" \
    docs/html/index.html -F "Schema v${SCHEMA_VERSION}"
require_match "index.html crate count (header)" \
    docs/html/index.html -F "${CRATE_COUNT} Rust crates"
require_match "index.html crate count (architecture card)" \
    docs/html/index.html -F "${CRATE_COUNT} crates"

# LIBRARY-SCHEMA.md states the current version in prose + version history.
require_match "LIBRARY-SCHEMA.md current-version prose" \
    docs/LIBRARY-SCHEMA.md -F "current applied version is **${SCHEMA_VERSION}**"
require_match "LIBRARY-SCHEMA.md version-history row" \
    docs/LIBRARY-SCHEMA.md -E "^\| ${SCHEMA_VERSION} +\|"

echo ""
if [ "$FAILURES" -ne 0 ]; then
    echo "docs-check: ${FAILURES} drift(s) found. Update the doc(s) above to match the code, then re-run."
    exit 1
fi

echo "docs-check: docs are in sync with code."
