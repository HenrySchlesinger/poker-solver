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
}
