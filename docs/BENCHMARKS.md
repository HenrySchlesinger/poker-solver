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

# River KPI harness. Wired to real NLHE subgames as of 2026-04-23
# (A62). Runs three benches: `river_canonical_spot` (100 CFR+ iters),
# `river_degenerate_spot` (1000 iters), `river_wet_board` (100 iters).
# See the "Iteration-count note" in the Primary KPI table for why the
# heavier spots are at 100 iters pre-SIMD.
cargo bench -p solver-core --bench river
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
The primary KPI. A known river spot (`AhKhQh2d4s`, hero `"AA,KK,AKs"`
vs villain `"22+,AJs+,KQs"`, pot 100, stack 500, default 5-sizing bet
tree). Currently **100 CFR+ iterations** pre-SIMD (see iteration-count
note). Measures wall-clock time.

**Target: < 300 ms on M1 Pro at 1000 iterations. Hard limit: < 1 s.**

**Day 1 (scalar) baseline: 478 ms @ 100 iters** (2026-04-23, A62).
**Day 2+3 (flat + SIMD): 434.65 ms @ 100 iters** (2026-04-23, A64). See
the primary KPI table for the full post-A64 numbers and the
"Why the jump is smaller than expected" analysis.

```
cargo bench -p solver-core -- river_canonical_spot
```

### `river_degenerate_spot`
Already-all-in → river (both players all-in entering the river, so the
tree collapses to Check/Check → showdown). Ranges are specific single
combos: hero `AhKh`, villain `AsAd`, board `2c7d9hTsJs`, pot 1000,
stack 0. Tests that the solver handles trivial subgames fast, no
degenerate loops. Runs at **1000 CFR+ iterations** (sub-microsecond per
iteration).

**Target: < 50 ms.**

**Day 1 (scalar) baseline: 363 µs @ 1000 iters** (2026-04-23, A62) —
137× under target.
**Day 2+3 (flat + SIMD): 268.15 µs @ 1000 iters** (2026-04-23, A64) —
186× under target.

### `river_wet_board`
Wet/drawy board `JhTh9c8h7s` (four-to-a-flush + straight texture), hero
`"AA,AKs,QTs"` vs villain `"22+,AQs+"`, pot 100, stack 500. More action
complexity, more nodes. Stress test for the node count. Currently
**100 CFR+ iterations** pre-SIMD.

**Target: < 500 ms on M1 Pro at 1000 iterations.**

**Day 1 (scalar) baseline: 667 ms @ 100 iters** (2026-04-23, A62) —
~6.67 ms/iter, extrapolates to ~6.67 s at 1000 iters pre-SIMD.
**Day 2+3 (flat + SIMD): 589.27 ms @ 100 iters** (2026-04-23, A64) —
~5.89 ms/iter, still extrapolates to ~5.89 s at 1000 iters.

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

The "Day 1 (scalar, baseline)" numbers below are the most recent committed
baseline run. The append-only source of truth lives in
[`bench-history/`](../bench-history/) — one dated JSON per run. Recent
snapshots:

- `bench-history/2026-04-23_182335_7d6556e.json` (commit `7d6556e`, agent
  A70) — **current river KPI snapshot**: Vector CFR landed as default.
- `bench-history/2026-04-23_110058_3480502.json` (commit `3480502`, agent
  A64) — post-flat+SIMD integration (pre-Vector).
- `bench-history/2026-04-23_105049_d8505fa.json` (commit `d8505fa`, agent
  A62) — river KPI pre-flat+SIMD baseline.
- `bench-history/2026-04-23_094257_8e26e00.json` (commit `8e26e00`, agent
  A55) — `regret_matching_scalar` + `cfr_plus_kuhn` scalar baseline.

### Primary KPI (river)

| Bench | Day 1 (scalar+HashMap, A62) | Day 2+3 (flat+SIMD, A64) | v0.2 (Vector CFR, A70) | Δ vs A64 | v0.1 target |
|---|---|---|---|---|---|
| `river_canonical_spot`  | 478.23 ms @ 100 iters | 434.65 ms @ 100 iters | **40.70 ms @ 100 iters** *(~0.41 ms/iter)* | **-90.6 % (~10.7×)** | < 300 ms @ 1000 iters |
| `river_degenerate_spot` | 363.13 µs @ 1000 iters | 268.15 µs @ 1000 iters | **13.02 ms @ 1000 iters** | +4755 % (see note) | < 50 ms @ 1000 iters |
| `river_wet_board`       | 667.14 ms @ 100 iters | 589.27 ms @ 100 iters | **41.81 ms @ 100 iters** *(~0.42 ms/iter)* | **-92.9 % (~14.1×)** | < 500 ms @ 1000 iters |

**Post-A70 numbers are criterion mean on Henry's M-series MacBook,
clean run, commit `7d6556e` with `CfrPlusVector` wired through
`benches/river.rs`. See `bench-history/2026-04-23_182335_7d6556e.json`
for the full snapshot.**

**Extrapolated to 1000 iters:**
- `river_canonical_spot` ~407 ms @ 1000 iters → clears 1 s hard limit,
  over the 300 ms ideal target by ~107 ms.
- `river_wet_board` ~418 ms @ 1000 iters → under the 500 ms target.
- `river_degenerate_spot` stays at 13.02 ms @ 1000 iters regardless —
  well under the 50 ms target even though the per-iter floor regressed.

**Degenerate-spot regression note.** The vector solver always walks
1326-wide reach vectors, even on a 1-combo-vs-1-combo spot where
scalar would walk a single lane. The per-iteration floor is therefore
higher for trivial spots (~13 µs/iter in vector vs ~0.27 µs/iter in
A64 flat). This is an accepted trade-off: the trivial path is still
186× under the 50 ms target, and real river spots (canonical, wet)
net 10-14× faster. Only a small fraction of production traffic hits
the trivial path — most river solves have 100+ viable combo pairs.

**Iteration-count note (2026-04-23, A62/A64).** The CFR+ inner
loop on the two heavy spots still takes ~4-6 ms per iteration post-A64,
so 1000 iterations × 100 criterion samples = ~450–600 s per bench.
That's still unreasonable for routine baseline captures. A62 wired
`benches/river.rs` to use **100 CFR+ iterations** for canonical +
wet-board and kept 1000 for the degenerate spot (which runs in
sub-microsecond per iteration). A64 **left that constant at 100**
because the flat+SIMD improvement was smaller than expected (see
"Why the jump is smaller than expected" below) — at ~4.35 ms/iter on
canonical the 1000-iter extrapolation is still ~4.35 s, which means
100-iter samples remain the right choice for routine runs.

Extrapolating the post-A64 per-iter numbers:
- canonical at 1000 iters ≈ 4.35 s (4.35× over the 1 s hard limit)
- wet-board at 1000 iters ≈ 5.89 s (~11.8× over the 500 ms target)

**Why A64's jump was smaller than expected (2026-04-23, A64 → A70
resolution).** The A64 integration wired `CfrPlusFlat` +
`regret_match_simd` through the river bench (and through solver-cli +
solver-ffi as the default solver). Pre-A64 expected: 3-9× gain,
driven by the 9× SIMD speedup measured on `regret_matching_scalar/1326`.
Post-A64 measured: 9-26 % gain per spot. The delta is **not** a bug —
it's the NLHE bet-tree shape.

**A70 closed this.** The fix was to restructure the CFR walk so the
1326-combo dimension becomes a SIMD lane instead of the action axis.
`CfrPlusVector` walks the action-only tree once per iteration (not
once per chance root as `CfrPlusFlat` did), carrying 1326-wide reach
vectors and integrating the showdown matrix via SIMD matmul. The
10-14× speedup on real river spots lands.

The SIMD path in `matching_simd.rs` has a `SIMD_THRESHOLD = 8`: for
action sets smaller than 8 it short-circuits to the scalar path,
because `f32x8` setup + horizontal-reduce overhead dominates the
actual arithmetic on tiny inputs. The NLHE v0.1 bet tree caps out at
**5 actions** per info set (check / bet-sizing × 3 / all-in), so the
SIMD branch *never fires* in the river inner loop. Every
`regret_match_simd` call on an NLHE river info set forwards to
`regret_match` (scalar) after the threshold check.

What we **did** get is the flat-array `RegretTables` speedup: the
~9-26% improvement per spot matches the "skip HashMap + skip Vec
pointer chase" cost on a tight regret-accumulation loop. That's a
real win, but it's roughly 10% — not 10×.

Where SIMD *would* fire:

1. **Vector CFR at the river.** If we iterate over the 1326 hero
   combos as a *vector* regret update (regrets-per-combo per info
   set) instead of looping per info set, each regret-matching call
   is N=1326 and the 9× SIMD speedup lands. This is the "vector CFR"
   layout that `docs/LIMITING_FACTOR.md` calls out as step #2. Not
   wired in A64 — left for v0.2.
2. **Preflop 169-grid.** At the preflop hand grid of 169 entries the
   SIMD path does fire; not a v0.1 hot path today.

v0.1 target (`river_canonical_spot < 300 ms @ 1000 iters`, which at
100 iters is `< 30 ms`) is **not met** by A64 alone. The remaining
~15× gap cannot come from scalar-sized regret matching; the only
paths to close it are (a) vector CFR with the combo axis inside
`regret_match_simd`, or (b) rayon-based tree-walk parallelism to use
all cores (A64 is single-threaded). Both are v0.2 work.

**Net:** A64 proves the flat-array integration is correct (Kuhn 10k
equivalence to 1e-6 holds, NLHE river still produces matching JSON),
and it delivers a real ~10-26% gain. The meaningful next optimization
is not more SIMD on the current layout — it's changing the layout so
the SIMD can bite.

### Inner-loop microbench (`regret_matching` — scalar only, Day 1 owns this file)

| Bench | Day 1 (scalar, baseline) | Target |
|---|---|---|
| `regret_matching_scalar/3`    | **2.24 ns**  | — |
| `regret_matching_scalar/8`    | **4.86 ns**  | — |
| `regret_matching_scalar/26`   | **16.08 ns** | — |
| `regret_matching_scalar/169`  | **154.70 ns** | — |
| `regret_matching_scalar/1326` | **1.74 µs**  | < 1 µs post-SIMD (A20) |

N=1326 is ~1.74 µs scalar — close to the SIMD target of < 1 µs, which is
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
| `cfr_plus_kuhn/10`                | **66.28 µs**  | ~151k iters/s |
| `cfr_plus_kuhn/100`               | **634.31 µs** | ~158k iters/s |
| `cfr_plus_kuhn/1000`              | **5.91 ms**   | ~169k iters/s |
| `cfr_plus_kuhn_single_iteration`  | **6.96 µs**   | — |

Linear scaling (10 → 100 → 1000 iters produces ~10× → ~100× wall time,
with the fixed "fresh solver + avg-strategy computation" overhead
visible at the 10-iter size). Good signal the tree walk is steady-state.

Note on the 2026-04-23 snapshot (commit `8e26e00`, agent A55): these
numbers were captured while an unrelated `cargo test -p solver-cli
--test e2e_integration` invocation was running on the same box, so the
Kuhn numbers may carry ~10–20 % core-contention overhead. The
`regret_matching` numbers above were captured first and are clean
(criterion "Change within noise threshold" at N=169 and N=1326). A
cleaner rerun may land a follow-up snapshot in `bench-history/` with
slightly better Kuhn numbers — keep the append-only history, don't
overwrite.

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
