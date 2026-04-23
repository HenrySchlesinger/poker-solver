#!/usr/bin/env bash
#
# build-xcframework.sh — wrap the build-release.sh output into an .xcframework
# zip that SwiftPM's `.binaryTarget(url:)` can actually consume.
#
# Usage:
#   scripts/build-xcframework.sh                 # infer version from git describe
#   scripts/build-xcframework.sh v0.1.0          # explicit version
#   scripts/build-xcframework.sh v0.1.0-test     # dry-run / pre-release name
#
# Preconditions:
#   - scripts/build-release.sh <VERSION> has already run and produced
#     target/release-bundle/solver-<VERSION>/ with lib/ and include/.
#   - Xcode command line tools are installed (xcodebuild, lipo).
#
# Produces, under `target/release-bundle/`:
#   PokerSolver.xcframework/                     # the framework dir itself
#     Info.plist                                 # Apple xcframework manifest
#     macos-arm64_x86_64/                        # universal slice
#       libsolver_ffi.a                          # universal static lib
#       Headers/solver.h                         # cbindgen'd C header
#   PokerSolver-<VERSION>.xcframework.zip        # zip for SPM binaryTarget
#   PokerSolver-<VERSION>.xcframework.zip.sha256
#
# Why this script exists:
#   A28's build-release.sh ships a raw .a/.dylib + header. SwiftPM's
#   .binaryTarget(url:) accepts only .zip archives of .xcframework
#   directories (`swift package dump-package` errors on `.tar.gz` with
#   "unsupported extension"). So this script takes A28's bundle and
#   wraps it into the xcframework shape xcodebuild expects, then zips it.
#
#   `xcodebuild -create-xcframework` builds the directory; we zip it
#   with `ditto -c -k --keepParent` afterward — that's Apple's
#   recommended tool for notarization-compatible zip archives that
#   preserve symlinks and resource forks. SwiftPM consumes zips
#   produced by `ditto` interchangeably with `zip`.

set -euo pipefail

# --- repo root ----------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# --- version resolution -------------------------------------------------------
if [[ $# -ge 1 && -n "${1:-}" ]]; then
    VERSION="$1"
else
    VERSION="$(git describe --tags --abbrev=0 2>/dev/null || echo "dev")"
fi

echo "=== poker-solver xcframework build ==="
echo "version: $VERSION"
echo

# --- preflight: required tools -----------------------------------------------
need() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "error: required tool '$1' not found in PATH" >&2
        exit 2
    fi
}
need xcodebuild
need lipo
need ditto
need shasum

# --- preflight: input bundle must exist --------------------------------------
BUNDLE_PARENT="target/release-bundle"
BUNDLE_NAME="solver-$VERSION"
IN_DIR="$BUNDLE_PARENT/$BUNDLE_NAME"

if [[ ! -d "$IN_DIR" ]]; then
    echo "error: release bundle not found at $IN_DIR" >&2
    echo "       run: scripts/build-release.sh $VERSION" >&2
    exit 3
fi

STATIC_LIB="$IN_DIR/lib/libsolver_ffi.a"
HEADER_DIR="$IN_DIR/include"
HEADER_FILE="$HEADER_DIR/solver.h"

for f in "$STATIC_LIB" "$HEADER_FILE"; do
    if [[ ! -f "$f" ]]; then
        echo "error: expected bundle artifact missing: $f" >&2
        echo "       re-run: scripts/build-release.sh $VERSION" >&2
        exit 3
    fi
done

# Sanity: confirm the static lib really is universal before wrapping it.
# A single-arch .xcframework is still valid but silently breaks Intel
# Rosetta users, so fail loud here.
LIPO_INFO="$(lipo -info "$STATIC_LIB" 2>&1)"
if ! echo "$LIPO_INFO" | grep -q "arm64" || ! echo "$LIPO_INFO" | grep -q "x86_64"; then
    echo "error: $STATIC_LIB is not universal (arm64 + x86_64)" >&2
    echo "       lipo -info: $LIPO_INFO" >&2
    exit 4
fi
echo "input: $STATIC_LIB"
echo "  $LIPO_INFO"
echo

# --- build the .xcframework ---------------------------------------------------
XCF_DIR="$BUNDLE_PARENT/PokerSolver.xcframework"
rm -rf "$XCF_DIR"

# xcodebuild -create-xcframework requires the headers dir to contain only
# headers we want to expose. Our include/ already matches that shape.
echo "--- xcodebuild -create-xcframework ---"
xcodebuild -create-xcframework \
    -library "$STATIC_LIB" \
    -headers "$HEADER_DIR" \
    -output "$XCF_DIR"

# Sanity: the resulting xcframework must have an Info.plist referencing
# the universal slice (macos-arm64_x86_64).
PLIST="$XCF_DIR/Info.plist"
if [[ ! -f "$PLIST" ]]; then
    echo "error: xcframework missing Info.plist at $PLIST" >&2
    exit 5
fi
if ! grep -q "macos-arm64_x86_64" "$PLIST"; then
    echo "error: Info.plist does not reference macos-arm64_x86_64 slice" >&2
    echo "       contents:" >&2
    sed 's/^/  /' "$PLIST" >&2
    exit 5
fi

echo
echo "--- xcframework contents ($XCF_DIR) ---"
( cd "$XCF_DIR" && find . -type f | sort | sed 's|^\./|  |' )
echo

# --- zip ---------------------------------------------------------------------
ZIP_NAME="PokerSolver-$VERSION.xcframework.zip"
ZIP_PATH="$BUNDLE_PARENT/$ZIP_NAME"
SHA_PATH="$ZIP_PATH.sha256"

rm -f "$ZIP_PATH" "$SHA_PATH"

echo "--- packaging zip ---"
# ditto from inside release-bundle so the archive stores a relative path
# (PokerSolver.xcframework/...) rather than an absolute one.
# --keepParent preserves PokerSolver.xcframework as the top-level dir.
( cd "$BUNDLE_PARENT" && ditto -c -k --keepParent "PokerSolver.xcframework" "$ZIP_NAME" )

# SwiftPM's `.binaryTarget(url:)` checksum for a remote xcframework zip
# is NOT a sha256-of-the-file — it's the sha256 computed by
# `swift package compute-checksum <zip>`. Empirically for a regular zip
# these match sha256(file), but we emit both for clarity and compute the
# authoritative value via `swift package compute-checksum`.
( cd "$BUNDLE_PARENT" && shasum -a 256 "$ZIP_NAME" > "$ZIP_NAME.sha256" )

# Best effort: if `swift` is installed, compute the canonical SPM checksum
# and print it. This is what goes into Package.swift.
SPM_CHECKSUM=""
if command -v swift >/dev/null 2>&1; then
    SPM_CHECKSUM="$(swift package compute-checksum "$ZIP_PATH" 2>/dev/null || true)"
fi

ZIP_SIZE_BYTES="$(wc -c <"$ZIP_PATH" | tr -d ' ')"
ZIP_SIZE_MB="$(( (ZIP_SIZE_BYTES + 1024 * 1024 - 1) / (1024 * 1024) ))"
FILE_SHA="$(awk '{print $1}' "$SHA_PATH")"

echo
echo "=== xcframework ready ==="
echo "framework:   $XCF_DIR"
echo "zip:         $ZIP_PATH"
echo "size:        ${ZIP_SIZE_BYTES} bytes (~${ZIP_SIZE_MB} MiB)"
echo "file sha256: $FILE_SHA"
if [[ -n "$SPM_CHECKSUM" ]]; then
    echo "spm check:   $SPM_CHECKSUM"
    CHECKSUM="$SPM_CHECKSUM"
else
    echo "spm check:   (swift not in PATH; use file sha256)"
    CHECKSUM="$FILE_SHA"
fi
echo
echo "next: update crates/solver-ffi/Package.swift with checksum $CHECKSUM,"
echo "      then: scripts/gh-release.sh $VERSION"
