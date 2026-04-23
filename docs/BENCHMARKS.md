# Benchmarks

## Why benchmarks are load-bearing

"I think it's faster" doesn't ship. **Every performance claim must be
backed by a criterion run.** With 10 parallel agents optimizing different
parts of the solver, the only way to keep the product fast is to measure
continuously and refuse regressions.

## Running benchmarks

```bash
cd ~/Desktop/poker-solver
cargo bench -p solver-core
```

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

### `regret_matching_inner`
Pure microbench: 1326-wide regret matching. Measures the SIMD inner loop
in isolation.

**Target: < 1 µs per call.**

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

Numbers get filled in as Day 3 completes. Until then, targets only.

| Bench | Day 2 (scalar) | Day 3 (SIMD+rayon) | Day 4 (Metal, if built) | v0.1 target |
|---|---|---|---|---|
| `river_canonical_spot` | TBD | TBD | TBD | < 300 ms |
| `river_degenerate_spot` | TBD | TBD | TBD | < 50 ms |
| `river_wet_board` | TBD | TBD | TBD | < 500 ms |
| `regret_matching_inner` | TBD | TBD | TBD | < 1 µs |
| `range_vs_range_equity` | TBD | TBD | TBD | < 2 ms |
| `turn_canonical_spot` | TBD | TBD | TBD | < 30 s |
| `cache_lookup` | TBD | — | — | < 10 µs |

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
