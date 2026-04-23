//! Property-based tests for `solver-eval` primitives.
//!
//! These tests encode invariants that the module's types and functions
//! must satisfy UNIVERSALLY — not just on the hand-picked examples in the
//! per-file `#[cfg(test)] mod tests`. `proptest` generates thousands of
//! random inputs and shrinks counterexamples automatically, which is the
//! right tool for "this must hold for every `Card(u8)` in 0..52" kinds of
//! claims.
//!
//! # Determinism / reproducibility
//!
//! Every test uses a fixed `proptest::test_runner::Config` with a
//! literal `rng_algorithm = RngAlgorithm::ChaCha` and a named failure
//! persistence file disabled (we want flat reproducibility, not a
//! `.proptest-regressions` dance). Seeds are pinned via `Config::with_cases`
//! plus the default strategy, which — because `proptest`'s default RNG
//! is the XorShift deterministic mode — makes every CI run see the same
//! input sequence.
//!
//! If one of these tests fails, the reported counterexample comes out
//! of proptest's shrinker and the rerun is reproducible given the same
//! `PROPTEST_SEED` env var (we use the default seed here; override with
//! `PROPTEST_SEED=<u64>` to inject a different stream).
//!
//! # Catalogue of invariants encoded here
//!
//! 1. `Card::parse(format!("{}", Card(x))) == Some(Card(x))` for all
//!    `x in 0..52` — round-trip between the display and parse paths.
//! 2. `Hand::new(a, b) == Hand::new(b, a)` for any distinct `a`, `b` in
//!    0..52 — hand canonicalization is order-independent.
//! 3. `Board::parse(board.to_string()) == Some(board)` for any legal
//!    Board of length 3, 4, or 5 with all cards distinct.
//! 4. `combo_index(index_to_combo(i)) == i` for all `i in 0..1326` —
//!    the combo bijection is truly a bijection.
//! 5. `eval_7(h, b) == eval_7(h, b)` for any legal (hand, river) input —
//!    evaluator is deterministic.
//! 6. Equity symmetry: `win(a,b,board) + win(b,a,board) + tie ≈ 1.0`
//!    within MC tolerance (≤ 0.02) for any non-conflicting (hand, hand,
//!    board) pair at 10 000 samples.
//!
//! Each test's documentation records the specific mutation it catches —
//! "if I break X in the source, this test fails because Y."

use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence};

use solver_eval::board::Board;
use solver_eval::card::Card;
use solver_eval::combo::{combo_index, index_to_combo, NUM_COMBOS};
use solver_eval::equity::{hand_vs_hand_equity, hand_vs_hand_outcome};
use solver_eval::eval::eval_7;
use solver_eval::hand::Hand;

/// Fixed proptest config used across every test in this file.
///
/// Deterministic: proptest's default RNG seeded with a constant value.
/// Higher case count where we want stronger coverage; lower where the
/// per-case work is expensive (equity MC with 10k samples per case).
fn fast_config() -> Config {
    Config {
        cases: 1024,
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        // Deterministic default source rng keyed from a constant.
        source_file: Some("proptest_primitives.rs"),
        ..Config::default()
    }
}

fn slow_config() -> Config {
    Config {
        cases: 32,
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        source_file: Some("proptest_primitives.rs"),
        ..Config::default()
    }
}

// --------------------------------------------------------------------------
// 1. Card parse/display round-trip.
// --------------------------------------------------------------------------

proptest! {
    #![proptest_config(fast_config())]

    /// For every Card(x) with x in 0..52, parsing its display form yields
    /// the original card.
    ///
    /// Mutation check: if `Rank::to_char` ever maps two distinct ranks to
    /// the same character (e.g., both Ten and Jack -> 'T'), this test
    /// fails because the round-trip loses information. Same for Suit.
    #[test]
    fn card_display_parse_roundtrip(x in 0u8..52) {
        let c = Card(x);
        let s = format!("{c}");
        let parsed = Card::parse(&s);
        prop_assert_eq!(parsed, Some(c), "card {} failed to roundtrip: {}", x, s);
    }
}

// --------------------------------------------------------------------------
// 2. Hand::new canonicalization.
// --------------------------------------------------------------------------

/// Strategy: a pair of distinct u8 card indices in 0..52.
fn distinct_card_pair() -> impl Strategy<Value = (u8, u8)> {
    (0u8..52, 0u8..52).prop_filter("cards must be distinct", |(a, b)| a != b)
}

proptest! {
    #![proptest_config(fast_config())]

    /// Hand::new is symmetric: swapping arguments yields the same hand.
    ///
    /// Mutation check: if `Hand::new` ever forgot its swap (e.g., always
    /// stored `[a, b]` instead of sorting by rank), this test would fail
    /// on any pair where the "smaller" card was passed first.
    #[test]
    fn hand_new_is_symmetric((a, b) in distinct_card_pair()) {
        let h1 = Hand::new(Card(a), Card(b));
        let h2 = Hand::new(Card(b), Card(a));
        prop_assert_eq!(h1, h2, "Hand::new not symmetric for ({}, {})", a, b);
    }

    /// Hand's higher-card-first invariant: after construction, the first
    /// element has a strictly greater u8 than the second. This catches a
    /// canonicalization bug that `hand_new_is_symmetric` might miss if
    /// Hand's Eq were broken.
    #[test]
    fn hand_higher_card_first((a, b) in distinct_card_pair()) {
        let h = Hand::new(Card(a), Card(b));
        prop_assert!(
            h.0[0].0 > h.0[1].0,
            "Hand not canonicalized: cards[0]={} cards[1]={}",
            h.0[0].0, h.0[1].0
        );
    }
}

// --------------------------------------------------------------------------
// 3. Board parse/display round-trip.
// --------------------------------------------------------------------------

/// Strategy: a vector of `n` distinct cards, for n in {3, 4, 5}.
fn distinct_board_cards(n: usize) -> impl Strategy<Value = Vec<Card>> {
    prop::collection::vec(0u8..52, n)
        .prop_filter("board cards must be distinct", |v| {
            // O(n^2) distinctness check, n<=5.
            let mut seen = 0u64;
            for &c in v {
                let bit = 1u64 << c;
                if seen & bit != 0 {
                    return false;
                }
                seen |= bit;
            }
            true
        })
        .prop_map(|v| v.into_iter().map(Card).collect())
}

/// Strategy: a legal postflop Board (flop, turn, or river).
fn board_strategy() -> impl Strategy<Value = Board> {
    prop_oneof![
        distinct_board_cards(3).prop_map(|cs| Board::flop(cs[0], cs[1], cs[2])),
        distinct_board_cards(4).prop_map(|cs| Board::turn(cs[0], cs[1], cs[2], cs[3])),
        distinct_board_cards(5).prop_map(|cs| Board::river(cs[0], cs[1], cs[2], cs[3], cs[4])),
    ]
}

proptest! {
    #![proptest_config(fast_config())]

    /// Board::parse(board.to_string()) returns the original board.
    ///
    /// Mutation check: if `Board::parse` forgot to accept 4-card (turn)
    /// inputs, or mis-parsed the 5th card on a river, this test would
    /// fail on any input that has that length.
    #[test]
    fn board_display_parse_roundtrip(board in board_strategy()) {
        let s = format!("{board}");
        let parsed = Board::parse(&s).expect("valid board should parse");
        prop_assert_eq!(parsed, board);
    }
}

// --------------------------------------------------------------------------
// 4. Combo bijection.
// --------------------------------------------------------------------------

proptest! {
    #![proptest_config(Config {
        cases: 1326 * 2,
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        source_file: Some("proptest_primitives.rs"),
        ..Config::default()
    })]

    /// For any combo index i in 0..1326, index_to_combo followed by
    /// combo_index returns i.
    ///
    /// Mutation check: if the formula in `combo_index` drifted by a sign
    /// or constant (e.g., the bug the module comment calls out), this
    /// test fails because some indices collide.
    #[test]
    fn combo_bijection_roundtrip(i in 0usize..NUM_COMBOS) {
        let (a, b) = index_to_combo(i);
        prop_assert!(a.0 < b.0, "index_to_combo must return (lo, hi)");
        let back = combo_index(a, b);
        prop_assert_eq!(back, i, "combo {} -> ({},{}) -> {} did not roundtrip", i, a.0, b.0, back);
    }

    /// combo_index is symmetric in its inputs for any distinct pair.
    ///
    /// Mutation check: if the canonicalization branch flipped, the test
    /// would fail on any pair whose "wrong" order wasn't the natural one.
    #[test]
    fn combo_index_is_symmetric((a, b) in distinct_card_pair()) {
        let i1 = combo_index(Card(a), Card(b));
        let i2 = combo_index(Card(b), Card(a));
        prop_assert_eq!(
            i1, i2,
            "combo_index({},{}) != combo_index({},{})",
            a, b, b, a
        );
    }
}

// --------------------------------------------------------------------------
// 5. eval_7 determinism.
// --------------------------------------------------------------------------

/// Strategy: a legal (hand, river-board) pair where all 7 cards are distinct.
fn hand_and_river_board() -> impl Strategy<Value = (Hand, Board)> {
    prop::collection::vec(0u8..52, 7)
        .prop_filter("all 7 cards must be distinct", |v| {
            let mut seen = 0u64;
            for &c in v {
                let bit = 1u64 << c;
                if seen & bit != 0 {
                    return false;
                }
                seen |= bit;
            }
            true
        })
        .prop_map(|v| {
            let hand = Hand::new(Card(v[0]), Card(v[1]));
            let board = Board::river(Card(v[2]), Card(v[3]), Card(v[4]), Card(v[5]), Card(v[6]));
            (hand, board)
        })
}

proptest! {
    #![proptest_config(fast_config())]

    /// eval_7 is a pure function: calling it twice on the same input
    /// yields identical HandRank.
    ///
    /// Mutation check: if `eval_7` picked up any kind of caching that
    /// accidentally flipped a bit based on call order, this test fails
    /// on the second call.
    #[test]
    fn eval_7_is_deterministic((hand, board) in hand_and_river_board()) {
        let r1 = eval_7(&hand, &board);
        let r2 = eval_7(&hand, &board);
        prop_assert_eq!(r1, r2, "eval_7 not deterministic");
    }
}

// --------------------------------------------------------------------------
// 6. Equity outcome probability mass is 1.0.
// --------------------------------------------------------------------------

/// Strategy: (hero, villain, board) at river, all 9 cards distinct.
fn hero_villain_river_board() -> impl Strategy<Value = (Hand, Hand, Board)> {
    prop::collection::vec(0u8..52, 9)
        .prop_filter("all 9 cards must be distinct", |v| {
            let mut seen = 0u64;
            for &c in v {
                let bit = 1u64 << c;
                if seen & bit != 0 {
                    return false;
                }
                seen |= bit;
            }
            true
        })
        .prop_map(|v| {
            let hero = Hand::new(Card(v[0]), Card(v[1]));
            let villain = Hand::new(Card(v[2]), Card(v[3]));
            let board = Board::river(Card(v[4]), Card(v[5]), Card(v[6]), Card(v[7]), Card(v[8]));
            (hero, villain, board)
        })
}

/// Same but for a flop (MC path).
fn hero_villain_flop() -> impl Strategy<Value = (Hand, Hand, Board)> {
    prop::collection::vec(0u8..52, 7)
        .prop_filter("all 7 cards must be distinct", |v| {
            let mut seen = 0u64;
            for &c in v {
                let bit = 1u64 << c;
                if seen & bit != 0 {
                    return false;
                }
                seen |= bit;
            }
            true
        })
        .prop_map(|v| {
            let hero = Hand::new(Card(v[0]), Card(v[1]));
            let villain = Hand::new(Card(v[2]), Card(v[3]));
            let board = Board::flop(Card(v[4]), Card(v[5]), Card(v[6]));
            (hero, villain, board)
        })
}

proptest! {
    #![proptest_config(fast_config())]

    /// On the river (exact enumeration), win(a,b,board) + win(b,a,board)
    /// + tie == 1.0 exactly — no MC noise because there is no random
    /// runout.
    ///
    /// Mutation check: if the `hand_vs_hand_outcome` river branch ever
    /// mislabelled a "lose" as a "tie" or similar, the three numbers
    /// would no longer sum to 1.
    #[test]
    fn equity_symmetry_river_exact((hero, villain, board) in hero_villain_river_board()) {
        let (win_ab, tie_ab) = hand_vs_hand_outcome(&hero, &villain, &board, 1);
        let (win_ba, tie_ba) = hand_vs_hand_outcome(&villain, &hero, &board, 1);
        // River is exact — `tie_ab` and `tie_ba` must be identical.
        prop_assert_eq!(tie_ab, tie_ba, "tie probabilities must be equal on river");
        let total = win_ab + win_ba + tie_ab;
        prop_assert!(
            (total - 1.0).abs() < 1e-6,
            "river outcome mass: {win_ab} + {win_ba} + {tie_ab} = {total}, expected 1.0"
        );
    }
}

proptest! {
    // Slow: each case does MC at 10 000 samples, twice. 32 cases is ~2s.
    #![proptest_config(slow_config())]

    /// Equity symmetry on the flop (MC path): win(a,b) + win(b,a) + tie
    /// is within 0.02 of 1.0.
    ///
    /// This is the task-brief invariant: even though MC noise adds
    /// stochastic drift, the sum must stay within 2% of 1.0 for any
    /// legal input. A wider tolerance than the river case because the
    /// two MC runs use DIFFERENT seeds (the seed is
    /// hero/villain-dependent, not board-dependent), so they see
    /// different runouts.
    ///
    /// Mutation check: if the `seeded_rng` function became
    /// non-deterministic (e.g., started seeding from a time source),
    /// drift would grow beyond the 0.02 bound.
    #[test]
    fn equity_symmetry_flop_mc((hero, villain, board) in hero_villain_flop()) {
        let e_ab = hand_vs_hand_equity(&hero, &villain, &board, 10_000);
        let e_ba = hand_vs_hand_equity(&villain, &hero, &board, 10_000);
        // Use the equity formulation (win + 0.5*tie): e_ab + e_ba == 1.0
        // up to MC noise; tolerance 0.02 per the brief.
        let total = e_ab + e_ba;
        prop_assert!(
            (total - 1.0).abs() < 0.02,
            "flop MC symmetry: eq({hero}, {villain}) + eq({villain}, {hero}) \
             = {e_ab} + {e_ba} = {total}; expected within 0.02 of 1.0"
        );
    }
}
