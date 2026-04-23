#!/usr/bin/env bash
#
# e2e.sh — the "does the whole pipeline work?" gate for poker-solver v0.1.
#
# Runs three consumers of the same canonical spot (royal-flush-on-board,
# both players AKs, 100/500, 100 iters) and reports per-path status:
#
#     CLI (JSON over stdout)           ─┐
#     FFI (cargo test in-process)      ─┼─→ Same SolveResult (expected)
#     Swift (swiftc-built binary)      ─┘
#
# Structure: each path runs independently and reports green / blocked-on-<agent>
# / skipped. A missing upstream dependency (stubbed solver, missing swiftc,
# etc.) does NOT fail the whole script — it produces a clear blocker message
# and the other paths continue to run. Only a catastrophic failure (workspace
# build broken) aborts.
#
# Exit status: 0 iff all runnable paths produced expected output and no
# blockers remain. Non-zero if any blocker was hit (so CI surfaces it),
# but the script always finishes and reports all three paths.

# Note: -u catches unset-variable typos, pipefail catches failures in pipelines,
# but NOT -e — we want individual step failures to be reported, not fatal.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Tally of path statuses: 0 = ran cleanly or cleanly skipped; 1 = blocker.
FAILED=0

# CLI test params. Kept here (not inlined) so one edit updates all three
# paths' canonical-spot constants.
CLI_BOARD="AhKhQhJhTh"
CLI_HERO="AKs"
CLI_VILLAIN="AKs"
CLI_POT=100
CLI_STACK=500
CLI_ITERS=100

# Step 0: workspace build. If this fails, nothing else can run — abort.
echo "==> cargo build --release --workspace"
if ! cargo build --release --workspace 2>&1 | tail -5; then
    echo "FATAL: workspace build failed — stopping before running any path"
    exit 1
fi

# ----------------------------------------------------------------------
# [1/3] CLI path
# ----------------------------------------------------------------------
echo
echo "==> [1/3] CLI path: ./target/release/solver-cli solve --board $CLI_BOARD ..."
# Bound the CLI's compute time — on a stubbed/incomplete solver it can
# blow up RAM or spin indefinitely. macOS ships neither `timeout` nor
# `gtimeout` by default, so prefer those when available and otherwise
# fall back to a portable background-pid-watchdog.
CLI_TIMEOUT_SEC=60
CLI_OUT_FILE="$(mktemp -t solver-cli-e2e.XXXXXX)"
CLI_ERR_FILE="$(mktemp -t solver-cli-e2e.XXXXXX)"

run_cli_with_timeout() {
    local out="$1" err="$2"
    if command -v gtimeout >/dev/null; then
        gtimeout "$CLI_TIMEOUT_SEC" ./target/release/solver-cli solve \
            --board "$CLI_BOARD" --hero-range "$CLI_HERO" \
            --villain-range "$CLI_VILLAIN" --pot "$CLI_POT" \
            --stack "$CLI_STACK" --iterations "$CLI_ITERS" \
            >"$out" 2>"$err"
        return $?
    fi
    if command -v timeout >/dev/null; then
        timeout "$CLI_TIMEOUT_SEC" ./target/release/solver-cli solve \
            --board "$CLI_BOARD" --hero-range "$CLI_HERO" \
            --villain-range "$CLI_VILLAIN" --pot "$CLI_POT" \
            --stack "$CLI_STACK" --iterations "$CLI_ITERS" \
            >"$out" 2>"$err"
        return $?
    fi
    # Portable watchdog: launch the CLI in the background, kill it if
    # it doesn't finish within CLI_TIMEOUT_SEC. Returns 124 on timeout
    # (matching coreutils convention) so the caller can distinguish.
    ./target/release/solver-cli solve \
        --board "$CLI_BOARD" --hero-range "$CLI_HERO" \
        --villain-range "$CLI_VILLAIN" --pot "$CLI_POT" \
        --stack "$CLI_STACK" --iterations "$CLI_ITERS" \
        >"$out" 2>"$err" &
    local pid=$!
    local waited=0
    while kill -0 "$pid" 2>/dev/null && [[ $waited -lt $CLI_TIMEOUT_SEC ]]; do
        sleep 1
        waited=$((waited + 1))
    done
    if kill -0 "$pid" 2>/dev/null; then
        kill -KILL "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
        return 124
    fi
    wait "$pid"
    return $?
}

run_cli_with_timeout "$CLI_OUT_FILE" "$CLI_ERR_FILE"
CLI_RC=$?

if [[ $CLI_RC -eq 0 ]]; then
    # Validate JSON parses. jq is present on most dev boxes; if it's not,
    # accept any non-empty stdout as "looks plausible".
    if command -v jq >/dev/null; then
        if jq . "$CLI_OUT_FILE" >/dev/null 2>&1; then
            echo "  ok: CLI produced valid JSON ($(wc -c <"$CLI_OUT_FILE") bytes)"
            # Pretty-print for the transcript — useful when the run is cold-reviewed.
            jq . "$CLI_OUT_FILE" | head -25
        else
            echo "  BLOCKED: CLI exit 0 but stdout not valid JSON:"
            head -5 "$CLI_OUT_FILE"
            FAILED=1
        fi
    else
        if [[ -s "$CLI_OUT_FILE" ]]; then
            echo "  ok: CLI exit 0, produced $(wc -c <"$CLI_OUT_FILE") bytes (jq unavailable, skipping JSON validation)"
        else
            echo "  BLOCKED: CLI exit 0 but produced no output"
            FAILED=1
        fi
    fi
elif [[ $CLI_RC -eq 124 || $CLI_RC -eq 137 ]]; then
    # 124 = gtimeout/timeout killed it; 137 = SIGKILL (often OOM).
    echo "  BLOCKED on solver-core/main-path wiring: CLI hit resource limit"
    echo "  (exit $CLI_RC after ${CLI_TIMEOUT_SEC}s or OOM — the CFR walk on a"
    echo "   river spot is exhausting memory/time; likely the CFR path needs more"
    echo "   pruning or a smaller default iteration budget)."
    head -10 "$CLI_ERR_FILE" | sed 's/^/    /'
    FAILED=1
else
    echo "  BLOCKED on A47 (solver-cli wiring): exit $CLI_RC"
    head -15 "$CLI_ERR_FILE" | sed 's/^/    /'
    FAILED=1
fi

# ----------------------------------------------------------------------
# [2/3] FFI path (Rust integration test)
# ----------------------------------------------------------------------
#
# The test itself has a 60s CLI-subprocess watchdog built in (see
# CLI_TIMEOUT_SEC in e2e_integration.rs), so a runaway CFR won't wedge
# this step. We still cap the whole cargo-test run at 3× that plus a
# little slack to catch infinite-loop regressions in the FFI dispatch
# path as a last resort.
FFI_TIMEOUT_SEC=240

echo
echo "==> [2/3] FFI path (cargo test --ignored end_to_end)"
FFI_OUT_FILE="$(mktemp -t solver-ffi-e2e.XXXXXX)"
cargo test --release -p solver-cli --test e2e_integration \
    -- --ignored --test-threads=1 end_to_end \
    >"$FFI_OUT_FILE" 2>&1 &
FFI_PID=$!
FFI_WAITED=0
while kill -0 "$FFI_PID" 2>/dev/null && [[ $FFI_WAITED -lt $FFI_TIMEOUT_SEC ]]; do
    sleep 2
    FFI_WAITED=$((FFI_WAITED + 2))
done
if kill -0 "$FFI_PID" 2>/dev/null; then
    echo "  BLOCKED: cargo test exceeded ${FFI_TIMEOUT_SEC}s wall-clock watchdog — killing"
    kill -KILL "$FFI_PID" 2>/dev/null || true
    # Also reap any stray child solver-cli processes. `pkill -P` targets
    # cargo's children specifically; a broader `pkill solver-cli` would
    # be unsafe if the user is running one in another terminal.
    pkill -P "$FFI_PID" 2>/dev/null || true
    wait "$FFI_PID" 2>/dev/null || true
    FFI_RC=124
else
    wait "$FFI_PID"
    FFI_RC=$?
fi
if [[ $FFI_RC -eq 0 ]]; then
    # `cargo test` returns 0 when no tests matched, too — verify we actually ran it.
    if grep -q "test end_to_end .* ok" "$FFI_OUT_FILE"; then
        echo "  ok: FFI integration test passed"
    elif grep -q "0 passed; 0 failed" "$FFI_OUT_FILE"; then
        echo "  SKIPPED: no tests matched 'end_to_end' (the test was renamed?)"
        FAILED=1
    else
        echo "  ok: cargo test exit 0 (no matching test name found in output — verify)"
        tail -10 "$FFI_OUT_FILE" | sed 's/^/    /'
    fi
else
    echo "  BLOCKED on A47 (solver_solve stub in solver-ffi/src/lib.rs returns InternalError)"
    tail -25 "$FFI_OUT_FILE" | sed 's/^/    /'
    FAILED=1
fi

# ----------------------------------------------------------------------
# [3/3] Swift path
# ----------------------------------------------------------------------
echo
echo "==> [3/3] Swift path"
if ! command -v swiftc >/dev/null; then
    echo "  SKIPPED: swiftc not on PATH (run 'xcode-select --install' to get it)"
elif ! [[ -f "target/release/libsolver_ffi.a" ]]; then
    echo "  SKIPPED: target/release/libsolver_ffi.a missing (workspace build earlier produced cdylib only?)"
else
    # Build the harness. If swiftc link fails, report it without aborting.
    SWIFT_BUILD_OUT="$(mktemp -t swift-harness-build.XXXXXX)"
    if bash scripts/build_swift_harness.sh >"$SWIFT_BUILD_OUT" 2>&1; then
        tail -3 "$SWIFT_BUILD_OUT"
        # Run the e2e harness (the one that asserts Ok, not the smoke binary).
        SWIFT_OUT_FILE="$(mktemp -t swift-harness-e2e.XXXXXX)"
        ./target/swift-harness-e2e >"$SWIFT_OUT_FILE" 2>&1
        SWIFT_RC=$?
        if [[ $SWIFT_RC -eq 0 ]]; then
            echo "  ok: Swift harness asserted SolveResult.Ok"
            head -10 "$SWIFT_OUT_FILE" | sed 's/^/    /'
        else
            echo "  BLOCKED: Swift harness ran but exited $SWIFT_RC"
            head -15 "$SWIFT_OUT_FILE" | sed 's/^/    /'
            FAILED=1
        fi
    else
        echo "  BLOCKED: swiftc build failed"
        tail -20 "$SWIFT_BUILD_OUT" | sed 's/^/    /'
        FAILED=1
    fi
fi

# ----------------------------------------------------------------------
# Summary
# ----------------------------------------------------------------------
echo
if [[ $FAILED -eq 0 ]]; then
    echo "==> e2e.sh: ALL PATHS GREEN"
    exit 0
else
    echo "==> e2e.sh: completed with blockers (see per-path report above)"
    echo "    The most common blocker today is A47's wiring of"
    echo "    solver_ffi::solver_solve into solver_core::CfrPlus. When that"
    echo "    lands, re-run this script; blockers should clear."
    exit 1
fi
