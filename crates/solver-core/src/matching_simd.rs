//! SIMD-vectorized regret matching — the river hot-path optimization.
//!
//! This is the vectorized counterpart to [`crate::matching::regret_match`].
//! On the river inner loop we run regret matching on vectors of length
//! 1326 (one NLHE combo per lane) and the scalar loop is the throughput
//! bottleneck. An 8-wide SIMD path gives us most of the theoretical
//! speedup that `docs/LIMITING_FACTOR.md` calls for.
//!
//! # Why `wide` and not `std::simd`
//!
//! `std::simd` (the `portable_simd` feature, tracking issue rust-lang/rust
//! #86656) is still nightly-only on the pinned 1.85 stable toolchain this
//! workspace uses (`rust-toolchain.toml`). Henry's machine runs stable;
//! no nightly. `wide = "0.7"` gives us a portable `f32x8` that maps to
//! NEON on Apple Silicon and AVX on x86 (under `#[cfg(target_feature =
//! "avx")]`), with a safe scalar fallback elsewhere. That is everything
//! we need for the river inner loop.
//!
//! # Semantic equivalence with the scalar path
//!
//! The invariant that matters is: for every input `regrets`, the output
//! of `regret_match_simd` is element-wise within f32 rounding of
//! `regret_match`'s output. See `tests/simd_equivalence.rs` for the
//! full property sweep; the critical cases are
//!
//! - all non-positive → uniform `1 / n`,
//! - at least one positive → `out[i] = max(r[i], 0) / S` where `S` is
//!   the sum of positive entries,
//! - `NaN` regrets are treated as non-positive (`r > 0` with `NaN` is
//!   always false, so they contribute 0 to the sum and to the numerator
//!   — identical handling to the scalar path).
//!
//! Because `wide::f32x8::cmp_gt` returns a mask where `NaN > 0` is
//! `false` (matching the scalar `r > 0.0` semantics), a straight
//! "select-masked" approach gives exact element-wise parity on the NaN
//! edge. We deliberately avoid `simd_max(r, 0)` here — `max` on NaN is
//! IEEE-ambiguous and could propagate a NaN into the positive-sum.
//!
//! # Layout of the implementation
//!
//! 1. Tiny inputs (`n < 8`) short-circuit to the scalar path. SIMD setup
//!    overhead dominates for 3-action bet trees.
//! 2. Larger inputs do two passes: (a) compute the positive-sum across
//!    the input in 8-wide chunks + scalar tail; (b) write `out` with a
//!    masked division or the uniform fallback, again in 8-wide chunks +
//!    scalar tail.
//!
//! This matches the scalar path's two-pass structure exactly. The
//! summation order is *not* identical to the scalar path — we sum
//! 8 parallel partials and then horizontal-sum — but the observed drift
//! in the equivalence test at 2000-length inputs is well under the
//! `1e-6` tolerance the test enforces.

use wide::{f32x8, CmpGt};

use crate::matching::regret_match;

/// Below this length we just call the scalar path. Eight lanes of SIMD
/// on a 3-element action set is pure overhead.
const SIMD_THRESHOLD: usize = 8;

/// SIMD f32 regret matching. Equivalent to [`crate::matching::regret_match`]
/// within f32 rounding. Requires `regrets.len() == out.len()`; both must be
/// non-empty. No length restriction (handles tails scalar).
///
/// # Panics
///
/// Mirrors [`crate::matching::regret_match`]: panics if lengths differ or
/// if either slice is empty.
pub fn regret_match_simd(regrets: &[f32], out: &mut [f32]) {
    assert_eq!(
        regrets.len(),
        out.len(),
        "regret_match_simd: regrets and out must have the same length"
    );
    assert!(
        !regrets.is_empty(),
        "regret_match_simd: cannot operate on an empty action set"
    );

    // For tiny inputs the SIMD overhead dominates. Forward to scalar.
    // Also nicely handles n < 8 without special tail code.
    if regrets.len() < SIMD_THRESHOLD {
        regret_match(regrets, out);
        return;
    }

    let n = regrets.len();
    let zero8 = f32x8::splat(0.0);

    // --- Pass 1: sum of positive regrets ---------------------------------
    //
    // We want `sum_positive = sum_i max(r[i], 0)`. Using `max` directly is
    // risky with NaN — IEEE `max` is ambiguous on NaN — so we use a
    // mask-and-blend: where `r > 0.0`, include `r`; elsewhere include 0.
    // This matches the scalar path bit-for-bit on the NaN edge (scalar
    // tests `r > 0.0`, which is false for NaN, and contributes 0).
    let mut acc = f32x8::splat(0.0);
    let chunks = n / 8;
    let tail_start = chunks * 8;

    for c in 0..chunks {
        let base = c * 8;
        let slice: [f32; 8] = regrets[base..base + 8].try_into().unwrap();
        let v = f32x8::from(slice);
        let mask = v.cmp_gt(zero8);
        // `mask & v` sets non-positive lanes (including NaN) to +0.0.
        // This is the bit-exact equivalent of `if r > 0 { r } else { 0 }`
        // per the scalar loop.
        let masked = mask & v;
        acc += masked;
    }

    let mut sum_positive = acc.reduce_add();
    // Scalar tail (0..7 leftover elements).
    for &r in &regrets[tail_start..] {
        if r > 0.0 {
            sum_positive += r;
        }
    }

    // --- Pass 2: write output --------------------------------------------
    if sum_positive > 0.0 {
        // Proportional write.
        let inv = 1.0 / sum_positive;
        let inv8 = f32x8::splat(inv);

        for c in 0..chunks {
            let base = c * 8;
            let slice: [f32; 8] = regrets[base..base + 8].try_into().unwrap();
            let v = f32x8::from(slice);
            let mask = v.cmp_gt(zero8);
            // Same mask-and-blend trick. `(mask & v) * inv` writes
            // `r * inv` where `r > 0` and `0.0` elsewhere.
            let result = (mask & v) * inv8;
            let arr: [f32; 8] = result.into();
            out[base..base + 8].copy_from_slice(&arr);
        }
        // Scalar tail.
        for (o, &r) in out[tail_start..].iter_mut().zip(&regrets[tail_start..]) {
            *o = if r > 0.0 { r * inv } else { 0.0 };
        }
    } else {
        // Uniform fallback.
        let u = 1.0 / (n as f32);
        let u8v = f32x8::splat(u);
        let u8_arr: [f32; 8] = u8v.into();
        for c in 0..chunks {
            let base = c * 8;
            out[base..base + 8].copy_from_slice(&u8_arr);
        }
        for o in &mut out[tail_start..n] {
            *o = u;
        }
    }
}

/// Vector-CFR regret matching: combo-lane-major.
///
/// Input/output layout: `regrets[a]` and `out[a]` are parallel slices
/// holding **`N` combo-lane values** for action `a`. The caller passes
/// slice-of-slices (one per action); for each combo lane `i` we compute
/// the regret-matched probability of every action and write it back in
/// the same shape.
///
/// This is the primitive the v0.2 Vector CFR needs: at each decision
/// node of an NLHE river subgame, every hero combo has its own
/// regret-sum vector (one entry per action), and we want to normalize
/// all 1326 lanes in parallel.
///
/// # Semantics (per combo lane `i`)
///
/// Let `r[a] = regrets[a][i]` for `a` in `0..A`. Then:
///
/// - If any `r[a] > 0`, `out[a][i] = max(r[a], 0) / S` where `S` is the
///   positive-sum across actions.
/// - Otherwise, `out[a][i] = 1 / A` (uniform fallback).
///
/// Matches [`regret_match`] and [`regret_match_simd`] element-wise
/// per-lane; the only difference is the axis we SIMD across is the
/// **combo** dimension (length `N`), not the action dimension (length
/// `A`). That's the change that lets the 1326-wide combo dimension
/// actually hit the SIMD path on NLHE's 5-action bet tree.
///
/// # Panics
///
/// Panics if `regrets.len() != out.len()` (must both be `A`),
/// if `A == 0`, or if any slice has a different length (must all be
/// `N`).
pub fn regret_match_simd_vector(regrets: &[&[f32]], out: &mut [&mut [f32]]) {
    let a = regrets.len();
    assert!(
        a > 0,
        "regret_match_simd_vector: action axis must be non-empty"
    );
    assert_eq!(
        a,
        out.len(),
        "regret_match_simd_vector: regrets.len() and out.len() must match"
    );
    let n = regrets[0].len();
    assert!(
        n > 0,
        "regret_match_simd_vector: combo axis must be non-empty"
    );
    for slice in regrets.iter() {
        assert_eq!(
            slice.len(),
            n,
            "regret_match_simd_vector: all regret slices must have length {n}"
        );
    }
    for slice in out.iter() {
        assert_eq!(
            slice.len(),
            n,
            "regret_match_simd_vector: all out slices must have length {n}"
        );
    }

    let zero8 = f32x8::splat(0.0);
    let chunks = n / 8;
    let tail_start = chunks * 8;

    // --- Pass 1: accumulate sum_positive[lane] across actions -----------
    // We reuse the first output row as scratch for `sum_positive`. After
    // pass 2 we overwrite it with the strategy for action 0, so the
    // scratch lifetime stays within one call.
    //
    // SAFETY (soundness, not `unsafe`): `out[0]` is &mut, so writes are
    // exclusive. We write zeros first, then accumulate positive-regret
    // contributions, then overwrite with the final strategy in pass 2.
    {
        let s = &mut *out[0];
        for o in s.iter_mut() {
            *o = 0.0;
        }
    }

    // Accumulate `sum[i] += max(r[a][i], 0)` for each action `a`.
    //
    // We loop `for each action { for each chunk { ... } }` rather than
    // the reverse so that per-action data stays in cache: `regrets[a]`
    // is 1326 contiguous f32s (5.2 KB), fits in L1; the output scratch
    // (another 5.2 KB) also fits. Reading both sequentially once per
    // action is the cache-optimal layout on Apple Silicon's 128 KB L1D.
    //
    // We need a separate `sum_scratch` (not a re-read from out[0]) to
    // avoid the anti-dependency that would serialize the SIMD
    // accumulate across iterations. `out[0]` is used only for the final
    // readback.
    //
    // For cache-friendliness, do a two-pass structure:
    //   - Allocate one stack-resident f32x8 chunk at a time (one chunk
    //     = 8 f32s = 32 bytes).
    //   - For each chunk, sweep all actions with SIMD adds, then write
    //     the sum back to the scratch buffer.
    //
    // This flips the loop order relative to the naive "sum over actions
    // outer, chunks inner" — the action axis is inner. Since `A=5` is
    // tiny (fits in registers), this is a much better layout: each
    // chunk's 8 lanes stay in SIMD registers across the action sweep.
    for c in 0..chunks {
        let base = c * 8;
        let mut acc = zero8;
        for regrets_a in regrets.iter() {
            // SAFETY: base+8 <= 8*chunks <= n, guaranteed by the chunks
            // computation.
            let slice: [f32; 8] = regrets_a[base..base + 8].try_into().unwrap();
            let v = f32x8::from(slice);
            let mask = v.cmp_gt(zero8);
            acc += mask & v;
        }
        let arr: [f32; 8] = acc.into();
        out[0][base..base + 8].copy_from_slice(&arr);
    }
    for i in tail_start..n {
        let mut s = 0.0f32;
        for regrets_a in regrets.iter() {
            let r = regrets_a[i];
            if r > 0.0 {
                s += r;
            }
        }
        out[0][i] = s;
    }

    // --- Pass 2: write strategy, reading sum_positive from out[0] ------
    //
    // For lanes where sum_positive > 0: `out[a][i] = max(r[a][i], 0) *
    // inv_sum[i]`. For lanes where sum_positive == 0: uniform 1/A.
    //
    // We must compute action 0's output AFTER we're done reading out[0]
    // as the sum. Do pass 2 action-by-action, reverse order, so action
    // 0 is written last.

    // Compute `1/sum_positive` or the uniform-fallback constant into
    // the scratch. After this, out[0] holds the per-lane multiplier;
    // uniform-fallback lanes are detected by the original sum being 0.
    //
    // We build a second scratch (on stack-only by keeping chunks local)
    // that holds both the `inv` and the `is_zero` mask. Simpler: keep
    // one scratch array of `inv = sum > 0 ? 1/sum : 1/A` values; for
    // non-uniform lanes, `max(r[a], 0) * inv` is correct, and for
    // uniform lanes, `max(r[a], 0) * inv = 0 * (1/A) = 0`, which is
    // wrong — we need `1/A` regardless of r[a].
    //
    // Cleanest: for each action, a lane where sum_positive==0 gets
    // `1/A`, otherwise `max(r[a], 0) / sum_positive`. We handle the two
    // cases by letting pass 2 read the old `sum_positive` value per
    // lane, not a mutated `inv`, and do the branch per lane.

    // We need to write action `0`'s output last because we're reusing
    // out[0] as the sum scratch. Write a..1 first, then 0 last. At the
    // point we overwrite out[0], we've already consumed it.
    //
    // Since the loop over actions accesses `out[a]` mutably, we cannot
    // safely borrow out[0] AND out[1..] simultaneously through the
    // slice-of-slices. Work around with indexing on a single mutable
    // borrow of the outer slice.

    let uniform = 1.0 / (a as f32);
    for action_idx in (0..a).rev() {
        let r_a = regrets[action_idx];
        if action_idx == 0 {
            // Action 0: must read sum from out[0] BEFORE overwriting it.
            // Copy sum into a stack-resident vector first. We reuse the
            // slice by using a single pass that reads and writes from
            // the same slice — since we never write out[0][i] before
            // reading out[0][i] for the same i within one step, this is
            // safe.
            for c in 0..chunks {
                let base = c * 8;
                // Read sum lanes first (scratch value).
                let sum_slice: [f32; 8] = out[0][base..base + 8].try_into().unwrap();
                let sum_v = f32x8::from(sum_slice);
                let sum_pos_mask = sum_v.cmp_gt(zero8);

                let r_slice: [f32; 8] = r_a[base..base + 8].try_into().unwrap();
                let r_v = f32x8::from(r_slice);
                let r_pos_mask = r_v.cmp_gt(zero8);
                let r_pos = r_pos_mask & r_v;

                // max(r, 0) * (1/sum) for lanes with sum>0. We compute
                // reciprocal per-lane; `wide::f32x8` has no blend, so use
                // masked pieces.
                //
                // For lanes with sum>0: strategy = r_pos / sum
                // For lanes with sum==0: strategy = 1/A (uniform)
                //
                // blend(mask, a, b) = (mask & a) | (!mask & b)
                let inv_sum = f32x8::splat(1.0) / sum_v;
                // In lanes where sum == 0, inv_sum is inf (or NaN for
                // negative sums, but sum >= 0 by construction). Clear
                // those lanes with the mask.
                let prop = r_pos * inv_sum;
                let uniform_v = f32x8::splat(uniform);
                // Build the final via masked select:
                //   final = (sum_pos_mask & prop) | (!sum_pos_mask & uniform_v)
                let positive_part = sum_pos_mask & prop;
                let uniform_part = !sum_pos_mask & uniform_v;
                let result = positive_part | uniform_part;
                let arr: [f32; 8] = result.into();
                out[0][base..base + 8].copy_from_slice(&arr);
            }
            for (i, r) in r_a.iter().enumerate().take(n).skip(tail_start) {
                let sum = out[0][i];
                out[0][i] = if sum > 0.0 {
                    if *r > 0.0 {
                        r / sum
                    } else {
                        0.0
                    }
                } else {
                    uniform
                };
            }
        } else {
            // For later actions we still need sum_positive per lane,
            // which lives in out[0]. Read it without mutating.
            //
            // Split the borrow: get a &mut slice on action_idx row and
            // a &slice on row 0.
            let (row0, rest) = out.split_at_mut(1);
            let sum_scratch: &[f32] = row0[0];
            let write_row: &mut [f32] = rest[action_idx - 1];

            for c in 0..chunks {
                let base = c * 8;
                let sum_slice: [f32; 8] = sum_scratch[base..base + 8].try_into().unwrap();
                let sum_v = f32x8::from(sum_slice);
                let sum_pos_mask = sum_v.cmp_gt(zero8);

                let r_slice: [f32; 8] = r_a[base..base + 8].try_into().unwrap();
                let r_v = f32x8::from(r_slice);
                let r_pos_mask = r_v.cmp_gt(zero8);
                let r_pos = r_pos_mask & r_v;

                let inv_sum = f32x8::splat(1.0) / sum_v;
                let prop = r_pos * inv_sum;
                let uniform_v = f32x8::splat(uniform);
                let positive_part = sum_pos_mask & prop;
                let uniform_part = !sum_pos_mask & uniform_v;
                let result = positive_part | uniform_part;
                let arr: [f32; 8] = result.into();
                write_row[base..base + 8].copy_from_slice(&arr);
            }
            for i in tail_start..n {
                let sum = sum_scratch[i];
                let r = r_a[i];
                write_row[i] = if sum > 0.0 {
                    if r > 0.0 {
                        r / sum
                    } else {
                        0.0
                    }
                } else {
                    uniform
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Unit-level sanity checks. The heavy lifting — 10 000-trial
    //! equivalence sweep vs the scalar path — lives in the separate
    //! integration test at `tests/simd_equivalence.rs`. These tests here
    //! exist to catch dumb-mistake regressions at `cargo test -p
    //! solver-core --lib` (which runs much faster than the full
    //! integration suite) and to document the expected shapes inline.

    use super::*;
    use crate::matching::regret_match_vec;

    fn simd_vec(regrets: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0f32; regrets.len()];
        regret_match_simd(regrets, &mut out);
        out
    }

    #[test]
    fn tiny_input_matches_scalar() {
        let regrets = [1.0f32, -2.0, 3.0];
        let s = simd_vec(&regrets);
        let expected = regret_match_vec(&regrets);
        for (a, b) in s.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn length_eight_hits_simd_path() {
        let regrets = [1.0f32, -1.0, 2.0, -2.0, 3.0, -3.0, 4.0, -4.0];
        let s = simd_vec(&regrets);
        let expected = regret_match_vec(&regrets);
        for (a, b) in s.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn length_nine_hits_tail_path() {
        let regrets = [1.0f32, -1.0, 2.0, -2.0, 3.0, -3.0, 4.0, -4.0, 5.0];
        let s = simd_vec(&regrets);
        let expected = regret_match_vec(&regrets);
        for (a, b) in s.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn all_negative_uniform_simd() {
        let regrets = [-1.0f32; 16];
        let s = simd_vec(&regrets);
        let expected = 1.0 / 16.0;
        for &p in &s {
            assert!((p - expected).abs() < 1e-6);
        }
    }

    #[test]
    #[should_panic(expected = "empty action set")]
    fn empty_panics() {
        let mut out: [f32; 0] = [];
        regret_match_simd(&[], &mut out);
    }

    #[test]
    #[should_panic(expected = "same length")]
    fn mismatched_lengths_panic() {
        let regrets = [1.0f32; 8];
        let mut out = [0.0f32; 9];
        regret_match_simd(&regrets, &mut out);
    }

    // --- Vector-CFR primitive tests -------------------------------------

    /// Compare `regret_match_simd_vector`'s output against the scalar
    /// `regret_match` applied per-combo lane, entrywise within 1e-6.
    fn assert_vector_matches_scalar_per_lane(regrets: &[Vec<f32>], n: usize) {
        let a = regrets.len();
        let refs: Vec<&[f32]> = regrets.iter().map(|v| v.as_slice()).collect();
        let mut out_buf: Vec<Vec<f32>> = (0..a).map(|_| vec![0.0f32; n]).collect();
        {
            // Need a Vec<&mut [f32]> — borrow each row exclusively.
            let mut out_refs: Vec<&mut [f32]> =
                out_buf.iter_mut().map(|v| v.as_mut_slice()).collect();
            regret_match_simd_vector(&refs, &mut out_refs);
        }

        // Scalar per-lane reference.
        for i in 0..n {
            let lane_r: Vec<f32> = regrets.iter().map(|v| v[i]).collect();
            let lane_expected = regret_match_vec(&lane_r);
            for (a_idx, out_row) in out_buf.iter().enumerate().take(a) {
                let got = out_row[i];
                let want = lane_expected[a_idx];
                let diff = (got - want).abs();
                assert!(
                    diff < 1e-6,
                    "lane {i} action {a_idx}: got {got}, want {want}, diff {diff}"
                );
            }
        }
    }

    #[test]
    fn vector_matches_scalar_per_lane_small() {
        // N=16, A=3: hand-rolled regrets with mixed positive/negative lanes.
        let n = 16;
        let a = 3;
        let mut regrets: Vec<Vec<f32>> = vec![vec![0.0; n]; a];
        for i in 0..n {
            regrets[0][i] = (i as f32) * 0.5 - 3.0;
            regrets[1][i] = (n - i) as f32 * 0.3 - 2.0;
            regrets[2][i] = if i % 3 == 0 { 1.0 } else { -0.5 };
        }
        assert_vector_matches_scalar_per_lane(&regrets, n);
    }

    #[test]
    fn vector_matches_scalar_per_lane_nlhe_scale() {
        // N=1326, A=5: the NLHE river hot-path shape.
        use rand::{Rng, SeedableRng};
        use rand_xoshiro::Xoshiro256PlusPlus;
        let mut rng = Xoshiro256PlusPlus::from_seed([7; 32]);
        let n = 1326;
        let a = 5;
        let regrets: Vec<Vec<f32>> = (0..a)
            .map(|_| (0..n).map(|_| rng.gen_range(-2.0f32..2.0)).collect())
            .collect();
        assert_vector_matches_scalar_per_lane(&regrets, n);
    }

    #[test]
    fn vector_all_negative_is_uniform() {
        // Every action has a negative regret at every lane — all lanes
        // fall back to uniform 1/A.
        let n = 32;
        let a = 4;
        let regrets: Vec<Vec<f32>> = (0..a).map(|_| vec![-1.0f32; n]).collect();
        let refs: Vec<&[f32]> = regrets.iter().map(|v| v.as_slice()).collect();
        let mut out_buf: Vec<Vec<f32>> = (0..a).map(|_| vec![0.0f32; n]).collect();
        {
            let mut out_refs: Vec<&mut [f32]> =
                out_buf.iter_mut().map(|v| v.as_mut_slice()).collect();
            regret_match_simd_vector(&refs, &mut out_refs);
        }
        let expected = 1.0 / (a as f32);
        for row in &out_buf {
            for &p in row {
                assert!((p - expected).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn vector_pure_strategy_when_one_action_positive_per_lane() {
        // Lane i: only action (i % A) is positive, rest are negative.
        let n = 40;
        let a = 5;
        let mut regrets: Vec<Vec<f32>> = vec![vec![-1.0f32; n]; a];
        for i in 0..n {
            regrets[i % a][i] = 3.7;
        }
        let refs: Vec<&[f32]> = regrets.iter().map(|v| v.as_slice()).collect();
        let mut out_buf: Vec<Vec<f32>> = (0..a).map(|_| vec![0.0f32; n]).collect();
        {
            let mut out_refs: Vec<&mut [f32]> =
                out_buf.iter_mut().map(|v| v.as_mut_slice()).collect();
            regret_match_simd_vector(&refs, &mut out_refs);
        }
        for i in 0..n {
            let active = i % a;
            for (a_idx, out_row) in out_buf.iter().enumerate().take(a) {
                let got = out_row[i];
                let want = if a_idx == active { 1.0 } else { 0.0 };
                assert!(
                    (got - want).abs() < 1e-6,
                    "lane {i} action {a_idx}: got {got}, want {want}"
                );
            }
        }
    }

    #[test]
    fn vector_sums_to_one_per_lane() {
        // Property: the strategy at every lane must sum to 1.0 across
        // actions.
        use rand::{Rng, SeedableRng};
        use rand_xoshiro::Xoshiro256PlusPlus;
        let mut rng = Xoshiro256PlusPlus::from_seed([11; 32]);
        let n = 169;
        let a = 7;
        let regrets: Vec<Vec<f32>> = (0..a)
            .map(|_| (0..n).map(|_| rng.gen_range(-1.0f32..1.0)).collect())
            .collect();
        let refs: Vec<&[f32]> = regrets.iter().map(|v| v.as_slice()).collect();
        let mut out_buf: Vec<Vec<f32>> = (0..a).map(|_| vec![0.0f32; n]).collect();
        {
            let mut out_refs: Vec<&mut [f32]> =
                out_buf.iter_mut().map(|v| v.as_mut_slice()).collect();
            regret_match_simd_vector(&refs, &mut out_refs);
        }
        for i in 0..n {
            let s: f32 = (0..a).map(|a_idx| out_buf[a_idx][i]).sum();
            assert!(
                (s - 1.0).abs() < 1e-5,
                "lane {i}: strategy sum {s}, expected 1.0"
            );
        }
    }
}
