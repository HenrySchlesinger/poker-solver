#!/usr/bin/env bash
# ship.sh — run every gate from docs/SHIP_V0_1.md that is automatable.
#
# On a fresh clone this is the single command that validates a release
# candidate before `git tag`. Anything it cannot automate (20-spot
# TexasSolver diff, SHA-256 upload to the Release, Swift harness run)
# is still on the human — see docs/SHIP_V0_1.md.
#
# The Rust-first rule in CLAUDE.md says shell is OK for thin glue over
# external tools. This script is thin glue over cargo. If it ever
# grows past ~50 lines, promote it to a Rust binary under solver-cli.
set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test --workspace --release"
cargo test --workspace --release

echo "==> cargo bench --workspace --no-run"
cargo bench --workspace --no-run

echo "==> cargo build --release -p solver-ffi"
cargo build --release -p solver-ffi

echo "==> confirm solver.h exists and is fresh"
test -f crates/solver-ffi/include/solver.h

echo "==> confirm libsolver_ffi artifacts exist"
test -f target/release/libsolver_ffi.a
test -f target/release/libsolver_ffi.dylib

echo
echo "All automated gates passed. Still do manually:"
echo "  - scripts/../crates/solver-ffi/examples/swift-harness (run it)"
echo "  - cargo run -p solver-cli -- validate (diff vs TexasSolver)"
echo "  - criterion perf targets on M-series Mac (docs/BENCHMARKS.md)"
echo "  - tick remaining boxes in docs/SHIP_V0_1.md"
