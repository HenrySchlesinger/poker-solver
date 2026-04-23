//! Ported from the showdown / winner-finding pattern that appears in
//! `engine/rules/equity_optimized.py::exact_river_equity` and
//! `engine/rules/holdem.py::calculate_player_percentages`:
//!
//! ```python
//! scores = {p: evaluator.evaluate(board, p.hole) for p in players}
//! best_score = min(scores.values())
//! winners = [p for p, s in scores.items() if s == best_score]
//! ```
//!
//! In our Rust world `solver_eval::HandRank` is a "higher is better"
//! packed u32 — so `min` becomes `max`, but the tie-handling and return
//! shape are otherwise literal.

use solver_eval::eval::HandRank;

/// Given a list of hand ranks (one per player), return the indices of
/// all players tied for the best hand.
///
/// * Empty input → empty `Vec` (matches the Python `if not winners`
///   early-out).
/// * One winner → `Vec` of length 1.
/// * N-way split → `Vec` of length N, in input order.
///
/// The Python source uses a dict keyed by `seat_id`; here we use the
/// input slice index as the identity. The caller is free to map back.
pub fn reference_showdown_winners(ranks: &[HandRank]) -> Vec<usize> {
    if ranks.is_empty() {
        return Vec::new();
    }

    // Python: `best_score = min(scores.values())` — but treys uses
    // lower-is-better. Our HandRank is higher-is-better, so we take
    // the max. That's the only non-literal step in this port; the
    // ordering inversion is called out in `eval.rs`.
    let mut best = ranks[0];
    for &r in &ranks[1..] {
        if r > best {
            best = r;
        }
    }

    // Python: `winners = [sid for sid, s in scores.items() if s == best]`
    let mut winners = Vec::with_capacity(ranks.len());
    for (i, &r) in ranks.iter().enumerate() {
        if r == best {
            winners.push(i);
        }
    }
    winners
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_empty() {
        assert!(reference_showdown_winners(&[]).is_empty());
    }

    #[test]
    fn single_winner() {
        // Handcrafted HandRanks: 10 beats 5 beats 2.
        let ranks = [HandRank(10), HandRank(5), HandRank(2)];
        assert_eq!(reference_showdown_winners(&ranks), vec![0]);
    }

    #[test]
    fn three_way_tie_returns_all_three() {
        let ranks = [HandRank(42), HandRank(42), HandRank(10), HandRank(42)];
        assert_eq!(reference_showdown_winners(&ranks), vec![0, 1, 3]);
    }

    #[test]
    fn all_equal_returns_all() {
        let ranks = [HandRank(7), HandRank(7), HandRank(7)];
        assert_eq!(reference_showdown_winners(&ranks), vec![0, 1, 2]);
    }
}
