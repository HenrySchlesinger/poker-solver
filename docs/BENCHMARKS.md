# Benchmarks

## Why benchmarks are load-bearing

"I think it's faster" doesn't ship. **Every performance claim must be
backed by a criterion run.** With 10 parallel agents optimizing different
parts of the solver, the only way to keep the product fast is to measure
continuously and refuse regressions.

## Running benchmarks

Run everything under `solver-core`:

```bash
cd ~/Desktop/poker-solver
cargo bench -p solver-core
```

Or target one bench file at a time (much faster while iterating):

```bash
# Regret-matching microbench (sizes 3, 8, 26, 169, 1326)
cargo bench -p solver-core --bench regret_matching

# Full CFR+ solve on Kuhn Poker (10, 100, 1000 iterations)
cargo bench -p solver-core --bench cfr_kuhn

# River KPI harness. Until NLHE lands, only the Kuhn placeholder
# subgroup runs. The real `river_canonical_spot` / `river_degenerate_spot`
# / `river_wet_board` benches are stubs that print "SKIPPED" — flip the
# env var once `solver-nlhe::NlheSubgame` is implemented:
cargo bench -p solver-core --bench river
SOLVER_RUN_RIVER_BENCH=1 cargo bench -p solver-core --bench river
```

Filter within a bench file by passing a substring:

```bash
cargo bench -p solver-core --bench regret_matching -- 1326
cargo bench -p solver-core --bench cfr_kuhn -- /1000
```

### Saving and comparing baselines

Before a refactor, save the current numbers as a named baseline:

```bash
cargo bench -p solver-core -- --save-baseline pre-change
```

After the refactor, compare against it. Criterion prints a regression
table showing percent change for every bench:

```bash
cargo bench -p solver-core -- --baseline pre-change
```

Baselines are stored under `target/criterion/<bench>/<baseline-name>/`.
They are per-workspace and not checked in; re-measure from `main` when
you need a fresh baseline.

### What counts as a regression

**> 5% slower on any bench is a regression.** Commits that cause one
without a written justification in the commit message are rejected.

Criterion's report makes this explicit: the "change" row flags any bench
with `p < 0.05` and a mean change outside a user-specified threshold.
The default threshold is 2 %; we raise the rejection bar to 5 % to absorb
hardware noise on a laptop, but the actual criterion output still shows
smaller changes so you can spot drift.

Criterion outputs HTML reports to `target/criterion/` and summary stats
to stdout. The summary is what goes in commit messages and PR descriptions.

## The benches we care about

### `river_canonical_spot`
The primary KPI. A known river spot (AhKh2s-Qh-4d, hero AA+ vs villain
broadway range, 100bb pot, 2-action bet tree). 1000 CFR+ iterations.
Measures wall-clock time.

**Target: < 300 ms on M1 Pro. Hard limit: < 1 s.**

```
cargo bench -p solver-core -- river_canonical_spot
```

### `river_degenerate_spot`
All-in preflop → river. Ranges are effectively binary (all hands go to
showdown). Tests that the solver handles trivial subgames fast, no
degenerate loops.

**Target: < 50 ms.**

### `river_wet_board`
Wet/drawy board like JhTh9c-8h-7h (four-to-a-flush + straight texture).
More action complexity, more nodes. Stress test for the node count.

**Target: < 500 ms on M1 Pro.**

### `regret_matching_scalar/{3, 8, 26, 169, 1326}`
Microbench group in `benches/regret_matching.rs`. Runs the scalar
`regret_match` inner loop at every size that shows up in practice:

- **3** — tiny bet tree (check / bet / raise)
- **8** — wider bet-sizing tree
- **26** — Kuhn-shaped action sets after history
- **169** — NLHE pre-flop hand grid
- **1326** — all NLHE combos (the river hot path)

Inputs are seeded random f32 regrets from
`rand_xoshiro::Xoshiro256PlusPlus` with `Seed([1; 32])` — reproducible
across runs on the same hardware. Agent A20 owns the SIMD counterpart in
`benches/simd_matching.rs`; DO NOT add SIMD variants to this file.

```bash
cargo bench -p solver-core --bench regret_matching
```

**Target (N=1326): < 1 µs per call (post-SIMD). Scalar baseline is
higher — see table below.**

### `cfr_plus_kuhn/{10, 100, 1000}`
Full CFR+ solve on Kuhn Poker at three iteration counts. Reports wall
time and iterations/sec. Not a direct proxy for NLHE, but catches
regressions in the generic `CfrPlus::run_from` tree walk that the NLHE
river benches would otherwise only catch after Day 3.

```bash
cargo bench -p solver-core --bench cfr_kuhn
```

There is also a `cfr_plus_kuhn_single_iteration` bench that measures one
`iterate_from` call on a fresh solver — useful for reasoning about
inner-loop cost in isolation from per-iteration averaging.

### `range_vs_range_equity`
Pure equity calculation (no CFR). 1326×1326 matmul. Sanity-check for the
matrix multiply performance; this is the floor the solver builds on.

**Target: < 2 ms for full board, < 20 µs per iteration at river.**

### `turn_canonical_spot`
Turn subgame, MCCFR external sampling, 500 iterations. Tests that we
handle the larger tree.

**Target: < 30 s on M1 Pro. Hard limit: < 60 s.**

### `cache_lookup`
Hashmap lookup for precomputed flop. Tests that the "cache hit" path is
actually fast.

**Target: < 10 µs.**

## Benchmark discipline

1. **Benches live with the code.** A new feature without a corresponding
   bench is incomplete. A performance optimization without a bench proving
   it is a vibes-based commit and doesn't merge.

2. **Regressions block.** If your commit regresses any bench by > 5%
   without a written justification, the commit is rejected. The criterion
   tool's `--baseline` flag makes this easy:
   ```bash
   cargo bench -p solver-core -- --save-baseline main
   # ... make changes ...
   cargo bench -p solver-core -- --baseline main
   ```
   It prints a regression table.

3. **Bench in release mode, always.** Criterion handles this, but if
   you're running ad-hoc perf tests, remember `cargo run --release`.

4. **Warm up and average.** Criterion's defaults (30+ samples) are fine.
   Don't commit numbers based on a single run.

5. **Report on reproducible hardware.** All "official" numbers are from
   Henry's M-series MacBook. Someone else's M2 Air running background
   apps is not a valid data source for a regression claim.

## The baseline table

Numbers from `cargo bench -p solver-core --bench <name>` on Henry's
MacBook (Apple Silicon, `cargo 1.95.0 stable`, release profile with
`lto = "fat"`, `codegen-units = 1`). All times are criterion's mean
(middle of the three-number interval it reports).

Day-1 (scalar) numbers are filled in and marked `baseline`. Day-2, Day-3
and Day-4 columns get filled in as those optimizations land.

### Primary KPI (river)

| Bench | Day 1 (scalar, baseline) | Day 2 (flat tables) | Day 3 (SIMD+rayon) | Day 4 (Metal, if built) | v0.1 target |
|---|---|---|---|---|---|
| `river_canonical_spot`  | not yet wired *(NLHE river subgame doesn't exist; see `benches/river.rs`)* | TBD | TBD | N/A (Metal slower at N=1326 — see SIMD vs Metal section below) | < 300 ms |
| `river_degenerate_spot` | not yet wired | TBD | TBD | N/A (same reason) | < 50 ms |
| `river_wet_board`       | not yet wired | TBD | TBD | N/A (same reason) | < 500 ms |

Placeholder we measure instead until NLHE lands:

| Bench | Day 1 (scalar, baseline) | Notes |
|---|---|---|
| `river_placeholder_kuhn_1000_iters` | **5.39 ms** | Kuhn Poker, 1000 CFR+ iterations. Proxy for wiring; NOT a proxy for NLHE river cost. |

### Inner-loop microbench (`regret_matching` — scalar only, Day 1 owns this file)

| Bench | Day 1 (scalar, baseline) | Target |
|---|---|---|
| `regret_matching_scalar/3`    | **2.26 ns**  | — |
| `regret_matching_scalar/8`    | **3.29 ns**  | — |
| `regret_matching_scalar/26`   | **12.72 ns** | — |
| `regret_matching_scalar/169`  | **141.8 ns** | — |
| `regret_matching_scalar/1326` | **1.67 µs**  | < 1 µs post-SIMD (A20) |

N=1326 is ~1.67 µs scalar — close to the SIMD target of < 1 µs, which is
why A20's SIMD path is a Day-3 priority rather than Day-1.

### SIMD vs scalar vs Metal (`simd_matching`, `metal_matching`)

Measured on Henry's M-series MacBook, release profile, criterion mean.
SIMD is the `wide::f32x8` path in `matching_simd.rs` (A20). Metal is the
GPU compute path in `src/metal/` (A26/A40/A51), gated behind
`--features metal` and `cfg(target_os = "macos")`.

| N    | Scalar  | SIMD (wide::f32x8) | Metal (GPU) | SIMD vs scalar | Metal vs SIMD |
|------|---------|---------------------|-------------|-----------------|----------------|
| 169  | 155 ns  | **17.4 ns**         | 110 µs      | **8.9× faster** | **6320× slower** |
| 1326 | 1.77 µs | **192.7 ns**        | 112 µs      | **9.2× faster** | **580× slower** |
| 4096 | 5.43 µs | (not benched)       | 105 µs      | — | — |

**Metal is dramatically slower than SIMD at these sizes on Apple Silicon.**
This is the expected, publishable finding: GPU dispatch overhead on the
shared command-buffer + wait_until_completed path is ~100 µs per call,
which dominates the actual compute work at any N < ~100k floats. The
dispatch overhead is roughly flat across N=169..4096, confirming that
what we're measuring is the launch cost, not the kernel cost.

Why Metal loses here specifically:

1. **The problem is 1326 floats.** That's 5 KB — it fits in L1 cache on
   any modern CPU. The GPU never gets a chance to amortize its launch
   overhead across enough work.
2. **We wait synchronously** (`command_buffer.wait_until_completed()`)
   because the solver's inner loop needs the result before the next
   regret-matching call. There's no way to hide the ~100 µs round-trip.
3. **Apple Silicon's unified memory doesn't help** at this size. The
   memcpy cost is negligible either way; what matters is the command
   submission + GPU wake + kernel launch + readback fence.

Metal would win at substantially larger problems — e.g. batched
regret-matching across thousands of info sets in one dispatch, or the
1326×1326 matmul in range-vs-range equity (not yet benched). For the
v0.1 solver's inner loop, **the SIMD path at ~193 ns is the shipping
choice**, and the Metal code is kept for future batched-kernel work.

Run the Metal bench (requires Xcode Metal Toolchain downloaded):

```bash
cargo bench -p solver-core --features metal --bench metal_matching
```

Run the equivalence test (gates the shader's correctness vs scalar
within 1e-4 across a 10k random-trial property sweep):

```bash
cargo test --release -p solver-core --features metal --test metal_equivalence
```

### Full-solve Kuhn (`cfr_kuhn`)

| Bench | Day 1 (scalar, baseline) | Iterations/sec |
|---|---|---|
| `cfr_plus_kuhn/10`                | **91.6 µs**  | ~109k iters/s |
| `cfr_plus_kuhn/100`               | **528.7 µs** | ~189k iters/s |
| `cfr_plus_kuhn/1000`              | **5.10 ms**  | ~196k iters/s |
| `cfr_plus_kuhn_single_iteration`  | **6.01 µs**  | — |

Linear scaling (10 → 100 → 1000 iters produces ~10× → ~100× wall time,
with the fixed "fresh solver + avg-strategy computation" overhead
visible at the 10-iter size). Good signal the tree walk is steady-state.

### Future benches (not yet implemented)

| Bench | Day N (scalar) | Day N (optimized) | v0.1 target |
|---|---|---|---|
| `range_vs_range_equity` | TBD | TBD | < 2 ms full-board, < 20 µs/iter at river |
| `turn_canonical_spot`   | TBD | TBD | < 30 s (hard limit 60 s) |
| `cache_lookup`          | TBD | — | < 10 µs |

## Flamegraphs

For deep dives, use `cargo flamegraph`:

```bash
cargo install flamegraph
cargo flamegraph --bench river -- river_canonical_spot --bench
open flamegraph.svg
```

Look for:
- Hot functions not in `solver-core` (indicates layering bugs)
- Allocator calls on the hot path (should be zero)
- Memcpy dominance (indicates layout / stride issues)
- Branch mispredictions in regret matching (dense conditionals are bad)

## Correctness ≠ performance

A fast solver that produces wrong strategies is worse than a slow correct
one. **Validation tests gate benchmarks.** Don't optimize a broken solve.

Validation lives in `crates/solver-core/tests/convergence.rs` and
`crates/solver-cli/tests/vs_texassolver.rs`. Must pass before any bench
numbers are considered valid.
