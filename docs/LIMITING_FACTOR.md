# The limiting factor

**Convergence speed on the river inner loop.** Every architectural
decision in v0.1 bends toward this.

## Why this is THE critical path

1. The river is where the overlay actually renders live. Turn and flop
   are cached; preflop is static. Live solving happens on the river.
2. Hands last 30–90 seconds. An overlay that takes 2 seconds to render
   feels broken. An overlay that takes 300 ms feels magic.
3. Every other solver task (turn, flop, cache lookup, FFI) is tractable
   and reasonably scoped. Only the river is "can we even make this fast
   enough."
4. If the river is fast, everything downstream works. If it's not, we
   restructure the whole product around precompute (slower ship, more
   data plumbing, higher operational cost).

## The shape of the inner loop

For each CFR+ iteration on a river subgame:

```
for each info set I at each decision node:
    for each action a in legal_actions(I):
        for each hero combo i (up to 1326):
            compute action_util[a][i]          (matrix-vector)
            accumulate regret_sum[I][a][i]
            accumulate strategy_sum[I][a][i]
```

The innermost loop over 1326 combos is the hot path. At a river with a
typical 3-action bet tree and ~10 nodes, that's:
`1000 iters × 10 nodes × 3 actions × 1326 combos = ~40M inner-loop ops`.

Each op is ~10 flops (f32 multiply-add, max, division). Total: ~400M
flops per solve. An M1 Pro does ~2 TFLOPs/s. **Theoretical lower bound:
200 µs.** We will not hit theoretical, but the gap between 200 µs and
300 ms is where our optimization lives.

## Optimization ladder (in order of attempt)

### 1. Cache-friendly data layout (Day 2)

Default: `regret_sum: Vec<Vec<Vec<f32>>>` — heap-chasing hell.

Fix: pack everything into flat arrays keyed by info-set index + action
index + combo index. Stride-friendly so the inner loop is contiguous
memory access.

```rust
// Instead of Vec<Vec<Vec<f32>>>:
struct SubgameRegrets {
    data: Box<[f32]>,   // length = n_info_sets * max_actions * 1326
    strides: [usize; 3],
}
```

Expected speedup vs naive Vec nesting: **3–5×**.

### 2. SIMD inner loop (Day 3)

Default: scalar f32 loops.

Fix: `std::simd::f32x8` for the combo dimension. 8-wide regret updates,
8-wide strategy normalization, 8-wide positive-regret clamps.

```rust
use std::simd::{f32x8, num::SimdFloat};

for i in (0..1326).step_by(8) {
    let util = f32x8::from_slice(&action_utils[i..]);
    let node = f32x8::splat(node_util[i_chunk]);
    let regret = util - node;
    let acc = f32x8::from_slice(&regrets[i..]) + regret;
    acc.max(f32x8::splat(0.0)).write_to_slice(&mut regrets[i..]);
}
```

Expected speedup: **6–8×** (8-wide ops, with some overhead).

### 3. Parallelize across info sets (Day 3)

Default: serial over info sets.

Fix: `rayon::par_iter` the outer loop. Each info set is independent within
an iteration.

Expected speedup on M1 Pro (10 cores): **5–7×** (not 10× due to Amdahl
and memory bandwidth saturation).

### 4. Metal compute shader (Day 4, conditional)

Default: Rust SIMD + rayon.

Fix: compute kernel in Metal, launched from FFI layer. Entire 1326-combo
update happens in one threadgroup. Unified memory means zero copy cost.

Expected speedup vs Rust SIMD: **3–10×**.

Only done if post-optimization Rust is > 500 ms on M1 Pro.

### 5. Vector CFR reformulation (already in algorithm choice)

This is at the algorithm level, not a low-level optimization. At river
we express the whole update as matrix-vector ops on 1326-wide vectors.
This is the architectural reason the lower-level SIMD/Metal work is even
possible.

## Measuring

**`cargo bench -p solver-core`** is the truth. The bench suite runs:

1. `river_canonical_spot` — 1000 iters on a known spot, report mean +
   std dev over 30 runs
2. `river_degenerate_spot` — all-in preflop, super narrow ranges
3. `river_wet_board` — JhTh9c-style texture
4. `regret_matching_inner` — pure inner-loop microbench

All numbers reported in µs and iters/sec. A commit that regresses any of
these by >5% needs justification.

## What we do if we miss < 1 s

Option A: **Precompute the river too.** Turn the live path into a cache
lookup. Cache size for river subgames is ~100 GB if exhaustive; we'd ship
a texture-bucketed subset at ~10 GB.

Option B: **Lower iteration count.** 500 iters instead of 1000 halves
latency at the cost of ~0.5% exploitability. Probably imperceptible on an
overlay.

Option C: **Ship with Metal path only.** Drop Intel/Rosetta support for
v0.1. Force Apple Silicon minimum.

Option A is the most robust. Options B and C trade quality/compatibility
for ship date.

## Validation

Any performance claim must be backed by a criterion run:

```bash
cd ~/Desktop/poker-solver
cargo bench -p solver-core -- river_canonical_spot

# criterion output:
# river_canonical_spot    time:   [285.3 µs 287.1 µs 289.4 µs]
#                         thrpt:  [3455 iters/sec 3484 iters/sec 3505 iters/sec]
```

Paste that output into the PR description / commit message. "I think it's
faster" doesn't count.

## After v0.1

v0.2 optimization priorities (in order):

1. Discounted CFR+ instead of CFR+ (faster convergence, no performance cost)
2. Multi-core turn solver (rayon over chance nodes)
3. Metal on turn (not just river)
4. Better board-texture bucketing (smaller cache, higher hit rate)
5. Warm-starting from similar cached spots

Each is a week of work, each cuts solve time by 1.5–3×. Compounds well.
