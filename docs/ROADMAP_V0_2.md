# v0.2 Roadmap

**Tag target:** 2026-05-13 (two weeks post-v0.1). Synthesizes findings
from [A61 ship audit](SHIP_V0_1.md), [A64 perf diagnosis](BENCHMARKS.md),
[A66 exploitability triage](EXPLOITABILITY_TRIAGE.md), and the standing
[limiting-factor analysis](LIMITING_FACTOR.md).

## Goals for v0.2

1. **Hit the v0.1 perf target that v0.1 shipped short of:**
   `river_canonical_spot < 300 ms @ 1000 iters` on M1 Pro (currently ~4.35 s
   extrapolated post-A64).
2. **Fix exploitability reporting** with a chance-layer-aware BR.
3. **Populate the real flop cache** from the Colab precompute pipeline
   (v0.1 ships a 374 KB format-only placeholder).
4. **Extend TexasSolver diff to 20/20 fixtures** with an auto-diff
   harness (v0.1 ships 1/20).

## Workstreams

### A. Vector CFR — combo-axis SIMD

- **Blocking:** `regret_match_simd` short-circuits to scalar below
  `SIMD_THRESHOLD = 8`; the NLHE bet tree caps at 5 actions, so SIMD never
  fires on the river hot path. See [BENCHMARKS §"Why the jump is smaller
  than expected"](BENCHMARKS.md).
- **Fix:** restructure `CfrPlusFlat::walk` to vectorize across the
  **1326 hero combos** per info set instead of across actions. Action
  dimension stays scalar (it's small and branchy); combo dimension
  becomes the 8-wide SIMD lane. This is the layout change
  [LIMITING_FACTOR.md §5](LIMITING_FACTOR.md) always called "vector CFR"
  and is where the 9× `regret_match_simd` speedup measured on N=1326
  actually lands.
- **Files:** `crates/solver-core/src/cfr_flat.rs`; a new
  `crates/solver-core/src/vector_cfr.rs` is likely.
- **Effort:** 3–5 days.
- **Risk:** **medium.** Changes inner-loop semantics; must preserve
  Kuhn convergence to 1e-6 (`tests/flat_equivalence.rs`) and produce
  matching action frequencies on the 20-spot TS battery.

### B. Chance-layer-aware exploitability

- **Blocking:** `CfrPlus::exploitability()` and
  `CfrPlusFlat::exploitability()` walk the phantom `initial_state()`
  (combos 0 and 1 — which share `Card 0`, an illegal deal) instead of
  `NlheSubgame::chance_roots()`. Full diagnosis in
  [EXPLOITABILITY_TRIAGE.md](EXPLOITABILITY_TRIAGE.md).
- **Fix:** add `exploitability_over_roots(&game, &strategy, &roots)` to
  `convergence.rs`; expose `exploitability_from(&roots)` on both
  `CfrPlus` and `CfrPlusFlat`; wire CLI and FFI to call it; convert the
  reported unit to a fraction of pot at the CLI boundary per
  `REQUIREMENTS.md:59`; deprecate the no-args `.exploitability()`.
  Regression tests: royal-tie `< 0.01 × pot` at 1000 iters;
  `spot_018 < 5 × TS's reported 0.28%`.
- **Files:** `crates/solver-core/src/convergence.rs`, `cfr.rs`,
  `cfr_flat.rs`; `crates/solver-cli/src/solve_cmd.rs`;
  `crates/solver-ffi/src/lib.rs`. ~80 lines across 3 crates.
- **Effort:** 1 day.
- **Risk:** **low.** Triage already specified the patch.

### C. Colab flop-cache population

- **Blocking:** `data/flop-cache/flop-cache-v0.1.bin` is 374 KB of 36
  placeholder entries ([SHIP_V0_1.md Data section](SHIP_V0_1.md)); the
  loader, format, and pack tooling are all proven.
- **Fix:** run `colab/precompute_flops.ipynb` on Colab free-tier to
  generate canonical-flop strategies for 1755 strategically distinct
  flops × SPR buckets × pot types; pack with the `pack-cache` subcommand;
  ship `flop-cache-v0.2.bin`. Bump the cache format version gate so a
  v0.1 consumer refuses the new file.
- **Blocker that is not a code change:** Henry must kick off the Colab
  notebooks.
- **Effort:** 3–7 days of Colab wall-time; minutes of setup.
- **Risk:** **low** (pipeline dry-run-verified by A49).

## Stretch (if time)

- Rayon multi-thread CFR tree walk (the other path to closing the
  river perf gap).
- Metal batched-kernel for 1326×1326 matrix-equity — A51 showed Metal
  only wins at batched scale.
- Warm-starting river subgames from cached flop entries.
- Exploitative opponent modeling (bet sizes off the default tree).

## Non-goals for v0.2

- PLO / Omaha (different combo space and evaluator).
- Tournament ICM math.
- Multi-way (3+ player) — first-class target for v0.3.

## Parallelism

Workstream A is the longest pole. B and C run in parallel with it (B is
a one-day solo, C is mostly wall-clock). Tag date **2026-05-13**
assumes A lands by day 10 and B + C slot into the remaining buffer.
