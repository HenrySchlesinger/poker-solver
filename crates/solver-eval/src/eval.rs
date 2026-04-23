//! 5-card and 7-card hand evaluator.
//!
//! # Evaluator choice: `rs_poker` (v4.1)
//!
//! Agent A2 (Day 1) picked `rs_poker` over the alternatives (`pokerlookup`,
//! a hand-rolled Cactus-Kev port) because:
//!
//! * Pure Rust, no C deps or build scripts — cross-compiles to whatever
//!   Poker Panel's `solver-ffi` target ends up being without fuss.
//! * Actively maintained (4.x line, Rust 2024 edition, on crates.io as of
//!   2026-04-22).
//! * Exposes exactly the two entry points we need: `Rankable::rank()` for
//!   the 7-card hot path, and `Rankable::rank_five()` for the 5-card
//!   fast-path microbench.
//! * `rs_poker::core::Rank` is a `PartialOrd`+`Ord` enum whose variant
//!   order is hand-strength order (`HighCard` < … < `StraightFlush`),
//!   with a `u32` inner field that breaks ties within a variant. So
//!   re-packing it into a single monotonic `u32` is a shift-and-or.
//! * Apache-2.0 license, fine for bundling in our proprietary binary.
//!
//! We pulled in `rs_poker` with `default-features = false` to skip the
//! `arena` (simulation harness — huge, needs nightly features) and
//! `serde` defaults. See `Cargo.toml`.
//!
//! # Encoding mapping
//!
//! Our `Card(u8)` is `rank << 2 | suit`, rank ∈ 0..13, suit ∈ 0..4.
//! `rs_poker::core::Card` is a struct with `value: Value, suit: Suit`
//! where both are `#[repr(u8)]` enums covering exactly those ranges
//! (Two=0..Ace=12 on rank; 0..3 on suit).
//!
//! The *numeric* mapping of suits differs between the two libraries
//! (rs_poker: Spade=0, Club=1, Heart=2, Diamond=3; ours: Clubs=0,
//! Diamonds=1, Hearts=2, Spades=3), but that is irrelevant to hand
//! evaluation — what matters is which cards share a suit, which is
//! preserved by any bijection. So we just pass the low 2 bits through
//! unchanged.

use rs_poker::core::{Card as RsCard, Rank as RsRank, Rankable, Suit as RsSuit, Value as RsValue};

use crate::board::Board;
use crate::card::Card;
use crate::hand::Hand;

/// Opaque hand-rank value. Higher = stronger hand.
///
/// Layout: top 4 bits carry the rank *category* (0 = HighCard …
/// 8 = StraightFlush — identical ordering to `rs_poker::core::Rank`'s
/// enum discriminants). The low 28 bits carry the within-category
/// discriminator that `rs_poker` returns, which is itself monotonic
/// within the category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HandRank(pub u32);

impl HandRank {
    /// Number of bits reserved for the within-category discriminator.
    /// `rs_poker`'s inner `u32` uses at most `((1 << 12) << 13) | ((1 << 13) - 1)`
    /// ≈ 2^25, so 28 bits gives comfortable headroom.
    const INNER_BITS: u32 = 28;

    #[inline]
    fn from_rs(rank: RsRank) -> Self {
        // Category tag — higher = stronger. Must match `rs_poker::core::Rank`
        // variant order. We re-derive it here rather than relying on
        // `mem::discriminant` because we want a stable numeric tag
        // that survives an rs_poker major-version bump.
        let (category, inner) = match rank {
            RsRank::HighCard(v) => (0u32, v),
            RsRank::OnePair(v) => (1, v),
            RsRank::TwoPair(v) => (2, v),
            RsRank::ThreeOfAKind(v) => (3, v),
            RsRank::Straight(v) => (4, v),
            RsRank::Flush(v) => (5, v),
            RsRank::FullHouse(v) => (6, v),
            RsRank::FourOfAKind(v) => (7, v),
            RsRank::StraightFlush(v) => (8, v),
        };
        // Defensive mask — if rs_poker's inner u32 ever grows past 28 bits
        // we want a debug-build panic rather than a silent alias into the
        // category tag.
        debug_assert!(
            inner < (1 << Self::INNER_BITS),
            "rs_poker inner rank overflowed 28 bits",
        );
        HandRank((category << Self::INNER_BITS) | (inner & ((1 << Self::INNER_BITS) - 1)))
    }
}

/// Translate our `Card(u8)` into `rs_poker::core::Card`.
///
/// Branch-light: one shift, one mask, two `From<u8>` conversions. In
/// rs_poker those are implemented as `transmute(min(v, MAX))` — one
/// compare + one transmute each. The compiler inlines this to a handful
/// of arithmetic ops in release mode.
///
/// We go through rs_poker's `From<u8>` rather than a bare `transmute`
/// because `Card(pub u8)` is publicly constructible with arbitrary
/// bytes — so we can't assume upstream validity. The `min` clamp is
/// cheap and keeps us sound against a `Card(255)` smuggled in from a
/// buggy caller.
#[inline(always)]
fn to_rs(card: Card) -> RsCard {
    let rank = card.0 >> 2;
    let suit = card.0 & 0b11;
    RsCard {
        value: RsValue::from(rank),
        suit: RsSuit::from(suit),
    }
}

/// Evaluate the best 5-card hand from 7 cards (2 hole + 5 board).
///
/// This is the hot path — called ~10^7 times per river solve. Keep it
/// inlineable and branch-light.
///
/// Preconditions:
/// * `board.len == 5` — this function assumes a full river board. On
///   earlier streets, use the equity calculator which samples runouts.
#[inline]
pub fn eval_7(hand: &Hand, board: &Board) -> HandRank {
    debug_assert_eq!(board.len, 5, "eval_7 requires a full 5-card (river) board");
    let cards: [RsCard; 7] = [
        to_rs(hand.0[0]),
        to_rs(hand.0[1]),
        to_rs(board.cards[0]),
        to_rs(board.cards[1]),
        to_rs(board.cards[2]),
        to_rs(board.cards[3]),
        to_rs(board.cards[4]),
    ];
    HandRank::from_rs(cards.as_slice().rank())
}

/// Evaluate exactly 5 cards. For the river microbench and tests; not
/// the hot path during solves (CFR always has 7).
#[inline]
pub fn eval_5(cards: &[Card; 5]) -> HandRank {
    let rs_cards: [RsCard; 5] = [
        to_rs(cards[0]),
        to_rs(cards[1]),
        to_rs(cards[2]),
        to_rs(cards[3]),
        to_rs(cards[4]),
    ];
    HandRank::from_rs(rs_cards.as_slice().rank_five())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: evaluate a hand-string like "AhKh" on a board-string like
    /// "QhJhTh2c3s".
    fn eval7_str(hand: &str, board: &str) -> HandRank {
        let h = Hand::parse(hand).unwrap_or_else(|| panic!("bad hand: {hand}"));
        let b = Board::parse(board).unwrap_or_else(|| panic!("bad board: {board}"));
        assert_eq!(b.len, 5, "eval_7 requires a river board");
        eval_7(&h, &b)
    }

    #[test]
    fn flush_beats_straight() {
        // Hero: 2d 3d on 4d 5d 6d 9s Ks → 6-high diamond flush.
        let flush = eval7_str("2d3d", "4d5d6d9sKs");
        // Villain: 8c 9c on 5d 6h 7s Th Qs → 9-high straight.
        let straight = eval7_str("8c9c", "5d6h7sThQs");
        assert!(
            flush > straight,
            "flush ({flush:?}) should beat straight ({straight:?})",
        );
    }

    #[test]
    fn full_house_beats_trips() {
        // Full house: AsAc on Ad Kh Kd 2c 7s → aces full of kings.
        let full_house = eval7_str("AsAc", "AdKhKd2c7s");
        // Trips: AsAc on Ad 5h 9d 2c 7s → set of aces, no pair on board.
        let trips = eval7_str("AsAc", "Ad5h9d2c7s");
        assert!(
            full_house > trips,
            "full house ({full_house:?}) should beat trips ({trips:?})",
        );
    }

    #[test]
    fn royal_flush_is_top_rank() {
        // AhKh on Qh Jh Th 2c 3s → royal flush in hearts.
        let royal = eval7_str("AhKh", "QhJhTh2c3s");
        // Four of a kind aces as our strongest-but-still-below-royal ref.
        let quads = eval7_str("AsAc", "AdAhKc2s3d");
        assert!(
            royal > quads,
            "royal flush ({royal:?}) should beat quads ({quads:?})",
        );

        // Category tag in the top bits should be 8 (StraightFlush).
        let category = royal.0 >> HandRank::INNER_BITS;
        assert_eq!(
            category, 8,
            "royal flush should sit in StraightFlush category"
        );
    }

    #[test]
    fn eval_5_matches_eval_7_when_best_5_is_the_hole_plus_flop() {
        // On AhKhQhJhTh + 2c 3s, eval_7's best 5 is the royal flush, and
        // eval_5 on those same 5 hearts should produce the same HandRank.
        let r7 = eval7_str("AhKh", "QhJhTh2c3s");

        let five = [
            Card::parse("Ah").unwrap(),
            Card::parse("Kh").unwrap(),
            Card::parse("Qh").unwrap(),
            Card::parse("Jh").unwrap(),
            Card::parse("Th").unwrap(),
        ];
        let r5 = eval_5(&five);
        assert_eq!(r7, r5);
    }

    #[test]
    fn hand_rank_ordering_respects_category() {
        // Spot-check: a high card is weaker than a straight which is
        // weaker than a flush which is weaker than a full house which is
        // weaker than quads which is weaker than a straight flush.
        let high = eval7_str("Ah9c", "7s5d3c2hJs"); // A-high
        let pair = eval7_str("AhAs", "7s5d3c2hJc"); // aces
        let two_pair = eval7_str("AhAs", "7s7d3c2hJc"); // aces + sevens
        let trips = eval7_str("AhAs", "AcJd3c7hKs"); // set of aces
        let straight = eval7_str("9h8h", "7c6d5s2hJc"); // 9-high straight
        let flush = eval7_str("2h3h", "4h5h7h9sKs"); // heart flush
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
}
