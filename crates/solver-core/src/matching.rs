//! Regret matching — converting cumulative regrets to a strategy.
//!
//! Given a vector of cumulative regrets `r[a]` for each action `a`:
//! - If any are positive, strategy is proportional to the positive part:
//!   `strategy[a] = max(r[a], 0) / sum_a max(r[a], 0)`
//! - Otherwise, uniform strategy across all actions.
//!
//! The scalar implementation is the baseline. The SIMD path is the river
//! hot loop and is the thing Day 3 optimization focuses on.

// TODO (Day 1, agent A_main): scalar implementation
// pub fn regret_match(regrets: &[f32], out: &mut [f32])
//
// TODO (Day 3, agent A1): SIMD f32x8 path
// pub fn regret_match_simd(regrets: &[f32; 1326], out: &mut [f32; 1326])
//
// Both must produce the same output (within f32 rounding). Add a
// property-based test that runs both on random regrets and asserts equality.
