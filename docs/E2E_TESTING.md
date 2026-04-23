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
suited combo. `AKs` is the narrowest syntax that selects the hero's
exact hand on this board — 4 combos total, two of which get pruned by
card-conflict (AhKh uses board cards; so does AsKs).

## Running it

```bash
bash scripts/e2e.sh
```

That builds the workspace in release, runs the CLI solve, runs the
ignored Rust FFI integration test, builds the Swift harness (if swiftc
is installed), and runs it. Any failure stops the script non-zero.

To run just the Rust-side FFI test on its own (after a release build):

```bash
cargo build --release --workspace
cargo test --release -p solver-cli --test e2e_integration \
    -- --ignored end_to_end
```

To rebuild and run just the Swift harness:

```bash
cargo build --release -p solver-ffi
bash scripts/build_swift_harness.sh
./target/swift-harness-e2e
```

## The files

| Path | Role |
|---|---|
| `scripts/e2e.sh` | Outer driver — builds, runs CLI, runs Rust test, runs Swift binary. |
| `scripts/build_swift_harness.sh` | Compiles both `main.swift` (A13 smoke) and `main_e2e.swift` (A27 outcome) with swiftc. |
| `crates/solver-cli/tests/e2e_integration.rs` | The Rust integration test. Asserts JSON shape, FFI struct layout offsets, numeric agreement between CLI and FFI paths. `#[ignore]`d so plain `cargo test` skips it — it needs the release binary on disk. |
| `crates/solver-ffi/examples/swift-harness/main_e2e.swift` | Swift consumer that builds the canonical HandState from Swift, calls `solver_solve`, asserts the result, emits JSON for the driver to capture. |

## Fail-loud discipline (expected during v0.1 WIP)

**As of 2026-04-23, the e2e test is expected to fail.** It is not green
yet — and that is the point. Per the A27 task brief:

> If the solver is not ready to produce real output (NlheSubgame::new
> not yet wired to CLI), the test must **FAIL LOUDLY** with a clear
> error message — not silently skip.

The current failure modes are:

1. **CLI path fails** with
   `solver-nlhe::NlheSubgame is not yet implemented — run this after Day 2 main path lands (A-main agent owns NlheSubgame::new)`.
   That's the `build_subgame` placeholder in
   `crates/solver-cli/src/solve_cmd.rs` — it is explicitly still a
   guard that bails before calling `NlheSubgame::new`, which itself
   already exists in the library. Remove the `anyhow::bail!` and call
   `NlheSubgame::new(parsed.board, parsed.hero, parsed.villain, parsed.pot, parsed.stack, Player::Hero, parsed.bet_tree)`
   and this path goes green.
2. **FFI path fails** because `solver_ffi::solver_solve` returns
   `InternalError` (-2). The stub in `crates/solver-ffi/src/lib.rs` is
   a `TODO (Day 4, agent A_main): dispatch to solver_core`. When the
   FFI dispatches into `solver_core::CfrPlus` with a `NlheSubgame`
   constructed from the input `HandState`, this path goes green.
3. **Swift path fails** for the same reason as the FFI path — the
   Swift harness is the same `solver_solve` call, just from a Swift
   consumer.

When any of those stubs ships real behaviour, re-run `bash scripts/e2e.sh`
and the corresponding step turns green. The test is the shipping gate
for v0.1.

## Strictness guarantees

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
pull the repo, `bash scripts/e2e.sh`, and see exactly what Henry sees.
