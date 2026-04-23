# End-to-end integration testing

This doc explains the three-path end-to-end test for poker-solver v0.1.
It is the single gate that proves the pipeline works top-to-bottom:
command-line input → Rust solver → SolveResult → Swift consumer. If all
three paths agree on the same canonical spot, Poker Panel integration
is unblocked.

## The three paths

```
        CLI (JSON stdout)       FFI (Rust test)       Swift (binary)
              │                       │                      │
              │  solver-cli solve ... │ solver_ffi::         │ swiftc-built
              │  → serde_json JSON    │ solver_solve(...)    │ binary links
              │  → parsed by test     │ → populates          │ libsolver_ffi.a
              │                       │   SolveResult        │ → same solve
              │                       │                      │
              └───────────────────────┼──────────────────────┘
                                      ▼
                             Same SolveResult
                          (hero_equity, iterations,
                           action_freq all agree)
```

The three paths cover three distinct risks:

| Path | What it catches |
|---|---|
| CLI | The entire `serde_json` output schema + the `solver-cli` argparse layer. Regressions to the JSON shape break automation scripts that shell out. |
| FFI (Rust test) | The C-ABI surface as exposed from Rust. Catches layout drift in `HandState` / `SolveResult`, stubs that never got wired, panic-leakage across the boundary. |
| Swift | Clang's C importer agreeing with cbindgen's output, the static-library link, and — most important — Swift's inferred struct layout byte-equal to Rust's. Poker Panel consumes the FFI from Swift, not C, so this is the actual consumer-representative test. |

## The canonical spot

All three paths solve the same spot:

```
Board:          AhKhQhJhTh  (royal flush on board)
Hero range:     AKs
Villain range:  AKs
Pot:            100
Stack:          500
Iterations:     100
```

**Why this spot:** a royal flush on the board means both players play
the board; hero equity is exactly 0.5 regardless of which AKs combo is
dealt. That gives every path a deterministic numeric target to agree
on, independent of CFR convergence noise.

**Why not a single combo like `AsKs`:** the v0.1 range parser (`solver-nlhe::Range::parse`)
accepts `AA`, `AKs`, `AKo`, `AK`, `22+`, etc., but not yet a specific
suited combo on its own. `AKs` is the narrowest syntax that selects
hero's exact hand on this board — 4 combos total, two of which get
pruned by card-conflict (AhKh and AsKs use board cards).

## Running it

```bash
bash scripts/e2e.sh
```

That builds the workspace in release, runs the CLI solve, runs the
ignored Rust FFI integration test, builds the Swift harness (if swiftc
is installed), and runs it.

### Script discipline: resilient, not fail-fast

`scripts/e2e.sh` uses `set -uo pipefail` (deliberately not `-e`). A
failure in one path does NOT abort the others — each path runs
independently, reports its own status, and the summary says clearly
which paths are green and which are blocked and on whom. This matches
the v0.1 reality: we're in a speedrun where different agents are
landing different paths in parallel, and one agent's partial work
should not mask another's success.

**Watchdogs.** Both the CLI subprocess (inside the Rust FFI test) and
the outer `cargo test` run are bounded by wall-clock timeouts (60s and
240s respectively). This keeps a runaway CFR from wedging CI for 7.5
minutes when the solver blows up on the river spot.

### Running just the Rust-side FFI test (after a release build):

```bash
cargo build --release --workspace
cargo test --release -p solver-cli --test e2e_integration \
    -- --ignored end_to_end
```

### Running just the Swift harness:

```bash
cargo build --release -p solver-ffi
bash scripts/build_swift_harness.sh
./target/swift-harness-e2e
```

## The files

| Path | Role |
|---|---|
| `scripts/e2e.sh` | Outer driver — builds, runs CLI, runs Rust test, runs Swift binary. Resilient: each step reports independently. |
| `scripts/build_swift_harness.sh` | Compiles both `main.swift` (A13 smoke) and `main_e2e.swift` (A27 outcome) with swiftc. |
| `crates/solver-cli/tests/e2e_integration.rs` | The Rust integration test. Asserts JSON shape, FFI struct layout offsets, numeric agreement between CLI and FFI paths. `#[ignore]`d so plain `cargo test` skips it — it needs the release binary on disk. Has a 60-second CLI subprocess watchdog. |
| `crates/solver-ffi/examples/swift-harness/main_e2e.swift` | Swift consumer that builds the canonical HandState from Swift, calls `solver_solve`, asserts the result, emits JSON for the driver to capture. |

## Latest run (2026-04-23)

This is the actual output from `bash scripts/e2e.sh` on `main`:

```
==> cargo build --release --workspace
    Finished `release` profile [optimized] target(s)

==> [1/3] CLI path: ./target/release/solver-cli solve --board AhKhQhJhTh ...
  ok: CLI produced valid JSON (846 bytes)
{
  "input": {
    "bet_tree": "default",
    "board": "AhKhQhJhTh",
    "hero_range": "AKs",
    "iterations": 100,
    "pot": 100,
    "stack": 500,
    "villain_range": "AKs"
  },
  "result": {
    "action_frequencies": {
      "allin": 0.12381898611783981,
      "bet_100": 0.18642476201057434,
      "bet_200": 0.18642476201057434,
      "bet_33": 0.18643681704998016,
      "bet_66": 0.18642476201057434,
      "check": 0.1304698884487152
    },
    "compute_ms": 28,
    "ev_per_action": {
      "allin": 0.0024752477183938026,
      "bet_100": 0.0032817351166158915,
      "bet_200": 0.0010691173374652863,
      "bet_33": 0.004764307290315628,
      ...
    }
  }
}

==> [2/3] FFI path (cargo test --ignored end_to_end)
  BLOCKED on A47 (solver_solve stub in solver-ffi/src/lib.rs returns InternalError)
    running 1 test
    test end_to_end ... FAILED

    ---- end_to_end stdout ----
    thread 'end_to_end' panicked at crates/solver-cli/tests/e2e_integration.rs:627:5:
    assertion `left == right` failed: solver_solve returned status -2 (expected Ok=0).

    If rc == -2 (InternalError), `solver_ffi::solver_solve` is still the
    stub in crates/solver-ffi/src/lib.rs that returns InternalError
    unconditionally. Wire it to `solver_core::CfrPlus` + `NlheSubgame::new`
    to complete the FFI path.
      left: -2
     right: 0

    test result: FAILED. 0 passed; 1 failed; 0 ignored; ... finished in 0.25s

==> [3/3] Swift path
==> building target/swift-harness (A13 smoke test)
==> building target/swift-harness-e2e (A27 outcome test)
==> built target/swift-harness + target/swift-harness-e2e
  BLOCKED: Swift harness ran but exited 1
    FAIL: solver_solve returned status -2, expected 0 (Ok).

    If rc == -2 (InternalError), solver_ffi::solver_solve is still the
    Day 2 stub in crates/solver-ffi/src/lib.rs that returns
    InternalError unconditionally. Wire it to solver_core::CfrPlus +
    NlheSubgame::new to complete the Swift-facing FFI path.

==> e2e.sh: completed with blockers (see per-path report above)
    The most common blocker today is A47's wiring of
    solver_ffi::solver_solve into solver_core::CfrPlus. When that
    lands, re-run this script; blockers should clear.
```

## Path status as of 2026-04-23

| Path | Status | What's needed to turn it green |
|---|---|---|
| CLI | **GREEN** | Already solving the canonical spot end-to-end. Returns a valid JSON result block with all six `result.*` fields. Compute time ~28 ms on an M-series Mac. |
| FFI (Rust test) | **BLOCKED on A47** | `solver_ffi::solver_solve` in `crates/solver-ffi/src/lib.rs` is still a stub that returns `SolverStatus::InternalError`. Wire it to build a `NlheSubgame` from the incoming `HandState`, run `solver_core::CfrPlus::run_from`, and populate the `SolveResult` fields. |
| Swift | **BLOCKED on A47** (same root cause) | Depends on `solver_solve` returning `Ok`. The harness, build script, and static-library link path all work — when the FFI path comes online, the Swift path will at worst only need a recompile. |

## v0.1 ship blockers (derived from this test)

1. **Wire `solver_ffi::solver_solve`.** This is the single biggest item.
   When it's done, both FFI and Swift paths flip green simultaneously.
2. **Numeric agreement between CLI and FFI.** Once both paths produce
   output, the test's `assert_agreement` helper will check their
   `hero_equity` values match within `1e-6`. If they disagree, that
   points at a bug in one of the two paths — probably the FFI one,
   since the CLI has a longer tail of unit tests.
3. **Swift result-parse fidelity.** The Swift harness decodes the
   `SolveResult` struct via raw pointer arithmetic using the field
   offsets pinned in the Rust test. If Swift sees different values
   than Rust on the same `SolveResult`, we have an ABI-layout drift
   bug. The test's `assert_ffi_layout` guard is the canary.

## Fail-loud discipline (expected during v0.1 WIP)

The outer `scripts/e2e.sh` exits non-zero while any path is blocked.
That's by design: if CI goes green prematurely, we'd ship v0.1 with a
broken FFI and Poker Panel would silently get garbage results. The
script's per-path messages make blockers obvious so whichever agent
is picking up the next piece of work knows exactly where to look.

## Strictness guarantees (when all three paths are green)

The test is strict on purpose — the whole point is to catch integration
breakage the moment it happens:

- **JSON structure matches exactly.** The top-level keys of
  `solver-cli solve`'s JSON output must be exactly
  `{input, result, solver_version}`; any extra key or missing key is a
  failure. Same for the inner `input.*` and `result.*` fields.
- **Numeric agreement within `1e-6`.** Both paths' `hero_equity`
  values must agree, and both must be within `1e-2` of the known-tie
  value `0.5`.
- **FFI struct layout is pinned.** The test computes field offsets for
  `HandState` and `SolveResult` and asserts specific byte offsets. A
  reorder of fields in `solver-ffi/src/lib.rs` would silently desync
  the Swift bridging header; this test catches that before it ships.
- **SolverStatus enum values are pinned.** `Ok=0, CacheMiss=1,
  InvalidInput=-1, InternalError=-2, OutputTooSmall=-3` are locked.
  Changing them is an ABI break that Poker Panel must be notified of.
- **`iterations >= 1`** on both paths. Ensures neither side is
  short-circuiting and returning zeroed output.
- **Action frequencies sum to 1** if `action_count > 0`. Ensures the
  strategy is a valid probability distribution, not raw regrets.

## Reproducing in Colab

The canonical spot is trivial to reproduce anywhere with a Rust
toolchain and Xcode command-line tools. Nothing in the test depends on
networked services, random seeds the CI can't control, or
installed-in-Applications frameworks. That means a future agent can
pull the repo, `bash scripts/e2e.sh`, and see exactly what the latest
state is.
