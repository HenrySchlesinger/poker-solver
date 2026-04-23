//! Differential tests: `solver_eval::eval::eval_7` vs
//! `solver_eval_reference::reference_eval_7` on random inputs.
//!
//! # Why this test exists
//!
//! `solver_eval::eval::eval_7` wraps `rs_poker`'s hand evaluator. The
//! oracle in `solver_eval_reference::eval` is an independent
//! from-scratch implementation (categorical 5-card + C(7,5)=21
//! best-of subset). If the two disagree on a single (hand, board)
//! input, one of them has a bug. This test is the tripwire.
//!
//! Seeded PRNG → reproducible. If a future change introduces a
//! regression in either code path, the failure will re-surface with
//! the exact same inputs as CI saw.
//!
//! # What we compare — and what we don't
//!
//! Both evaluators produce `HandRank(u32)` packed as
//! `(category << 28) | inner`. Only the **category tag** and the
//! **induced total ordering** are the public contract:
//!
//! * `category` is stable: 0 = HighCard … 8 = StraightFlush. Both
//!   implementations use the same tag numbers.
//! * `inner` is implementation-defined. rs_poker uses an internal
//!   bitfield for kickers; the from-scratch oracle uses packed rank
//!   nibbles. The *ordering* is identical; the *bit layouts* are
//!   not. So we never compare raw u32 equality — that would be an
//!   over-specified test that fails even when both evaluators are
//!   correct.
//!
//! We run two tests:
//!
//! 1. `eval_7_category_matches_reference_on_random_inputs` — catches
//!    gross category bugs (misclassified straight-vs-flush).
//! 2. `eval_7_ordering_matches_reference_on_random_pairs` — catches
//!    tie-breaking and kicker bugs. This is the real correctness
//!    test.
//!
//! # Scope (Day 1)
//!
//! Right now we only exercise `eval_7`. As the equity and range-
//! math modules in `solver_eval` land (Day 2, agent A3), we'll add:
//! * `equity::hand_vs_hand_equity` vs
//!   `solver_eval_reference::reference_exact_river_equity` on all
//!   5-card boards.
//! * `equity::hand_vs_hand_equity` Monte Carlo agreement against
//!   `reference_equity_monte_carlo` (same seed).
//! * `solver_eval::combo::combo_index` bijection cross-checked
//!   against `reference_normalize_hand_169` coverage.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use solver_eval::eval::{eval_7, HandRank};
use solver_eval::{Board, Card, Hand};

use solver_eval_reference::reference_eval_7;

/// Extract the 4-bit category tag from a `HandRank`. Matches the
/// packing in both `solver_eval::eval::HandRank::from_rs` and
/// `solver_eval_reference::eval::eval_five`.
fn category(r: HandRank) -> u32 {
    r.0 >> 28
}

/// Shuffle-and-draw helper. Returns (hand, river_board) with all 7
/// cards distinct.
fn sample_scenario(rng: &mut StdRng) -> (Hand, Board) {
    // Build a 52-card deck, shuffle the first 7 slots, take them.
    let mut deck: [u8; 52] = std::array::from_fn(|i| i as u8);
    for i in 0..7 {
        let j = rng.gen_range(i..52);
        deck.swap(i, j);
    }
    let hand = Hand::new(Card(deck[0]), Card(deck[1]));
    let board = Board::river(
        Card(deck[2]),
        Card(deck[3]),
        Card(deck[4]),
        Card(deck[5]),
        Card(deck[6]),
    );
    (hand, board)
}

#[test]
fn eval_7_category_matches_reference_on_random_inputs() {
    // Deterministic seed so any failure is exactly reproducible. If
    // you change this seed, document WHY in the commit message — a
    // freshly-seeded test that passes by luck is worse than nothing.
    let mut rng = StdRng::seed_from_u64(0xA8_2026_04_22);

    for _ in 0..10_000 {
        let (hand, board) = sample_scenario(&mut rng);
        let prod = eval_7(&hand, &board);
        let oracle = reference_eval_7(&hand, &board);
        assert_eq!(
            category(prod),
            category(oracle),
            "category mismatch on hand={hand}, board={board}: \
             eval_7={prod:?} (cat {}), reference={oracle:?} (cat {})",
            category(prod),
            category(oracle),
        );
    }
}

#[test]
fn eval_7_ordering_matches_reference_on_random_pairs() {
    let mut rng = StdRng::seed_from_u64(0xA8_2026_04_22_u64.wrapping_add(1));

    for _ in 0..10_000 {
        let (h1, b1) = sample_scenario(&mut rng);
        let (h2, b2) = sample_scenario(&mut rng);

        let prod_ord = eval_7(&h1, &b1).cmp(&eval_7(&h2, &b2));
        let oracle_ord = reference_eval_7(&h1, &b1).cmp(&reference_eval_7(&h2, &b2));
        assert_eq!(
            prod_ord, oracle_ord,
            "ordering mismatch: ({h1}, {b1}) vs ({h2}, {b2}) — \
             prod_ord={prod_ord:?}, oracle_ord={oracle_ord:?}",
        );
    }
}
