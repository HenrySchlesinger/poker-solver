#!/usr/bin/env bash
#
# build-release.sh — build a universal macOS release bundle of solver-ffi.
#
# Usage:
#   scripts/build-release.sh                 # infer version from `git describe`
#   scripts/build-release.sh v0.1.0          # force a specific version string
#   scripts/build-release.sh v0.1.0-test     # dry-run / pre-release name
#
# Produces, under `target/release-bundle/solver-<VERSION>/`:
#   lib/libsolver_ffi.a          universal staticlib (arm64 + x86_64)
#   lib/libsolver_ffi.dylib      universal cdylib    (arm64 + x86_64)
#   include/solver.h             cbindgen-generated C header
#   VERSION                      human-readable build metadata
#   CHECKSUMS.sha256             sha256 of every artifact in the bundle
#
# And alongside it:
#   target/release-bundle/solver-<VERSION>-macos-universal.tar.gz
#   target/release-bundle/solver-<VERSION>-macos-universal.tar.gz.sha256
#
# Prereqs (see docs/RELEASE_PROCESS.md):
#   rustup target add aarch64-apple-darwin
#   rustup target add x86_64-apple-darwin
#   macOS with `lipo` (ships with Xcode command line tools).

set -euo pipefail

# --- repo root (resolve even if invoked from elsewhere) -----------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# --- version resolution -------------------------------------------------------
if [[ $# -ge 1 && -n "${1:-}" ]]; then
    VERSION="$1"
else
    VERSION="$(git describe --tags --abbrev=0 2>/dev/null || echo "dev")"
fi

GIT_SHA="$(git rev-parse HEAD 2>/dev/null || echo "unknown")"
GIT_SHA_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")"
BUILD_TIMESTAMP="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
BUILD_HOST="$(uname -sm)"

echo "=== poker-solver release build ==="
echo "version:    $VERSION"
echo "git sha:    $GIT_SHA_SHORT ($GIT_SHA)"
echo "timestamp:  $BUILD_TIMESTAMP"
echo "host:       $BUILD_HOST"
echo

# --- preflight: required tools & targets -------------------------------------
need() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "error: required tool '$1' not found in PATH" >&2
        exit 2
    fi
}
need cargo
need lipo
need tar
need shasum

# Rustup targets must be installed; fail loudly if not.
if command -v rustup >/dev/null 2>&1; then
    INSTALLED_TARGETS="$(rustup target list --installed)"
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
        if ! echo "$INSTALLED_TARGETS" | grep -qx "$t"; then
            echo "error: rustup target '$t' not installed" >&2
            echo "       run: rustup target add $t" >&2
            exit 2
        fi
    done
else
    echo "warning: rustup not in PATH; assuming both darwin targets are available" >&2
fi

# --- output layout ------------------------------------------------------------
BUNDLE_PARENT="target/release-bundle"
BUNDLE_NAME="solver-$VERSION"
OUT_DIR="$BUNDLE_PARENT/$BUNDLE_NAME"
TARBALL_BASENAME="$BUNDLE_NAME-macos-universal.tar.gz"

# Start clean so stale files can't leak into the tarball.
rm -rf "$OUT_DIR" "$BUNDLE_PARENT/$TARBALL_BASENAME" "$BUNDLE_PARENT/$TARBALL_BASENAME.sha256"
mkdir -p "$OUT_DIR/lib" "$OUT_DIR/include"

# --- build both architectures -------------------------------------------------
echo "--- building aarch64-apple-darwin ---"
cargo build --release --target aarch64-apple-darwin -p solver-ffi
echo "--- building x86_64-apple-darwin ---"
cargo build --release --target x86_64-apple-darwin -p solver-ffi

ARM_STATIC="target/aarch64-apple-darwin/release/libsolver_ffi.a"
X86_STATIC="target/x86_64-apple-darwin/release/libsolver_ffi.a"
ARM_DYLIB="target/aarch64-apple-darwin/release/libsolver_ffi.dylib"
X86_DYLIB="target/x86_64-apple-darwin/release/libsolver_ffi.dylib"

for f in "$ARM_STATIC" "$X86_STATIC" "$ARM_DYLIB" "$X86_DYLIB"; do
    if [[ ! -f "$f" ]]; then
        echo "error: expected build artifact missing: $f" >&2
        exit 3
    fi
done

# --- lipo: glue arm64 + x86_64 into fat binaries ------------------------------
echo "--- lipo: libsolver_ffi.a ---"
lipo -create "$ARM_STATIC" "$X86_STATIC" -output "$OUT_DIR/lib/libsolver_ffi.a"

echo "--- lipo: libsolver_ffi.dylib ---"
lipo -create "$ARM_DYLIB" "$X86_DYLIB" -output "$OUT_DIR/lib/libsolver_ffi.dylib"

# Sanity: the resulting libs must report both slices.
verify_universal() {
    local f="$1"
    local info
    info="$(lipo -info "$f" 2>&1)"
    if ! echo "$info" | grep -q "arm64" || ! echo "$info" | grep -q "x86_64"; then
        echo "error: $f is not universal — lipo -info reports:" >&2
        echo "  $info" >&2
        exit 4
    fi
    echo "  ok: $f ($info)"
}
verify_universal "$OUT_DIR/lib/libsolver_ffi.a"
verify_universal "$OUT_DIR/lib/libsolver_ffi.dylib"

# --- header -------------------------------------------------------------------
HEADER_SRC="crates/solver-ffi/include/solver.h"
if [[ ! -f "$HEADER_SRC" ]]; then
    echo "error: header not found at $HEADER_SRC" >&2
    echo "       run 'cargo build -p solver-ffi' first to regenerate it" >&2
    exit 5
fi
cp "$HEADER_SRC" "$OUT_DIR/include/solver.h"

# --- VERSION metadata ---------------------------------------------------------
# Plain-text, greppable, easy to cat in a shell. Intentionally not JSON —
# the tarball consumer is a Swift app that already has its own manifest.
cat >"$OUT_DIR/VERSION" <<EOF
name:       poker-solver
version:    $VERSION
git_sha:    $GIT_SHA
git_short:  $GIT_SHA_SHORT
built_at:   $BUILD_TIMESTAMP
built_on:   $BUILD_HOST
targets:    aarch64-apple-darwin, x86_64-apple-darwin
artifacts:  lib/libsolver_ffi.a, lib/libsolver_ffi.dylib, include/solver.h
EOF

# --- CHECKSUMS inside the bundle ---------------------------------------------
# Paths are bundle-relative so consumers can verify after extraction.
( cd "$OUT_DIR" && \
  shasum -a 256 \
    lib/libsolver_ffi.a \
    lib/libsolver_ffi.dylib \
    include/solver.h \
    VERSION \
    > CHECKSUMS.sha256 )

echo
echo "--- bundle contents ($OUT_DIR) ---"
( cd "$OUT_DIR" && find . -type f | sort | sed 's|^\./|  |' )
echo
echo "--- CHECKSUMS.sha256 ---"
sed 's/^/  /' "$OUT_DIR/CHECKSUMS.sha256"
echo

# --- tarball ------------------------------------------------------------------
echo "--- packaging tarball ---"
( cd "$BUNDLE_PARENT" && tar czf "$TARBALL_BASENAME" "$BUNDLE_NAME" )

# sha256 of the tarball itself — this is what goes into Package.swift
# `checksum:` once we cut the GitHub Release.
( cd "$BUNDLE_PARENT" && shasum -a 256 "$TARBALL_BASENAME" > "$TARBALL_BASENAME.sha256" )

TARBALL_PATH="$BUNDLE_PARENT/$TARBALL_BASENAME"
TARBALL_SIZE_BYTES="$(wc -c <"$TARBALL_PATH" | tr -d ' ')"
TARBALL_SIZE_MB="$(( (TARBALL_SIZE_BYTES + 1024 * 1024 - 1) / (1024 * 1024) ))"

echo
echo "=== release bundle ready ==="
echo "tarball:   $TARBALL_PATH"
echo "size:      ${TARBALL_SIZE_BYTES} bytes (~${TARBALL_SIZE_MB} MiB)"
echo "sha256:    $(cat "$TARBALL_PATH.sha256")"
echo
echo "next step: scripts/gh-release.sh $VERSION"
