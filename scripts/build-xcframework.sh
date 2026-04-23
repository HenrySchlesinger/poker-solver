#!/usr/bin/env bash
#
# build-xcframework.sh — wrap the build-release.sh output into an .xcframework
# tarball that SwiftPM's `.binaryTarget(url:)` can actually consume.
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
#   PokerSolver-<VERSION>.xcframework.tar.gz     # tarball for SPM binaryTarget
#   PokerSolver-<VERSION>.xcframework.tar.gz.sha256
#
# Why this script exists:
#   A28's build-release.sh ships a raw .a/.dylib + header. SwiftPM's
#   .binaryTarget(url:) refuses anything other than a .xcframework.zip or
#   .xcframework.tar.gz. So this script takes A28's bundle and wraps it
#   into the xcframework shape xcodebuild expects, then tarballs that.
#
#   `xcodebuild -create-xcframework` builds the directory; we tarball it
#   with `tar czf` after because xcodebuild doesn't have a tarball mode.

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
need tar
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

# --- tarball ------------------------------------------------------------------
TARBALL_NAME="PokerSolver-$VERSION.xcframework.tar.gz"
TARBALL_PATH="$BUNDLE_PARENT/$TARBALL_NAME"
SHA_PATH="$TARBALL_PATH.sha256"

rm -f "$TARBALL_PATH" "$SHA_PATH"

echo "--- packaging tarball ---"
# tar from inside release-bundle so the archive stores a relative path
# (PokerSolver.xcframework/...) rather than an absolute one.
( cd "$BUNDLE_PARENT" && tar czf "$TARBALL_NAME" "PokerSolver.xcframework" )

# SwiftPM's `.binaryTarget(url:)` checksum is `swift package compute-checksum`.
# That algorithm is literally sha256-of-the-file, identical to `shasum -a 256`,
# so we emit that here. The Package.swift consumer will paste this value.
( cd "$BUNDLE_PARENT" && shasum -a 256 "$TARBALL_NAME" > "$TARBALL_NAME.sha256" )

TARBALL_SIZE_BYTES="$(wc -c <"$TARBALL_PATH" | tr -d ' ')"
TARBALL_SIZE_MB="$(( (TARBALL_SIZE_BYTES + 1024 * 1024 - 1) / (1024 * 1024) ))"
CHECKSUM="$(awk '{print $1}' "$SHA_PATH")"

echo
echo "=== xcframework ready ==="
echo "framework: $XCF_DIR"
echo "tarball:   $TARBALL_PATH"
echo "size:      ${TARBALL_SIZE_BYTES} bytes (~${TARBALL_SIZE_MB} MiB)"
echo "sha256:    $CHECKSUM"
echo
echo "next: update crates/solver-ffi/Package.swift with checksum $CHECKSUM,"
echo "      then: scripts/gh-release.sh $VERSION"
