//! Independent 5-card and 7-card hand evaluator — ported from
//! Poker Panel's reliance on `treys.Evaluator` by re-deriving the
//! standard 5-card category-and-kicker algorithm directly.
//!
//! # Why reimplement instead of wrap `rs_poker`?
//!
//! The whole point of this crate is *differential* testing — we
//! compare a from-scratch oracle to the production code in
//! `solver-eval`. `solver-eval` wraps `rs_poker`. If this crate also
//! wraps `rs_poker` then any bug in `rs_poker` would be blessed by
//! *both* implementations and never get caught. A second independent
//! evaluator here means that:
//!
//! 1. If `solver-eval::eval::eval_7` disagrees with us, there's a
//!    genuine bug somewhere — either rs_poker has a regression, the
//!    wrapping layer in `solver-eval` is wrong, or we are wrong.
//! 2. We also don't inherit rs_poker's edition-2024 toolchain
//!    requirement (4.1.0 needs Rust 1.85+), so this crate builds on
//!    the repo's pinned 1.82.0.
//!
//! # Algorithm
//!
//! Classic "best-of-21" for 7-card: enumerate the C(7, 5) = 21
//! subsets, evaluate each as a 5-card hand, take the max. For 5-card
//! evaluation we do a direct categorical check:
//!
//!   1. Straight flush: 5 same suit AND ranks form a straight.
//!   2. Quads: 4 of one rank.
//!   3. Full house: 3 + 2 of distinct ranks.
//!   4. Flush: 5 same suit.
//!   5. Straight: 5 consecutive ranks (with A-5 wheel).
//!   6. Trips: 3 of one rank.
//!   7. Two pair.
//!   8. One pair.
//!   9. High card.
//!
//! Within each category we pack kickers into a `u32` in descending
//! rank order. The output `HandRank` format is documented in
//! `solver_eval::eval::HandRank` — we re-derive the same scheme here
//! (4-bit category || 28-bit inner) so differential tests can compare
//! the two outputs with `==`.
//!
//! This is O(21 * constant) per 7-card eval — plenty fast for an
//! oracle. The production path (`solver_eval::eval::eval_7`) uses
//! `rs_poker`'s bit-trick evaluator, which is ~100× faster; that's
//! the hot path. This one is the "written so clearly you can audit
//! it by eye" path.

use solver_eval::eval::HandRank;
use solver_eval::{Board, Card, Hand};

/// Public 7-card evaluator. Mirrors
/// `engine/rules/holdem.py::evaluate_best_five_of_seven`: take the
/// 2 hole cards + 5 board cards and return the best 5-card hand rank.
pub fn reference_eval_7(hand: &Hand, board: &Board) -> HandRank {
    debug_assert_eq!(
        board.len, 5,
        "reference_eval_7 requires a full 5-card (river) board",
    );
    let cards: [Card; 7] = [
        hand.0[0],
        hand.0[1],
        board.cards[0],
        board.cards[1],
        board.cards[2],
        board.cards[3],
        board.cards[4],
    ];
    best_of_seven(&cards)
}

/// Public 5-card evaluator. For tests and the river microbench.
pub fn reference_eval_5(cards: &[Card; 5]) -> HandRank {
    eval_five(cards)
}

/// Internal: enumerate C(7, 5) = 21 subsets, return the max rank.
fn best_of_seven(cards: &[Card; 7]) -> HandRank {
    // 21 hardcoded subset indices — every combination of 5 distinct
    // indices from 0..7. Faster to unroll than to do nested loops.
    const SUBSETS: [[u8; 5]; 21] = [
        [0, 1, 2, 3, 4],
        [0, 1, 2, 3, 5],
        [0, 1, 2, 3, 6],
        [0, 1, 2, 4, 5],
        [0, 1, 2, 4, 6],
        [0, 1, 2, 5, 6],
        [0, 1, 3, 4, 5],
        [0, 1, 3, 4, 6],
        [0, 1, 3, 5, 6],
        [0, 1, 4, 5, 6],
        [0, 2, 3, 4, 5],
        [0, 2, 3, 4, 6],
        [0, 2, 3, 5, 6],
        [0, 2, 4, 5, 6],
        [0, 3, 4, 5, 6],
        [1, 2, 3, 4, 5],
        [1, 2, 3, 4, 6],
        [1, 2, 3, 5, 6],
        [1, 2, 4, 5, 6],
        [1, 3, 4, 5, 6],
        [2, 3, 4, 5, 6],
    ];
    let mut best = HandRank(0);
    for idx in &SUBSETS {
        let five = [
            cards[idx[0] as usize],
            cards[idx[1] as usize],
            cards[idx[2] as usize],
            cards[idx[3] as usize],
            cards[idx[4] as usize],
        ];
        let r = eval_five(&five);
        if r > best {
            best = r;
        }
    }
    best
}

/// Categorical 5-card hand evaluator. Returns `HandRank(category <<
/// 28 | inner)`, identical packing to `solver_eval::eval::HandRank`.
fn eval_five(cards: &[Card; 5]) -> HandRank {
    // Extract rank (0..13) and suit (0..4) per card.
    let mut ranks: [u8; 5] = [
        cards[0].0 >> 2,
        cards[1].0 >> 2,
        cards[2].0 >> 2,
        cards[3].0 >> 2,
        cards[4].0 >> 2,
    ];
    let suits: [u8; 5] = [
        cards[0].0 & 0b11,
        cards[1].0 & 0b11,
        cards[2].0 & 0b11,
        cards[3].0 & 0b11,
        cards[4].0 & 0b11,
    ];
    // Sort ranks descending (insertion sort on 5 elements — trivial).
    ranks.sort_unstable_by(|a, b| b.cmp(a));

    let flush = suits[0] == suits[1]
        && suits[1] == suits[2]
        && suits[2] == suits[3]
        && suits[3] == suits[4];

    // Straight detection. `ranks` is descending. Normal straight:
    // ranks[i] == ranks[0] - i for i in 0..5.
    // Wheel (A-5): ranks == [12, 3, 2, 1, 0].
    let (is_straight, top_of_straight) = detect_straight(&ranks);

    // Rank histogram: count per rank (0..13). For 5 cards with
    // distinct ranks this is all 1s.
    let mut counts = [0u8; 13];
    for &r in ranks.iter() {
        counts[r as usize] += 1;
    }

    // Build a list of (count, rank) pairs sorted by (count desc,
    // rank desc). This is the canonical "multiplicity-first,
    // high-card-breaks-ties" ordering that every poker evaluator
    // uses.
    let mut multiplicities: Vec<(u8, u8)> = (0..13u8)
        .filter(|r| counts[*r as usize] > 0)
        .map(|r| (counts[r as usize], r))
        .collect();
    multiplicities.sort_by(|a, b| {
        b.0.cmp(&a.0).then(b.1.cmp(&a.1)) // count desc, rank desc
    });

    // Category tag matches `HandRank::from_rs` in solver_eval:
    //   0 HighCard, 1 OnePair, 2 TwoPair, 3 ThreeOfAKind,
    //   4 Straight, 5 Flush, 6 FullHouse, 7 FourOfAKind,
    //   8 StraightFlush.
    //
    // `inner` is a u32 that is monotonic within its category. We
    // construct it as "rank nibbles concatenated", high-nibble =
    // most-significant kicker. 4 bits per rank × up to 5 kickers =
    // 20 bits, comfortably under the 28-bit budget.
    const INNER_BITS: u32 = 28;

    let (category, inner): (u32, u32) = if flush && is_straight {
        // 8 — straight flush. Inner = top-of-straight rank.
        (8, top_of_straight as u32)
    } else if multiplicities[0].0 == 4 {
        // 7 — four of a kind. Inner: quad rank || kicker.
        let quad_rank = multiplicities[0].1;
        let kicker = multiplicities[1].1;
        (7, pack_ranks(&[quad_rank, kicker]))
    } else if multiplicities[0].0 == 3 && multiplicities.len() >= 2 && multiplicities[1].0 == 2 {
        // 6 — full house. Inner: trip rank || pair rank.
        let trip = multiplicities[0].1;
        let pair = multiplicities[1].1;
        (6, pack_ranks(&[trip, pair]))
    } else if flush {
        // 5 — flush. Inner: all 5 ranks, descending.
        (5, pack_ranks(&ranks))
    } else if is_straight {
        // 4 — straight. Inner: top rank.
        (4, top_of_straight as u32)
    } else if multiplicities[0].0 == 3 {
        // 3 — three of a kind. Inner: trip rank || 2 kickers.
        let trip = multiplicities[0].1;
        let k1 = multiplicities[1].1;
        let k2 = multiplicities[2].1;
        (3, pack_ranks(&[trip, k1, k2]))
    } else if multiplicities[0].0 == 2 && multiplicities.len() >= 2 && multiplicities[1].0 == 2 {
        // 2 — two pair. Inner: high pair || low pair || kicker.
        let hp = multiplicities[0].1;
        let lp = multiplicities[1].1;
        let k = multiplicities[2].1;
        (2, pack_ranks(&[hp, lp, k]))
    } else if multiplicities[0].0 == 2 {
        // 1 — one pair. Inner: pair rank || 3 kickers.
        let p = multiplicities[0].1;
        let k1 = multiplicities[1].1;
        let k2 = multiplicities[2].1;
        let k3 = multiplicities[3].1;
        (1, pack_ranks(&[p, k1, k2, k3]))
    } else {
        // 0 — high card. Inner: all 5 ranks, descending.
        (0, pack_ranks(&ranks))
    };

    debug_assert!(inner < (1 << INNER_BITS), "inner overflowed 28 bits");
    HandRank((category << INNER_BITS) | (inner & ((1 << INNER_BITS) - 1)))
}

/// Pack an ordered list of ranks (0..13, highest first) into a u32
/// with 4 bits per rank. Used to build the `inner` part of a
/// `HandRank` value.
fn pack_ranks(ranks: &[u8]) -> u32 {
    let mut out: u32 = 0;
    for &r in ranks.iter() {
        out = (out << 4) | (r as u32 & 0xF);
    }
    out
}

/// Detect straight. `ranks` is 5 elements, sorted descending.
/// Returns `(is_straight, top_rank)`. For the wheel (A-2-3-4-5) the
/// top rank is Five (3 in our 0-indexed rank encoding).
fn detect_straight(ranks: &[u8; 5]) -> (bool, u8) {
    // Normal straight: ranks[0] - 4 == ranks[4] and every step is 1.
    let normal = ranks[0] == ranks[1] + 1
        && ranks[1] == ranks[2] + 1
        && ranks[2] == ranks[3] + 1
        && ranks[3] == ranks[4] + 1;
    if normal {
        return (true, ranks[0]);
    }
    // Wheel: A-5-4-3-2. After sorting descending = [12, 3, 2, 1, 0].
    let wheel = ranks[0] == 12 && ranks[1] == 3 && ranks[2] == 2 && ranks[3] == 1 && ranks[4] == 0;
    if wheel {
        // Top-of-straight for scoring is Five (rank index 3), not Ace.
        return (true, 3);
    }
    (false, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: parse strings and call the reference evaluator.
    fn eval7_str(hand: &str, board: &str) -> HandRank {
        let h = Hand::parse(hand).unwrap();
        let b = Board::parse(board).unwrap();
        reference_eval_7(&h, &b)
    }

    #[test]
    fn flush_beats_straight() {
        let flush = eval7_str("2d3d", "4d5d6d9sKs");
        let straight = eval7_str("8c9c", "5d6h7sThQs");
        assert!(flush > straight);
    }

    #[test]
    fn full_house_beats_trips() {
        let full = eval7_str("AsAc", "AdKhKd2c7s");
        let trips = eval7_str("AsAc", "Ad5h9d2c7s");
        assert!(full > trips);
    }

    #[test]
    fn royal_flush_is_top() {
        let royal = eval7_str("AhKh", "QhJhTh2c3s");
        let quads = eval7_str("AsAc", "AdAhKc2s3d");
        assert!(royal > quads);

        // Category tag should be 8 (StraightFlush).
        let cat = royal.0 >> 28;
        assert_eq!(cat, 8);
    }

    #[test]
    fn wheel_straight_is_detected() {
        // A-2-3-4-5 straight on board 6h 8s + hero 7c Tc → straight
        // 5-high (wheel), or 8-high? Hero has 7c + board 8s 6h →
        // 8-7-6-5-4 straight (8-high). Craft a pure wheel instead:
        // hero: Ac 2c; board: 3h 4d 5s Kh 9c → wheel (A-2-3-4-5).
        let wheel = eval7_str("Ac2c", "3h4d5sKh9c");
        // Category should be 4 (Straight).
        let cat = wheel.0 >> 28;
        assert_eq!(cat, 4, "wheel should be a straight");
        // Inner should be 3 (top card of wheel is Five, rank index 3).
        let inner = wheel.0 & ((1 << 28) - 1);
        assert_eq!(inner, 3);
    }

    #[test]
    fn ordering_respects_category() {
        let high = eval7_str("Ah9c", "7s5d3c2hJs");
        let pair = eval7_str("AhAs", "7s5d3c2hJc");
        let two_pair = eval7_str("AhAs", "7s7d3c2hJc");
        let trips = eval7_str("AhAs", "AcJd3c7hKs");
        let straight = eval7_str("9h8h", "7c6d5s2hJc");
        let flush = eval7_str("2h3h", "4h5h7h9sKs");
        let full_house = eval7_str("AhAs", "AcKdKs7h2c");
        let quads = eval7_str("AhAs", "AcAd3c7h2s");
        let straight_flush = eval7_str("9h8h", "7h6h5hKcQc");

        assert!(high < pair);
        assert!(pair < two_pair);
        assert!(two_pair < trips);
        assert!(trips < straight);
        assert!(straight < flush);
        assert!(flush < full_house);
        assert!(full_house < quads);
        assert!(quads < straight_flush);
    }

    #[test]
    fn eval_5_matches_eval_7_on_royal() {
        let r7 = eval7_str("AhKh", "QhJhTh2c3s");
        let five = [
            Card::parse("Ah").unwrap(),
            Card::parse("Kh").unwrap(),
            Card::parse("Qh").unwrap(),
            Card::parse("Jh").unwrap(),
            Card::parse("Th").unwrap(),
        ];
        let r5 = reference_eval_5(&five);
        assert_eq!(r7, r5);
    }

    #[test]
    fn pair_of_aces_kicker_ordering() {
        // AsAc + K Q J beats AsAc + K Q T beats AsAc + K Q 9 — the
        // third kicker matters. Build boards that give each hand a
        // pair of aces and tight kickers.
        let strong = eval7_str("AsAc", "KdQhJs2c3d");
        let mid = eval7_str("AsAc", "KdQhTs2c3d");
        let weak = eval7_str("AsAc", "KdQh9s2c3d");
        assert!(strong > mid, "{strong:?} vs {mid:?}");
        assert!(mid > weak, "{mid:?} vs {weak:?}");
    }
}
