#!/usr/bin/env bash
# install-texassolver-colab.sh — build the TexasSolver console binary on
# a Colab-style Linux VM (Ubuntu / Debian, amd64).
#
# The macOS script (install-texassolver.sh) and this one share the same goal
# but differ in:
#   - package manager (apt vs brew)
#   - OpenMP wiring (gcc ships OpenMP; Apple Clang does not)
#   - assume-yes defaults (Colab is non-interactive)
#
# Usage:
#   ./scripts/install-texassolver-colab.sh            # build if missing
#   ./scripts/install-texassolver-colab.sh --force    # rebuild
#   TS_REV=<sha> ./scripts/install-texassolver-colab.sh   # pin revision
#
# Legal note: the TexasSolver source is AGPL-3.0. We use the compiled binary
# as a black-box test oracle on nightly validation runs. We do not link
# against its source; we do not ship its binary to end users. See
# docs/DIFFERENTIAL_TESTING.md for the full legal reasoning.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VENDOR_DIR="$REPO_ROOT/vendor/TexasSolver"
BUILD_DIR="$VENDOR_DIR/build"
INSTALL_DIR="$VENDOR_DIR/install"
BIN_DIR="$REPO_ROOT/bin"
OUT_BIN="$BIN_DIR/texassolver"

TS_REPO="${TS_REPO:-https://github.com/bupticybee/TexasSolver.git}"
TS_BRANCH="${TS_BRANCH:-console}"
TS_REV="${TS_REV:-}"

FORCE=0
for arg in "$@"; do
    case "$arg" in
        --force) FORCE=1 ;;
        -h|--help)
            sed -n '2,30p' "$0"
            exit 0
            ;;
        *) echo "unknown argument: $arg" >&2; exit 2 ;;
    esac
done

# Colab root-less vs rootful: `apt-get` works as root in Colab by default.
# If we're non-root and apt is missing, fail fast with a helpful message.
SUDO=""
if [[ "$EUID" -ne 0 ]]; then
    if command -v sudo >/dev/null 2>&1; then
        SUDO="sudo"
    fi
fi

# --- Idempotency --------------------------------------------------------------
if [[ -x "$OUT_BIN" && "$FORCE" -eq 0 ]]; then
    echo "[install-texassolver-colab] $OUT_BIN already present; nothing to do."
    echo "[install-texassolver-colab] pass --force to rebuild."
    exit 0
fi

# --- Preflight ----------------------------------------------------------------
if [[ "$(uname)" != "Linux" ]]; then
    echo "[install-texassolver-colab] this script is Linux-only." >&2
    echo "[install-texassolver-colab] on macOS run scripts/install-texassolver.sh" >&2
    exit 1
fi

# Install build deps via apt. Colab ships most but not all — be explicit.
if command -v apt-get >/dev/null 2>&1; then
    echo "[install-texassolver-colab] installing build dependencies via apt"
    DEBIAN_FRONTEND=noninteractive $SUDO apt-get update -y
    DEBIAN_FRONTEND=noninteractive $SUDO apt-get install -y \
        build-essential \
        cmake \
        git \
        libomp-dev \
        g++ \
        make
else
    echo "[install-texassolver-colab] WARNING: apt-get not found. Assuming" >&2
    echo "[install-texassolver-colab] build-essential, cmake, git, g++, libomp-dev" >&2
    echo "[install-texassolver-colab] are already installed." >&2
fi

for tool in git cmake make g++; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "[install-texassolver-colab] missing tool: $tool" >&2
        exit 1
    fi
done

mkdir -p "$REPO_ROOT/vendor" "$BIN_DIR"

# --- Clone / update -----------------------------------------------------------
if [[ ! -d "$VENDOR_DIR/.git" ]]; then
    echo "[install-texassolver-colab] cloning $TS_REPO ($TS_BRANCH)"
    git clone --recursive --branch "$TS_BRANCH" --depth 1 \
        "$TS_REPO" "$VENDOR_DIR"
else
    echo "[install-texassolver-colab] refreshing $VENDOR_DIR"
    (
        cd "$VENDOR_DIR"
        git fetch origin "$TS_BRANCH"
        git checkout "$TS_BRANCH"
        git pull --ff-only origin "$TS_BRANCH"
        git submodule update --init --recursive
    )
fi

if [[ -n "$TS_REV" ]]; then
    (
        cd "$VENDOR_DIR"
        git fetch origin "$TS_REV" || true
        git checkout "$TS_REV"
        git submodule update --init --recursive
    )
fi

# --- Build --------------------------------------------------------------------
rm -rf "$BUILD_DIR" "$INSTALL_DIR"
mkdir -p "$BUILD_DIR"

echo "[install-texassolver-colab] running cmake"
(
    cd "$BUILD_DIR"
    cmake \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$INSTALL_DIR" \
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
        ..
    make -j"$(nproc)" install
)

# --- Stage --------------------------------------------------------------------
CANDIDATE_BIN=""
for candidate in \
    "$INSTALL_DIR/console_solver" \
    "$VENDOR_DIR/install/console_solver" \
    "$BUILD_DIR/console_solver" \
; do
    if [[ -x "$candidate" ]]; then
        CANDIDATE_BIN="$candidate"
        break
    fi
done

if [[ -z "$CANDIDATE_BIN" ]]; then
    echo "[install-texassolver-colab] build finished but console_solver missing." >&2
    exit 1
fi

RESOURCES_SRC=""
for candidate in \
    "$INSTALL_DIR/resources" \
    "$VENDOR_DIR/resources" \
; do
    if [[ -d "$candidate" ]]; then
        RESOURCES_SRC="$candidate"
        break
    fi
done

cp "$CANDIDATE_BIN" "$OUT_BIN"
chmod +x "$OUT_BIN"

if [[ -n "$RESOURCES_SRC" ]]; then
    rm -rf "$BIN_DIR/resources"
    cp -R "$RESOURCES_SRC" "$BIN_DIR/resources"
fi

echo "[install-texassolver-colab] installed $OUT_BIN"

# Smoke test — many TexasSolver builds exit non-zero on --help; treat output
# presence, not exit code, as the signal.
"$OUT_BIN" --help 2>&1 | head -10 || true

echo "[install-texassolver-colab] done."
