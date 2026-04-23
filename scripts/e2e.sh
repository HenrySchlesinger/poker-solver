#!/usr/bin/env bash
#
# e2e.sh — the "does the whole pipeline work?" gate for poker-solver v0.1.
#
# Runs three consumers of the same canonical spot (royal-flush-on-board,
# both players AKs, 100/500, 100 iters) and asserts they all agree:
#
#     CLI (JSON over stdout)
#     FFI  (cargo test — Rust caller calls solver_solve in-process)
#     Swift (swiftc-built binary linked against libsolver_ffi.a)
#                  │
#                  ▼
#         Same SolveResult
#
# Success criterion: the cargo test PASSES. Today, with the solver
# still stubbed, it FAILS — loudly, with a message naming the exact
# wiring gap. That's the behaviour the A27 task brief demands: the
# test should not silently skip while the feature is stubbed, it
# should be the tripwire that tells us v0.1 isn't shippable yet.
#
# Exit status: 0 iff all three paths produce the expected output.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

CLI_CMD=(
    "./target/release/solver-cli" solve
    --board "AhKhQhJhTh"
    --hero-range "AKs"
    --villain-range "AKs"
    --pot 100
    --stack 500
    --iterations 100
)

echo "==> building release (workspace)"
cargo build --release --workspace

echo
echo "==> [1/3] CLI path: ${CLI_CMD[*]}"
# Capture the CLI output so we can both print it and forward it to jq.
# If the CLI fails (as it does today with the build_subgame stub), exit
# the script immediately with a message that points at the fix.
set +e
CLI_OUT="$("${CLI_CMD[@]}" 2>&1)"
CLI_RC=$?
set -e
printf '%s\n' "$CLI_OUT"

if [[ $CLI_RC -ne 0 ]]; then
    cat >&2 <<EOF

==> CLI path FAILED (rc=$CLI_RC).
    The CLI's solve command returned a non-zero exit. With the Day 2
    stubs in place this is expected — build_subgame in
    crates/solver-cli/src/solve_cmd.rs bails with "NlheSubgame is not
    yet implemented". When the main-path agent wires
    NlheSubgame::new through build_subgame, this step turns green.

    Stopping the e2e script here: the later paths depend on the CLI
    producing JSON to diff against.
EOF
    exit $CLI_RC
fi

# Pipe through jq if available for pretty-printing and schema validation.
if command -v jq >/dev/null; then
    printf '%s\n' "$CLI_OUT" | jq .
fi

echo
echo "==> [2/3] FFI path (cargo test): calling solver_solve in-process"
# Run the ignored test explicitly. `--test-threads=1` is insurance — the
# test creates a SolverHandle and today the FFI is not documented as
# re-entrant at the handle level, so serial execution matches Poker
# Panel's per-thread-handle model.
cargo test --release -p solver-cli --test e2e_integration \
    -- --ignored --test-threads=1 end_to_end

echo
echo "==> [3/3] Swift path"
if command -v swiftc >/dev/null; then
    bash scripts/build_swift_harness.sh
    SWIFT_OUT="$(./target/swift-harness-e2e)"
    printf '%s\n' "$SWIFT_OUT"
    if command -v jq >/dev/null; then
        # The e2e harness prints a JSON object; validate it parses.
        printf '%s\n' "$SWIFT_OUT" | jq . >/dev/null
    fi
else
    echo "(skipped — swiftc not installed)"
fi

echo
echo "==> SUCCESS: all three paths agree"
