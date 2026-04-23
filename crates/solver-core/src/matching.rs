//! Regret matching — converting cumulative regrets to a strategy.
//!
//! Given a vector of cumulative regrets `r[a]` for each action `a`:
//! - If any are positive, strategy is proportional to the positive part:
//!   `strategy[a] = max(r[a], 0) / sum_a max(r[a], 0)`
//! - Otherwise, uniform strategy across all actions.
//!
//! The scalar implementation is the baseline. The SIMD path is the river
//! hot loop and is the thing Day 3 optimization focuses on.

// Scalar implementation. The SIMD f32x8 variant is Day 3 (agent A1).

/// Convert cumulative regrets to a strategy, writing into `out`.
///
/// # Behavior
///
/// - If at least one regret is positive, `out[a] = max(regrets[a], 0) / S`
///   where `S` is the sum of positive regrets.
/// - If all regrets are `<= 0`, `out` is filled with a uniform
///   `1.0 / regrets.len()` strategy.
///
/// # Panics
///
/// Panics if `regrets.len() != out.len()`, or if either is empty.
///
/// # f32 hygiene
///
/// `NaN` regrets are treated as `<= 0` (contribute 0 to the sum and to the
/// numerator). This matches the "uniform fallback when nothing is
/// positive" semantics and keeps the function total over all f32 inputs.
pub fn regret_match(regrets: &[f32], out: &mut [f32]) {
    assert_eq!(
        regrets.len(),
        out.len(),
        "regret_match: regrets and out must have the same length"
    );
    assert!(
        !regrets.is_empty(),
        "regret_match: cannot operate on an empty action set"
    );

    let mut sum_positive = 0.0f32;
    for &r in regrets.iter() {
        if r > 0.0 {
            sum_positive += r;
        }
    }

    if sum_positive > 0.0 {
        let inv = 1.0 / sum_positive;
        for (o, &r) in out.iter_mut().zip(regrets.iter()) {
            *o = if r > 0.0 { r * inv } else { 0.0 };
        }
    } else {
        let u = 1.0 / (regrets.len() as f32);
        for o in out.iter_mut() {
            *o = u;
        }
    }
}

/// Convenience form of [`regret_match`] that allocates the output vector.
///
/// Prefer [`regret_match`] on hot paths; this helper exists for tests and
/// one-shot callers where allocation overhead is irrelevant.
pub fn regret_match_vec(regrets: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0f32; regrets.len()];
    regret_match(regrets, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Property: all-zero regrets produce a uniform strategy.
    #[test]
    fn all_zero_regrets_is_uniform() {
        for n in 1..=12 {
            let regrets = vec![0.0f32; n];
            let strat = regret_match_vec(&regrets);
            let expected = 1.0 / (n as f32);
            for (i, &p) in strat.iter().enumerate() {
                assert!(
                    (p - expected).abs() < 1e-6,
                    "n={n} idx={i}: expected uniform {expected}, got {p}"
                );
            }
            let total: f32 = strat.iter().sum();
            assert!((total - 1.0).abs() < 1e-6, "strategy must sum to 1");
        }
    }

    /// Property: all-negative regrets also fall back to uniform.
    #[test]
    fn all_negative_regrets_is_uniform() {
        let regrets = [-1.0, -2.0, -0.5, -10.0];
        let strat = regret_match_vec(&regrets);
        for &p in &strat {
            assert!((p - 0.25).abs() < 1e-6);
        }
    }

    /// Property: exactly one positive regret produces a pure strategy on
    /// that action (probability 1), with zeros elsewhere.
    #[test]
    fn one_positive_regret_is_pure() {
        // Active slot varies across the action set; others are negative.
        for n in 2..=8 {
            for active in 0..n {
                let mut regrets = vec![-1.0f32; n];
                regrets[active] = 3.7;
                let strat = regret_match_vec(&regrets);
                for (i, &p) in strat.iter().enumerate() {
                    let expected = if i == active { 1.0 } else { 0.0 };
                    assert!(
                        (p - expected).abs() < 1e-6,
                        "n={n} active={active} idx={i}: expected {expected}, got {p}"
                    );
                }
            }
        }
    }

    /// Property: one positive regret with others at zero is still pure.
    #[test]
    fn one_positive_others_zero_is_pure() {
        let regrets = [0.0, 0.0, 2.0, 0.0];
        let strat = regret_match_vec(&regrets);
        assert!((strat[2] - 1.0).abs() < 1e-6);
        assert!(strat[0].abs() < 1e-6);
        assert!(strat[1].abs() < 1e-6);
        assert!(strat[3].abs() < 1e-6);
    }

    /// Property: mixed positives normalize by their positive-sum.
    #[test]
    fn mixed_positive_regrets_normalize() {
        let regrets = [1.0, -2.0, 3.0, 0.0];
        let strat = regret_match_vec(&regrets);
        // Positive sum = 4.0. Expected: [0.25, 0.0, 0.75, 0.0]
        assert!((strat[0] - 0.25).abs() < 1e-6);
        assert!(strat[1].abs() < 1e-6);
        assert!((strat[2] - 0.75).abs() < 1e-6);
        assert!(strat[3].abs() < 1e-6);
        let total: f32 = strat.iter().sum();
        assert!((total - 1.0).abs() < 1e-6);
    }

    /// Sanity: output is always a valid probability distribution.
    #[test]
    fn output_is_probability_distribution() {
        let cases: &[&[f32]] = &[
            &[0.0],
            &[1.0],
            &[-1.0],
            &[1.0, 1.0, 1.0],
            &[-1.0, 2.0, -3.0, 4.0],
            &[0.0, 0.0, 5.0],
            &[100.0, 0.000_001, -50.0],
        ];
        for &regrets in cases {
            let s = regret_match_vec(regrets);
            for &p in &s {
                assert!(p >= 0.0, "strategy entry must be >= 0, got {p}");
                assert!(p <= 1.0 + 1e-6, "strategy entry must be <= 1, got {p}");
            }
            let sum: f32 = s.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-5,
                "sum must be 1.0 for {regrets:?}, got {sum}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "empty action set")]
    fn empty_input_panics() {
        let mut out: [f32; 0] = [];
        regret_match(&[], &mut out);
    }

    #[test]
    #[should_panic(expected = "same length")]
    fn mismatched_lengths_panic() {
        let regrets = [1.0, 2.0];
        let mut out = [0.0; 3];
        regret_match(&regrets, &mut out);
    }
}
