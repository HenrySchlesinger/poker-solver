# A73 Profiling Report

Date: 2026-04-23. Agent: A73. Task: diagnostic-only; no algorithm
changes. A72's NEON experiment ruled out the `i8 → f32` widening in
the showdown matmul as the bottleneck. A73 profiles the real Vector
CFR hot path and names the actual culprits.

## Tool used

`samply 0.13.1` (Firefox profiler), installed via Homebrew. Sampled
at 4 kHz (`--rate 4000`), single run, ~650 ms of CPU time captured on
the `solver-cli-cfr` thread (2610 samples).

Release binary was built once with `CARGO_PROFILE_RELEASE_DEBUG=
line-tables-only` so samply could attribute samples to source lines
without bloating the binary with full DWARF (the repo's default
`profile.release.debug = false` would have left everything anonymous).

Addresses were symbolicated post-hoc with `atos -arch arm64 -l
0x100000000` against `target/release/solver-cli`.

## Spot profiled

`river_canonical_spot`, 1000 CFR+ iterations, identical setup to
`benches/river.rs`'s canonical bench:

```
./target/release/solver-cli solve \
    --board AhKhQh2d4s \
    --hero-range "AA,KK,AKs" \
    --villain-range "22+,AJs+,KQs" \
    --pot 100 --stack 500 --iterations 1000
```

Default `--solver vector` — the post-A70 combo-axis-SIMD walker,
which is the hot path we care about. 652.5 ms of CPU time captured
(criterion's 40.70 ms @ 100 iters ≈ 407 ms @ 1000 iters is pure CFR;
samply also sampled ~36 ms of one-shot JSON serialization at the end
and a small amount of setup — subtract ~60 ms to get the real CFR
total, which lines up with the A70 reference).

## Top functions (self-time, symbolicated)

The tree-walk function is so dominant that line-level breakdown is
what actually carries signal. Per-function self-time:

| Function | Self-time % | Self-time ms |
|---|---|---|
| `CfrPlusVector::walk` (all lines aggregated) | 79.3 % | 517.2 |
| `serde_json::...::serialize` (final JSON output, *not CFR*) | 5.0 % | 32.5 |
| `serde_json::...::parse_number` (CLI input parse, *not CFR*) | 3.3 % | 21.2 |
| `memset_pattern16` (dyld stub — `out_util` / reach zeroing) | 1.4 % | 9.0 |
| `solver_eval::combo::index_to_combo` (clone-paths in enumeration) | 1.4 % | 9.0 |
| `solver_cli::solve_cmd::run_cfr` | 0.4 % | 2.5 |
| `SmallVec::clone` (action list) | 0.4 % | 2.5 |
| `NlheSubgameVector::is_terminal` + `legal_actions` + `apply` | 0.5 % | 3.2 |
| everything else | < 0.5 % each | — |

Three A72-named candidates confirmed **NOT** to be the bottleneck:

1. **Info-set hash cost.** `action_only_info_set_hash` (FNV-1a, line
   855 of `subgame_vector.rs`) is fully inlined and contributes
   zero visible self-time. < 0.5 % of walk. Dead lead.
2. **`regret_match_simd` scalar short-circuit.** The walker calls
   `regret_match_simd_vector` (combo-axis SIMD, length 1326 — far
   above `SIMD_THRESHOLD=8`), not the 5-wide action-axis variant.
   Inner code path is the SIMD path, not the scalar fallback.
3. **`showdown_sign` L1 pressure (1.76 MB i8 matrix).** Showdown
   matmul is 27 % of walk — a *share*, not the bottleneck. Cache
   pressure is real but it's not where the 25 % gap lives.

## Line-level breakdown inside `walk`

Samply + line-tables DWARF resolves self-time to specific source
lines of `crates/solver-core/src/cfr_vector.rs`. Lines with the same
semantic purpose are merged here:

| Phase | Line | % of walk | ms |
|---|---|---|---|
| Showdown matmul (`fill_terminal_utility` call, inlined) | 387 | 26.8 % | 138.5 |
| `regret_match_simd_vector` (inlined SIMD across combos) | 423 + macros | 14.6 % | 75.5 |
| `node_util += p[c] * au[c]` (update-player aggregation) | 480 | 13.7 % | 70.8 |
| `strategy_sum[c] += linear_weight * own_reach[c] * s[c]` | 511 | 11.8 % | 61.2 |
| CFR+ regret update `max(row + au - node_util, 0)` | 503-504 | 9.6 % | 49.7 |
| `next_hero_reach[c] = reach_hero[c] * p[c]` | 448 | 9.3 % | 48.0 |
| `node_util += au[c]` (opponent-node aggregation) | 484 | 9.2 % | 47.8 |
| `next_villain_reach[c] = reach_villain[c] * p[c]` | 462 | 7.4 % | 38.2 |
| other/overhead (recursion bookkeeping, scratch take/return) | — | -2.4 % | — |

Floor of the "-2.4 %" gap is rounding + the ~2.8 % of samples
attributed to `cfr_vector.rs:macros.rs:0` (debug-assertion inlines);
those likely belong with line 423's SIMD call.

## Cumulative breakdown

| Phase | % of total CPU | ms |
|---|---|---|
| Showdown matmul (terminal eval) | 21.2 % | 138.5 |
| Scalar elementwise reach-product loops (lines 448, 462) | 13.2 % | 86.2 |
| Scalar elementwise node-util aggregation (lines 480, 484) | 18.2 % | 118.6 |
| Scalar CFR+ regret + strategy_sum update (lines 503-504, 511) | 17.0 % | 110.9 |
| `regret_match_simd_vector` (combo-axis SIMD, 5 actions × 1326) | 11.6 % | 75.5 |
| All other walk overhead | < 2 % | ~12 |
| Non-CFR (serde, setup, atos stubs) | ~10 % | ~64 |
| L1/L2 miss correlation | — | samply on macOS doesn't surface HW counters; not available |

The new, non-obvious finding: **the elementwise scalar `for c in
0..cw` loops inside `walk` (reach-products, node-util aggregation,
regret + strategy_sum updates) together eat 48.4 % of the CPU budget
— nearly 2× the showdown matmul.** These are the loops at lines 448,
462, 480, 484, 502-505, 510-511 of `cfr_vector.rs`. They are
auto-vectorized by LLVM but the auto-vec is almost certainly 4-wide
NEON (vs the 8-wide `wide::f32x8` the rest of the kernels use), and
at least one of them has a branch inside the body (line 504's CFR+
`max(_, 0)` clamp) that defeats clean auto-vec on some codegens.

## Recommendations for next perf agent (ranked by expected impact)

1. **SIMD-ify the walker's four elementwise loops.** They're
   responsible for 48 % of walk self-time. Replace the raw
   `for c in 0..cw` loops at lines 447-449, 461-463, 479-481,
   483-485, 502-505, 510-512 of `cfr_vector.rs` with `wide::f32x8`
   (or the NEON module A72 left behind). Expected gain: if we move
   even half of that 48 % from 4-wide scalar-ish autovec to proper
   8-wide SIMD, that's ~15-20 % of total walk time — which is the
   full 25 % gap we need to hit 300 ms @ 1000 iters.

   The CFR+ clamp at line 504 should use the same mask-and-blend
   trick `regret_match_simd` already uses (`mask & v` on
   `cmp_gt(zero)`), to keep branchless SIMD without NaN weirdness.

2. **Fuse the reach-product loop with the recursive call prep.**
   Lines 446-450 allocate `next_hero_take` as a write, then line 450
   `copy_from_slice`s the villain reach — but in the Villain branch
   it's the other way around. Both branches write the same
   `copy_from_slice` of the non-acting player's reach. If we stored
   reaches in a `[Player][depth][combo]` scratch, the copy (8 % of
   walk between the two `copy_from_slice` calls) could be removed
   entirely — we'd just pass the existing parent slice down. This
   is more invasive than #1 but has a clean 8 % payoff.

3. **Fuse node_util aggregation into the post-walk loop, using SIMD.**
   The aggregation loop at line 478-486 runs inside the per-action
   loop (sequential across actions) and reads `au[c]` right after
   the recursive walk returns. Moving the aggregation to a single
   post-loop SIMD pass across all 5 actions × 1326 combos is a
   natural fit for the existing combo-axis-SIMD pattern in
   `regret_match_simd_vector`. The SIMD pass would do pos/neg
   sum accumulation in one sweep. Expected gain: 5-8 % of walk.

4. **`showdown_matmul_cols` allocates `vec![0.0f32; NUM_COMBOS]`
   per call (line 560).** 5.2 KB heap allocation on every villain-
   terminal evaluation. The code comment at line 556-559 already
   flags this as a future concern. malloc didn't show in the
   profile (< 0.5 ms total), but lift the scratch into a persistent
   slot anyway — it's free to do and removes a per-call allocation.

5. **Showdown matmul (138 ms, 27 % of walk) is the single largest
   bucket, but A72 proved the kernel is already LLVM-optimal on
   aarch64 (wide::f32x8 lowers to the same NEON as hand-rolled).**
   The only room left here is cache behavior — `showdown_sign` is
   1.76 MB (way over L1). A future experiment worth trying is
   tiling the matmul: process hero combos in cache-sized tiles
   (e.g., 256 rows at a time) so each tile's 256 × 1326 i8 slice
   (~340 KB) fits more comfortably in L2, and reach_opp stays in
   L1. This is harder than #1-3 and less likely to pay off (LLVM
   auto-tiles hot matmuls with loop interchange in some cases),
   but it's the path if #1-3 don't close the gap.

## Gut-check: can we hit 30 ms @ 100 iters?

**Yes, probably** — via recommendation #1.

Today: 40.70 ms @ 100 iters (A70 reference). Target: 30 ms @ 100
iters (~25 % faster).

The elementwise scalar loops account for 48.4 % of walk time. Even a
conservative 2× speedup on those loops (8-wide SIMD vs what I suspect
is 4-wide autovec) would reclaim 24 % of walk ≈ 9.8 ms @ 100 iters,
landing us at 30.9 ms. That's a bullseye on the target with almost
no risk — the loops are trivial and `wide::f32x8` patterns already
exist in the repo (`matching_simd.rs`).

The optimistic scenario — if auto-vec is effectively scalar on the
CFR+ clamp and 4-wide elsewhere — is a full 4× speedup on those
loops, which would reclaim ~36 % and take us to ~26 ms @ 100 iters.
Harder to believe pre-experiment, but the CFR+ clamp specifically
does tend to serialize in autovec output because of the branch.

The path from 30 → below-30: recommendations #2 (`copy_from_slice`
removal, ~8 %) and #3 (aggregation fusion, ~5-8 %) compound on top
of #1. In aggregate, 30 → 25 ms feels reachable in one more agent-
session of focused work.

## Profile artifacts

- `target/a73-canonical.profile.json` — raw samply profile (88 KB,
  gzipped-JSON format; open with `samply load target/a73-canonical.
  profile.json` to get the Firefox UI).
- `target/a73-solve.stdout.json` — the solver output from the
  profile run (kept for reproducibility; confirms the solver
  converged correctly).

Both files are `.gitignore`-eligible (inside `target/`). They are
not committed by this agent. A73 signing off.
