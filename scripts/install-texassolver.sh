#!/usr/bin/env bash
# install-texassolver.sh — build the TexasSolver console binary on macOS.
#
# What this does:
#   1. Clones https://github.com/bupticybee/TexasSolver (console branch) into
#      ./vendor/TexasSolver/ — NOT committed, NOT linked against.
#   2. Builds the `console_solver` binary using cmake (assumes Xcode CLT + cmake
#      already installed).
#   3. Drops the final binary at ./bin/texassolver and the matching `resources/`
#      directory alongside it so runtime file lookups resolve.
#
# Idempotency:
#   Re-running is a no-op if ./bin/texassolver already exists and is executable.
#   Pass --force to rebuild from scratch.
#
# Why the binary lives separately:
#   TexasSolver is AGPL-3.0. We use the compiled binary as a black-box test
#   oracle — we do NOT link against its source and we do NOT ship it. See
#   docs/DIFFERENTIAL_TESTING.md for the legal justification.
#
# Usage:
#   ./scripts/install-texassolver.sh            # build if missing
#   ./scripts/install-texassolver.sh --force    # always rebuild
#   TS_REV=<sha> ./scripts/install-texassolver.sh   # pin a specific revision

set -euo pipefail

# Resolve repo root relative to this script so it works from any cwd.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VENDOR_DIR="$REPO_ROOT/vendor/TexasSolver"
BUILD_DIR="$VENDOR_DIR/build"
INSTALL_DIR="$VENDOR_DIR/install"
BIN_DIR="$REPO_ROOT/bin"
OUT_BIN="$BIN_DIR/texassolver"

# console branch is where `console_solver` lives. The master branch is the GUI.
TS_REPO="${TS_REPO:-https://github.com/bupticybee/TexasSolver.git}"
TS_BRANCH="${TS_BRANCH:-console}"
TS_REV="${TS_REV:-}"  # optional: pin a specific commit SHA

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

# --- Idempotency check --------------------------------------------------------
if [[ -x "$OUT_BIN" && "$FORCE" -eq 0 ]]; then
    echo "[install-texassolver] $OUT_BIN already present; nothing to do."
    echo "[install-texassolver] pass --force to rebuild."
    exit 0
fi

# --- Preflight ----------------------------------------------------------------
if [[ "$(uname)" != "Darwin" ]]; then
    echo "[install-texassolver] this script is macOS-only." >&2
    echo "[install-texassolver] for Colab/Linux use scripts/install-texassolver-colab.sh" >&2
    exit 1
fi

for tool in git cmake make clang++; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "[install-texassolver] missing required tool: $tool" >&2
        echo "[install-texassolver] install Xcode command-line tools + cmake:" >&2
        echo "    xcode-select --install" >&2
        echo "    brew install cmake" >&2
        exit 1
    fi
done

# Apple Silicon + recent macOS ship with Apple Clang which lacks OpenMP headers
# by default. TexasSolver requires OpenMP (-fopenmp in its CMakeLists). Prefer
# libomp from Homebrew if present; warn clearly if not.
BREW_PREFIX=""
if command -v brew >/dev/null 2>&1; then
    BREW_PREFIX="$(brew --prefix 2>/dev/null || true)"
fi

LIBOMP_PREFIX=""
if [[ -n "$BREW_PREFIX" ]]; then
    if [[ -d "$BREW_PREFIX/opt/libomp" ]]; then
        LIBOMP_PREFIX="$BREW_PREFIX/opt/libomp"
    fi
fi

if [[ -z "$LIBOMP_PREFIX" ]]; then
    echo "[install-texassolver] WARNING: libomp not found via Homebrew." >&2
    echo "[install-texassolver] Apple Clang does not bundle OpenMP; the build" >&2
    echo "[install-texassolver] may fail with '<omp.h> not found'." >&2
    echo "[install-texassolver] If so, run:  brew install libomp  and re-run." >&2
fi

# --- Clone / update -----------------------------------------------------------
mkdir -p "$REPO_ROOT/vendor" "$BIN_DIR"

if [[ ! -d "$VENDOR_DIR/.git" ]]; then
    echo "[install-texassolver] cloning $TS_REPO (branch $TS_BRANCH) -> $VENDOR_DIR"
    git clone --recursive --branch "$TS_BRANCH" --depth 1 \
        "$TS_REPO" "$VENDOR_DIR"
else
    echo "[install-texassolver] refreshing $VENDOR_DIR"
    (
        cd "$VENDOR_DIR"
        git fetch origin "$TS_BRANCH"
        git checkout "$TS_BRANCH"
        git pull --ff-only origin "$TS_BRANCH"
        git submodule update --init --recursive
    )
fi

if [[ -n "$TS_REV" ]]; then
    echo "[install-texassolver] pinning to revision $TS_REV"
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

CMAKE_ARGS=(
    -DCMAKE_BUILD_TYPE=Release
    -DCMAKE_INSTALL_PREFIX="$INSTALL_DIR"
    # TexasSolver vendors googletest with a CMakeLists declaring
    # cmake_minimum_required(VERSION 2.6) which modern CMake rejects.
    # The escape hatch is this policy floor flag. Harmless on modern CMake.
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5
)

# Wire up libomp if we found it. OpenMP flags for Apple Clang differ from gcc.
if [[ -n "$LIBOMP_PREFIX" ]]; then
    CMAKE_ARGS+=(
        -DOpenMP_C_FLAGS="-Xpreprocessor -fopenmp -I$LIBOMP_PREFIX/include"
        -DOpenMP_CXX_FLAGS="-Xpreprocessor -fopenmp -I$LIBOMP_PREFIX/include"
        -DOpenMP_C_LIB_NAMES=omp
        -DOpenMP_CXX_LIB_NAMES=omp
        -DOpenMP_omp_LIBRARY="$LIBOMP_PREFIX/lib/libomp.dylib"
    )
fi

echo "[install-texassolver] running cmake in $BUILD_DIR"
(
    cd "$BUILD_DIR"
    # Apple Clang doesn't auto-link libomp even when -fopenmp is on. We
    # inject it via LDFLAGS / CXXFLAGS rather than patching TexasSolver's
    # CMakeLists (keeping their source tree pristine — we don't modify it).
    if [[ -n "$LIBOMP_PREFIX" ]]; then
        export LDFLAGS="-L$LIBOMP_PREFIX/lib -lomp ${LDFLAGS:-}"
        export CXXFLAGS="-I$LIBOMP_PREFIX/include ${CXXFLAGS:-}"
        export CPPFLAGS="-I$LIBOMP_PREFIX/include ${CPPFLAGS:-}"
    fi
    cmake "${CMAKE_ARGS[@]}" ..
    # TexasSolver's CMakeLists uses `make install` as the canonical build path.
    make -j"$(sysctl -n hw.ncpu)" install
)

# --- Stage binary + resources -------------------------------------------------
# TexasSolver expects its `resources/` directory next to the binary at runtime
# (for hand-evaluator tables etc.). Copy both to bin/.
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
    echo "[install-texassolver] build succeeded but console_solver not found." >&2
    echo "[install-texassolver] looked in:" >&2
    echo "    $INSTALL_DIR/console_solver" >&2
    echo "    $VENDOR_DIR/install/console_solver" >&2
    echo "    $BUILD_DIR/console_solver" >&2
    exit 1
fi

# Prefer the install-tree resources dir if present, else the source-tree one.
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

if [[ -z "$RESOURCES_SRC" ]]; then
    echo "[install-texassolver] WARNING: no resources/ dir found beside binary." >&2
    echo "[install-texassolver] solver may fail at runtime loading hand tables." >&2
fi

cp "$CANDIDATE_BIN" "$OUT_BIN"
chmod +x "$OUT_BIN"

if [[ -n "$RESOURCES_SRC" ]]; then
    rm -rf "$BIN_DIR/resources"
    cp -R "$RESOURCES_SRC" "$BIN_DIR/resources"
fi

echo "[install-texassolver] installed:"
echo "    binary:    $OUT_BIN"
if [[ -d "$BIN_DIR/resources" ]]; then
    echo "    resources: $BIN_DIR/resources"
fi

echo "[install-texassolver] verifying --help output..."
if ! "$OUT_BIN" --help 2>&1 | head -20; then
    # Some TexasSolver builds exit 1 on --help; as long as the binary runs
    # and prints *something* we consider it usable.
    :
fi

echo "[install-texassolver] done."
