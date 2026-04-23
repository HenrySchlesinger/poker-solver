//! Equity calculation: probability of winning given hands and (partial) board.
//!
//! Two modes:
//! - **Exact enumeration:** for river (remaining = 0 cards), single matmul.
//! - **Monte Carlo:** for earlier streets (remaining > 0), sample runouts.
//!
//! Monte Carlo is what Poker Panel already uses for the live equity bar.
//! We provide exact equity here primarily for the solver's internal use
//! (CFR needs accurate utility at terminals).

use crate::hand::Hand;
use crate::board::Board;

/// Win probability for `hero` vs `villain` on `board`.
///
/// If `board.len == 5` (river), exact. Otherwise, Monte Carlo with
/// `samples` random runouts.
// TODO (Day 2, agent A3): implement.
pub fn hand_vs_hand_equity(_hero: &Hand, _villain: &Hand, _board: &Board, _samples: u32) -> f32 {
    todo!()
}

/// Range-vs-range equity: weighted average over all pairs of combos.
///
/// This is the 1326×1326 matmul at the heart of Vector CFR on the river.
// TODO (Day 2, agent A3): implement.
pub fn range_vs_range_equity(
    _hero_weights: &[f32; 1326],
    _villain_weights: &[f32; 1326],
    _board: &Board,
) -> f32 {
    todo!()
}
