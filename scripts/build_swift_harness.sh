#!/usr/bin/env bash
#
# build_swift_harness.sh — compile the A27 end-to-end Swift harness
# against the release libsolver_ffi.a produced by cargo.
#
# Two binaries get built:
#
#   target/swift-harness          ← from examples/swift-harness/main.swift
#                                   (A13's ABI smoke test; any status OK)
#   target/swift-harness-e2e      ← from examples/swift-harness/main_e2e.swift
#                                   (A27's outcome test; exits non-zero
#                                   if solver_solve doesn't return Ok)
#
# The outer driver scripts/e2e.sh runs only the e2e binary, but the
# smoke binary is rebuilt in the same invocation so the two don't drift.
#
# Preconditions:
#   - swiftc on PATH (macOS has this via Xcode command-line tools)
#   - `cargo build --release -p solver-ffi` has produced
#     target/release/libsolver_ffi.a
#
# Exit status: 0 on successful build, non-zero on compile/link failure.
# Does NOT run the binaries — that's scripts/e2e.sh's job.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if ! command -v swiftc >/dev/null; then
    echo "build_swift_harness.sh: swiftc not found on PATH" >&2
    echo "  (install Xcode command-line tools: xcode-select --install)" >&2
    exit 2
fi

STATICLIB="target/release/libsolver_ffi.a"
if [[ ! -f "$STATICLIB" ]]; then
    echo "build_swift_harness.sh: $STATICLIB is missing" >&2
    echo "  run 'cargo build --release -p solver-ffi' first" >&2
    exit 2
fi

HEADER="crates/solver-ffi/include/solver.h"
if [[ ! -f "$HEADER" ]]; then
    echo "build_swift_harness.sh: $HEADER is missing" >&2
    echo "  cbindgen should have regenerated it during cargo build — try" >&2
    echo "  'cargo clean -p solver-ffi && cargo build -p solver-ffi'" >&2
    exit 2
fi

# Compile the A13 smoke harness (preserves existing behaviour).
echo "==> building target/swift-harness (A13 smoke test)"
swiftc crates/solver-ffi/examples/swift-harness/main.swift \
    -import-objc-header "$HEADER" \
    -L target/release -lsolver_ffi \
    -o target/swift-harness

# Compile the A27 e2e harness.
echo "==> building target/swift-harness-e2e (A27 outcome test)"
swiftc crates/solver-ffi/examples/swift-harness/main_e2e.swift \
    -import-objc-header "$HEADER" \
    -L target/release -lsolver_ffi \
    -o target/swift-harness-e2e

echo "==> built target/swift-harness + target/swift-harness-e2e"
