#!/usr/bin/env bash
#
# Regenerate the macOS AppIcon ladder from the 1024×1024 master PNG.
#
# Writes:
#   apple/Dub/Assets.xcassets/AppIcon.appiconset/icon_*.png
#   apple/Dub/Resources/AppIcon.icns
#
# actool flattens the asset catalog to opaque pixels (square Dock tile).
# We also ship a hand-built .icns from the rounded-rect PNGs (alpha preserved)
# and overwrite actool's thin copy at build time so the Dock gets rounded
# corners even for ad-hoc local builds.
#
# Re-run after editing render-macos-icon.swift. Safe to run any time.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

ICONSET_DIR="${REPO_ROOT}/apple/Dub/Assets.xcassets/AppIcon.appiconset"
RESOURCES_DIR="${REPO_ROOT}/apple/Dub/Resources"
MASTER="${ICONSET_DIR}/AppIcon-1024.png"
ICNS_OUT="${RESOURCES_DIR}/AppIcon.icns"

require_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "error: '$1' not found in PATH (need Xcode CLI tools)" >&2
        exit 1
    fi
}

require_tool sips
require_tool iconutil
require_tool swift

mkdir -p "${RESOURCES_DIR}"

echo "==> macOS rounded-rect icon master"
swift "${SCRIPT_DIR}/render-macos-icon.swift" "${MASTER}"

resize() {
    local px="$1"
    local out="$2"
    sips -z "${px}" "${px}" "${MASTER}" --out "${out}" >/dev/null
}

echo "==> AppIcon PNG ladder -> ${ICONSET_DIR}"
resize 16  "${ICONSET_DIR}/icon_16x16.png"
resize 32  "${ICONSET_DIR}/icon_16x16@2x.png"
resize 32  "${ICONSET_DIR}/icon_32x32.png"
resize 64  "${ICONSET_DIR}/icon_32x32@2x.png"
resize 128 "${ICONSET_DIR}/icon_128x128.png"
resize 256 "${ICONSET_DIR}/icon_128x128@2x.png"
resize 256 "${ICONSET_DIR}/icon_256x256.png"
resize 512 "${ICONSET_DIR}/icon_256x256@2x.png"
resize 512 "${ICONSET_DIR}/icon_512x512.png"

TMP_ICONSET="$(mktemp -d "${TMPDIR:-/tmp}/dub-appicon.XXXXXX.iconset")"
trap 'rm -rf "${TMP_ICONSET}"' EXIT

cp "${ICONSET_DIR}/icon_16x16.png"       "${TMP_ICONSET}/"
cp "${ICONSET_DIR}/icon_16x16@2x.png"   "${TMP_ICONSET}/"
cp "${ICONSET_DIR}/icon_32x32.png"      "${TMP_ICONSET}/"
cp "${ICONSET_DIR}/icon_32x32@2x.png"   "${TMP_ICONSET}/"
cp "${ICONSET_DIR}/icon_128x128.png"    "${TMP_ICONSET}/"
cp "${ICONSET_DIR}/icon_128x128@2x.png" "${TMP_ICONSET}/"
cp "${ICONSET_DIR}/icon_256x256.png"    "${TMP_ICONSET}/"
cp "${ICONSET_DIR}/icon_256x256@2x.png" "${TMP_ICONSET}/"
cp "${ICONSET_DIR}/icon_512x512.png"    "${TMP_ICONSET}/"
cp "${MASTER}"                          "${TMP_ICONSET}/icon_512x512@2x.png"

echo "==> AppIcon.icns (rounded-rect alpha) -> ${ICNS_OUT}"
iconutil -c icns "${TMP_ICONSET}" -o "${ICNS_OUT}"

BYTES="$(wc -c < "${ICNS_OUT}" | tr -d ' ')"
echo "    wrote ${BYTES} bytes"
