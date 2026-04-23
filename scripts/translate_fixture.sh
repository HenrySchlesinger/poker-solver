#!/usr/bin/env bash
# translate_fixture.sh — compatibility shim for the old Python script.
#
# The real translator lives in Rust at
# `crates/solver-cli/src/translate.rs` and is exposed as
# `solver-cli translate-fixture`. This shell script forwards all
# arguments to that subcommand so any call site that still references
# `scripts/translate_fixture.*` keeps working.
#
# Project rule: "Rust wherever possible" (see root CLAUDE.md). An earlier
# draft of this tool lived at `scripts/translate_fixture.py`; it was
# removed on 2026-04-23 when the Rust port landed.
#
# Prefer calling the binary directly in new code:
#     ./target/release/solver-cli translate-fixture --input ... --output ...
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RELEASE_BIN="$REPO_ROOT/target/release/solver-cli"

if [[ -x "$RELEASE_BIN" ]]; then
    exec "$RELEASE_BIN" translate-fixture "$@"
fi

# Fall back to `cargo run` if the release binary hasn't been built yet.
# This is slower but keeps the shim self-sufficient.
if command -v cargo >/dev/null 2>&1; then
    exec cargo run -p solver-cli --release --quiet -- translate-fixture "$@"
fi

echo "translate_fixture.sh: neither $RELEASE_BIN nor \`cargo\` is available." >&2
echo "Build first: cargo build --release -p solver-cli" >&2
exit 1
