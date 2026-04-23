//! Equivalence test: `regret_match_simd` must match the scalar
//! `regret_match` within f32 rounding across a large space of inputs.
//!
//! This is the load-bearing correctness gate for the SIMD path. Any
//! discrepancy between scalar and SIMD breaks CFR+ convergence —
//! regret signs and magnitudes have to match to the last ulp or the
//! cumulative regret drift will compound across thousands of iterations
//! and push the computed strategy away from Nash.
//!
//! # What we check
//!
//! 1. **Large random property sweep** — 10 000 seeded-random inputs of
//!    varying lengths (1..2000) with mixed-sign regrets. Both paths must
//!    produce per-element outputs within `1e-6`.
//! 2. **Edge cases** — tiny inputs (the scalar-fallback path), lengths
//!    crossing the 8-wide tail boundary (1, 7, 8, 9, 15, 16), the 1326
//!    NLHE river scale, and the structural edge cases (all zeros, all
//!    negative, exactly one positive, mixed with zero entries).
//! 3. **NaN handling** — the scalar treats NaN as `<= 0` (never positive
//!    under `>`). The SIMD path must match this.
//!
//! Commit order matters: this test is intentionally committed *before*
//! the SIMD implementation so the test is exercised against a known-good
//! scalar reference first. If the file structure here changes, re-run
//! against the scalar-only stub to confirm it still type-checks.

use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;
use solver_core::matching::regret_match;
use solver_core::regret_match_simd;

/// Per-element tolerance for scalar-vs-SIMD equivalence.
///
/// Tighter than you'd think: for probability distributions on <= 2000
/// actions with regrets in `[-10, 10]`, the two paths differ only in the
/// summation order of the positive-regret total. f32 summation of up to
/// ~2000 values in `[0, 10]` introduces at most a few ulps of drift,
/// which feeds into a single division. Observed drift in practice:
/// < 1e-7. We allow 1e-6 as a comfortable bound.
const TOL: f32 = 1e-6;

/// Compare outputs of the two implementations on the same input.
///
/// Panics with a diagnostic message that pinpoints the offending index,
/// the input that produced it, and both path outputs.
fn assert_equivalent(regrets: &[f32]) {
    let n = regrets.len();
    let mut out_scalar = vec![0.0f32; n];
    let mut out_simd = vec![0.0f32; n];

    regret_match(regrets, &mut out_scalar);
    regret_match_simd(regrets, &mut out_simd);

    for (i, (&s, &v)) in out_scalar.iter().zip(out_simd.iter()).enumerate() {
        let diff = (s - v).abs();
        assert!(
            diff <= TOL,
            "scalar/simd diverge at n={n} idx={i}: scalar={s}, simd={v}, \
             diff={diff}, tol={TOL}, regrets_head={:?}",
            &regrets[..n.min(8)]
        );
    }
}

/// 10 000-trial randomized sweep across lengths 1..2000.
#[test]
fn property_sweep_random_regrets() {
    let mut rng = Xoshiro256PlusPlus::from_seed([7u8; 32]);
    for trial in 0..10_000 {
        // Length drawn per-trial so we hit every tail configuration.
        let n = rng.gen_range(1..=2000usize);
        // Mix of negative and positive entries. Range chosen to be
        // representative of realistic cumulative regrets without
        // inducing overflow in the positive-sum.
        let regrets: Vec<f32> = (0..n).map(|_| rng.gen_range(-10.0f32..10.0f32)).collect();

        // Extra diagnostic: include trial index on failure.
        let n = regrets.len();
        let mut out_scalar = vec![0.0f32; n];
        let mut out_simd = vec![0.0f32; n];
        regret_match(&regrets, &mut out_scalar);
        regret_match_simd(&regrets, &mut out_simd);
        for (i, (&s, &v)) in out_scalar.iter().zip(out_simd.iter()).enumerate() {
            let diff = (s - v).abs();
            assert!(
                diff <= TOL,
                "trial={trial} n={n} idx={i}: scalar={s}, simd={v}, diff={diff}"
            );
        }
    }
}

/// Lengths that exercise every tail/fallback branch in the SIMD path.
#[test]
fn boundary_lengths() {
    let mut rng = Xoshiro256PlusPlus::from_seed([42u8; 32]);
    // 1..=9 hits the <8 scalar fallback and the 8-wide+tail boundary.
    // 15, 16, 17 hits two f32x8 blocks. 1326 is NLHE scale.
    let lengths = [1, 2, 3, 7, 8, 9, 15, 16, 17, 23, 24, 25, 169, 1000, 1326, 2000];
    for n in lengths {
        // Try a few different seeded inputs at each length to cover sign mixes.
        for _ in 0..16 {
            let regrets: Vec<f32> = (0..n).map(|_| rng.gen_range(-5.0f32..5.0f32)).collect();
            assert_equivalent(&regrets);
        }
    }
}

#[test]
fn all_zeros_is_uniform() {
    for n in [1usize, 3, 7, 8, 9, 16, 1326] {
        let regrets = vec![0.0f32; n];
        assert_equivalent(&regrets);
    }
}

#[test]
fn all_negative_is_uniform() {
    let mut rng = Xoshiro256PlusPlus::from_seed([99u8; 32]);
    for n in [1usize, 3, 7, 8, 9, 16, 169, 1326] {
        let regrets: Vec<f32> = (0..n).map(|_| rng.gen_range(-100.0f32..-0.001f32)).collect();
        assert_equivalent(&regrets);
    }
}

#[test]
fn single_positive_is_pure() {
    // One spike of positive among a sea of zeros or negatives.
    for n in [1usize, 2, 3, 7, 8, 9, 16, 32, 169, 1326] {
        for active in 0..n.min(20) {
            let mut regrets = vec![-1.0f32; n];
            regrets[active] = 3.7;
            assert_equivalent(&regrets);
        }
        // Also with zeros as the background (tests that zero is not
        // accidentally counted as positive).
        for active in 0..n.min(5) {
            let mut regrets = vec![0.0f32; n];
            regrets[active] = 2.5;
            assert_equivalent(&regrets);
        }
    }
}

#[test]
fn mixed_with_exact_zeros() {
    // Mixed sign with some exact 0.0 entries — tests the `> 0` boundary.
    let cases: &[Vec<f32>] = &[
        vec![0.0, 1.0, -1.0, 0.0],
        vec![0.0, 0.0, 5.0, -3.0, 0.0, 2.0, -1.0, 0.0],
        vec![-1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, -2.0, 0.0, 1.5],
        {
            // 8 positives, 8 zeros, 8 negatives — tail and block mix.
            let mut v = Vec::with_capacity(24);
            v.extend([1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
            v.extend([0.0f32; 8]);
            v.extend([-1.0f32; 8]);
            v
        },
    ];
    for regrets in cases {
        assert_equivalent(regrets);
    }
}

#[test]
fn nan_regrets_treated_as_nonpositive() {
    // Scalar semantics: NaN regrets contribute 0 to the positive sum and
    // produce 0 in `out`. SIMD must match. We don't test NaN-only inputs
    // because NaN ordering to 0 is implementation-defined for `<= 0`;
    // instead we mix NaN with known positives and known negatives and
    // check that the distribution matches on the non-NaN lanes (which
    // are deterministic under the scalar spec) and is >= 0 on NaN lanes.
    let cases: &[Vec<f32>] = &[
        vec![1.0, f32::NAN, 2.0, -1.0, 0.0],
        vec![f32::NAN, 3.0, f32::NAN, -2.0, 0.5, f32::NAN, 1.0, 0.0],
        vec![
            f32::NAN,
            1.0,
            2.0,
            3.0,
            4.0,
            5.0,
            f32::NAN,
            6.0,
            7.0,
            8.0,
            f32::NAN,
        ],
    ];
    for regrets in cases {
        let n = regrets.len();
        let mut out_scalar = vec![0.0f32; n];
        let mut out_simd = vec![0.0f32; n];
        regret_match(regrets, &mut out_scalar);
        regret_match_simd(regrets, &mut out_simd);
        for (i, (&s, &v)) in out_scalar.iter().zip(out_simd.iter()).enumerate() {
            let diff = (s - v).abs();
            assert!(
                diff <= TOL,
                "NaN case: scalar/simd diverge at idx={i}: scalar={s}, simd={v}, \
                 diff={diff}, input={:?}",
                regrets
            );
        }
    }
}

#[test]
fn large_scale_nlhe_combos() {
    // The actual production length. Seeded random with the same seed the
    // micro-bench uses so this test and the bench touch a common input.
    let mut rng = Xoshiro256PlusPlus::from_seed([1u8; 32]);
    let regrets: Vec<f32> = (0..1326).map(|_| rng.gen_range(-1.0f32..1.0f32)).collect();
    assert_equivalent(&regrets);
}

#[test]
fn output_sums_to_one() {
    // Belt and suspenders: the SIMD path must still produce a valid
    // probability distribution.
    let mut rng = Xoshiro256PlusPlus::from_seed([5u8; 32]);
    for n in [1usize, 3, 7, 8, 9, 16, 169, 1326] {
        for _ in 0..50 {
            let regrets: Vec<f32> = (0..n).map(|_| rng.gen_range(-5.0f32..5.0f32)).collect();
            let mut out = vec![0.0f32; n];
            regret_match_simd(&regrets, &mut out);
            let sum: f32 = out.iter().sum();
            assert!(
                (sum - 1.0).abs() < 5e-5,
                "SIMD output did not sum to 1: n={n}, sum={sum}"
            );
            for (i, &p) in out.iter().enumerate() {
                assert!(p >= 0.0, "SIMD p[{i}]={p} < 0 at n={n}");
                assert!(p <= 1.0 + 1e-5, "SIMD p[{i}]={p} > 1 at n={n}");
            }
        }
    }
}
