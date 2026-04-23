//! Card / board isomorphism.
//!
//! Canonical-renaming of suits so that strategically-equivalent boards
//! map to the same key. Reduces the flop-cache size by ~12× on average,
//! more on paired/flushy boards.
//!
//! Scheme: walk the board (and optionally the hero/villain hands)
//! in deal order, assigning suit labels 0, 1, 2, 3 to each new suit
//! as it appears. The canonical board is the result of that relabeling.
//!
//! Poker is invariant under suit relabeling: "AsKs on JhTh4c" is
//! strategically identical to "AhKh on JsTs4d". Canonicalization picks
//! one representative per equivalence class so we can cache by form.

use crate::board::Board;
use crate::card::Card;

/// Sentinel for "this original suit hasn't been seen yet" in the
/// 4-slot renaming map. We use 0xFF because valid canonical suit
/// labels are in 0..4.
const SUIT_UNSET: u8 = 0xFF;

/// A suit-relabeling map: for each original suit `s` in 0..4,
/// `map[s as usize]` is the canonical label, or `SUIT_UNSET` if
/// the suit has not yet appeared.
type SuitMap = [u8; 4];

/// Assign a canonical label to `orig_suit` if it hasn't been assigned
/// yet. `next_label` is the next free label (0..4).
#[inline]
fn observe_suit(map: &mut SuitMap, next_label: &mut u8, orig_suit: u8) {
    debug_assert!(orig_suit < 4, "suit out of range: {orig_suit}");
    if map[orig_suit as usize] == SUIT_UNSET {
        map[orig_suit as usize] = *next_label;
        *next_label += 1;
    }
}

/// Apply `map` to `card`: keep its rank, replace its suit with the
/// canonical label. The map must have an assignment for the card's suit.
#[inline]
fn relabel(card: Card, map: &SuitMap) -> Card {
    let suit = (card.0 & 0b11) as usize;
    let canonical = map[suit];
    debug_assert!(
        canonical != SUIT_UNSET,
        "card {} references unmapped suit {}",
        card.0,
        suit,
    );
    // Preserve rank (high 6 bits), replace suit (low 2 bits).
    Card((card.0 & !0b11) | (canonical & 0b11))
}

/// Build a suit map from the board alone, in deal order.
#[inline]
fn build_map_from_board(board: &Board) -> SuitMap {
    let mut map: SuitMap = [SUIT_UNSET; 4];
    let mut next = 0u8;
    for i in 0..board.len as usize {
        observe_suit(&mut map, &mut next, board.cards[i].0 & 0b11);
    }
    map
}

/// Rewrite each valid card of `board` through `map`; undefined slots
/// stay at `Card(0)` so the struct's hashing/equality stays stable.
#[inline]
fn apply_map_to_board(board: &Board, map: &SuitMap) -> Board {
    let mut out = Board::empty();
    out.len = board.len;
    for i in 0..board.len as usize {
        out.cards[i] = relabel(board.cards[i], map);
    }
    out
}

/// Canonicalize a board — assign suit labels in order of first
/// appearance, then rewrite each card.
pub fn canonical_board(board: &Board) -> Board {
    let map = build_map_from_board(board);
    apply_map_to_board(board, &map)
}

/// Decode a combo index in `0..1326` into its pair of card bytes
/// `(a, b)` with `a < b`.
///
/// Uses the triangular-index formula documented in
/// `crate::combo`. Done locally here so `iso` doesn't have to wait
/// on (or race with) the `combo` module landing.
#[inline]
fn combo_from_index(idx: u16) -> (u8, u8) {
    debug_assert!(idx < 1326, "combo index out of range: {idx}");
    // start(a) = number of pairs (i, j) with i < j and i < a
    //         = sum_{i=0..a} (51 - i) = 51a - a(a-1)/2.
    // Find the largest a such that start(a) <= idx.
    let idx = idx as u32;
    let mut a = 0u32;
    let mut start = 0u32;
    loop {
        let next_start = start + (51 - a);
        if next_start > idx {
            break;
        }
        start = next_start;
        a += 1;
    }
    let b = idx - start + a + 1;
    debug_assert!(a < b && b < 52);
    (a as u8, b as u8)
}

/// Encode a pair of card bytes `(a, b)` with `a < b` into a combo
/// index in `0..1326`. Inverse of `combo_from_index`.
#[inline]
fn index_from_combo(a: u8, b: u8) -> u16 {
    debug_assert!(a < b && b < 52, "invalid pair: ({a}, {b})");
    let a = a as u32;
    let b = b as u32;
    let start = 51 * a - a * (a.wrapping_sub(1)) / 2;
    (start + b - a - 1) as u16
}

/// Relabel a card pair `(a, b)` through `map`, then re-sort so the
/// resulting pair is still `a' < b'` (so it's a valid combo).
#[inline]
fn relabel_pair(a: u8, b: u8, map: &SuitMap) -> (u8, u8) {
    let ra = relabel(Card(a), map).0;
    let rb = relabel(Card(b), map).0;
    if ra < rb { (ra, rb) } else { (rb, ra) }
}

/// Full canonicalization including hero + villain hole cards. Used for
/// cache lookup: two spots with the same canonical representation have
/// identical strategies.
///
/// Walk order for suit assignment:
///   1. Hero's 2 hole cards (lower card value first)
///   2. Villain's 2 hole cards
///   3. Board cards in deal order
pub fn canonical_spot(
    board: &Board,
    hero_combo_idx: u16,
    villain_combo_idx: u16,
) -> (Board, u16, u16) {
    let (h_a, h_b) = combo_from_index(hero_combo_idx);
    let (v_a, v_b) = combo_from_index(villain_combo_idx);

    let mut map: SuitMap = [SUIT_UNSET; 4];
    let mut next = 0u8;

    // 1. Hero hole cards, ascending.
    observe_suit(&mut map, &mut next, h_a & 0b11);
    observe_suit(&mut map, &mut next, h_b & 0b11);
    // 2. Villain hole cards, ascending.
    observe_suit(&mut map, &mut next, v_a & 0b11);
    observe_suit(&mut map, &mut next, v_b & 0b11);
    // 3. Board cards, deal order.
    for i in 0..board.len as usize {
        observe_suit(&mut map, &mut next, board.cards[i].0 & 0b11);
    }

    let new_board = apply_map_to_board(board, &map);
    let (nh_a, nh_b) = relabel_pair(h_a, h_b, &map);
    let (nv_a, nv_b) = relabel_pair(v_a, v_b, &map);
    let new_hero = index_from_combo(nh_a, nh_b);
    let new_villain = index_from_combo(nv_a, nv_b);

    (new_board, new_hero, new_villain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    // Helpers ----------------------------------------------------------

    fn c(rank: Rank, suit: Suit) -> Card {
        Card::new(rank, suit)
    }

    fn flop(a: Card, b: Card, cc: Card) -> Board {
        let mut out = Board::empty();
        out.cards[0] = a;
        out.cards[1] = b;
        out.cards[2] = cc;
        out.len = 3;
        out
    }

    fn river(a: Card, b: Card, cc: Card, d: Card, e: Card) -> Board {
        let mut out = Board::empty();
        out.cards[0] = a;
        out.cards[1] = b;
        out.cards[2] = cc;
        out.cards[3] = d;
        out.cards[4] = e;
        out.len = 5;
        out
    }

    // canonical_board -------------------------------------------------

    #[test]
    fn already_canonical_stays_the_same() {
        // Jc Tc 4d: suit 0 (Clubs) then suit 1 (Diamonds) — already canonical.
        let b = flop(
            c(Rank::Jack, Suit::Clubs),
            c(Rank::Ten, Suit::Clubs),
            c(Rank::Four, Suit::Diamonds),
        );
        assert_eq!(canonical_board(&b), b);
    }

    #[test]
    fn equivalent_boards_canonicalize_to_same_form() {
        // JhTh4c ≡ JsTs4d ≡ JcTc4d (all "two matching + one different" flops
        // with the same ranks collapse to the same canonical rep).
        let hh_c = flop(
            c(Rank::Jack, Suit::Hearts),
            c(Rank::Ten, Suit::Hearts),
            c(Rank::Four, Suit::Clubs),
        );
        let ss_d = flop(
            c(Rank::Jack, Suit::Spades),
            c(Rank::Ten, Suit::Spades),
            c(Rank::Four, Suit::Diamonds),
        );
        let cc_d = flop(
            c(Rank::Jack, Suit::Clubs),
            c(Rank::Ten, Suit::Clubs),
            c(Rank::Four, Suit::Diamonds),
        );
        let x = canonical_board(&hh_c);
        let y = canonical_board(&ss_d);
        let z = canonical_board(&cc_d);
        assert_eq!(x, y);
        assert_eq!(y, z);
        // And the canonical form uses suit 0 and suit 1 only.
        for i in 0..x.len as usize {
            let s = x.cards[i].suit() as u8;
            assert!(s < 2, "canonical form should use suits 0/1 only, got {s}");
        }
    }

    #[test]
    fn idempotent() {
        let b = flop(
            c(Rank::Ace, Suit::Hearts),
            c(Rank::King, Suit::Diamonds),
            c(Rank::Two, Suit::Spades),
        );
        let once = canonical_board(&b);
        let twice = canonical_board(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn monotone_flop_canonicalizes_to_all_suit_zero() {
        // All Hearts → should all become suit 0 (Clubs).
        let b = flop(
            c(Rank::Ace, Suit::Hearts),
            c(Rank::Seven, Suit::Hearts),
            c(Rank::Two, Suit::Hearts),
        );
        let out = canonical_board(&b);
        for i in 0..out.len as usize {
            assert_eq!(out.cards[i].suit(), Suit::Clubs);
        }
    }

    #[test]
    fn rainbow_flop_canonicalizes_to_suits_0_1_2_in_order() {
        // Input suits: Hearts, Spades, Diamonds (in that deal order).
        // They should map to 0, 1, 2.
        let b = flop(
            c(Rank::Ace, Suit::Hearts),    // first new → 0
            c(Rank::King, Suit::Spades),   // second new → 1
            c(Rank::Two, Suit::Diamonds),  // third new → 2
        );
        let out = canonical_board(&b);
        assert_eq!(out.cards[0].suit() as u8, 0);
        assert_eq!(out.cards[1].suit() as u8, 1);
        assert_eq!(out.cards[2].suit() as u8, 2);
    }

    #[test]
    fn preserves_ranks() {
        let b = flop(
            c(Rank::Queen, Suit::Spades),
            c(Rank::Six, Suit::Diamonds),
            c(Rank::Three, Suit::Clubs),
        );
        let out = canonical_board(&b);
        assert_eq!(out.cards[0].rank(), Rank::Queen);
        assert_eq!(out.cards[1].rank(), Rank::Six);
        assert_eq!(out.cards[2].rank(), Rank::Three);
    }

    #[test]
    fn preserves_board_length() {
        // Turn: 4 cards.
        let mut turn = Board::empty();
        turn.cards[0] = c(Rank::Ace, Suit::Hearts);
        turn.cards[1] = c(Rank::King, Suit::Hearts);
        turn.cards[2] = c(Rank::Two, Suit::Spades);
        turn.cards[3] = c(Rank::Queen, Suit::Clubs);
        turn.len = 4;
        let out = canonical_board(&turn);
        assert_eq!(out.len, 4);

        // River: 5 cards.
        let rv = river(
            c(Rank::Ace, Suit::Hearts),
            c(Rank::King, Suit::Hearts),
            c(Rank::Two, Suit::Spades),
            c(Rank::Queen, Suit::Clubs),
            c(Rank::Four, Suit::Diamonds),
        );
        let out = canonical_board(&rv);
        assert_eq!(out.len, 5);
    }

    #[test]
    fn empty_preflop_board_is_a_noop() {
        let b = Board::empty();
        assert_eq!(canonical_board(&b), b);
    }

    #[test]
    fn permuting_suits_preserves_canonical_form() {
        // Start with JhTs4d — a rainbow flop.
        let original = flop(
            c(Rank::Jack, Suit::Hearts),
            c(Rank::Ten, Suit::Spades),
            c(Rank::Four, Suit::Diamonds),
        );
        // Permute: H→C, S→D, D→H (3-cycle).
        let permuted = flop(
            c(Rank::Jack, Suit::Clubs),
            c(Rank::Ten, Suit::Diamonds),
            c(Rank::Four, Suit::Hearts),
        );
        assert_eq!(canonical_board(&original), canonical_board(&permuted));
    }

    // Reduction-factor sanity check ----------------------------------

    /// Iterate over every unordered flop (a < b < c by u8 value) — the
    /// standard 22,100 = C(52, 3). Count distinct canonical forms under
    /// our first-appearance scheme.
    ///
    /// Result: **1,911** distinct canonical flops (≈11.6× reduction).
    ///
    /// The literature's 1,755 figure (≈12.6× reduction) is the
    /// theoretical minimum under suit isomorphism. Reaching it requires
    /// extra logic — e.g. picking the lexicographically smallest
    /// labeling when a pair is present, so that `{2c, 2d, 3c}` and
    /// `{2c, 2d, 3d}` collapse into a single form. Our simpler greedy
    /// labeling keeps those separate (they differ in which pair-suit
    /// the singleton shares). The excess is exactly 13 × 12 = 156 extra
    /// classes from the pair-plus-singleton case.
    ///
    /// For v0.1 the greedy form is fine: 1,911 still gives >11× cache-
    /// size collapse; bumping to 1,755 is a later optimization.
    #[test]
    fn reduction_factor_collapses_22100_flops() {
        use std::collections::HashSet;
        let mut seen: HashSet<Board> = HashSet::new();
        let mut total = 0u32;
        for a in 0..52u8 {
            for b in (a + 1)..52u8 {
                for cc in (b + 1)..52u8 {
                    total += 1;
                    let b_ = flop(Card(a), Card(b), Card(cc));
                    seen.insert(canonical_board(&b_));
                }
            }
        }
        assert_eq!(total, 22_100);
        assert_eq!(seen.len(), 1_911);
    }

    // combo_from_index / index_from_combo round-trip -----------------

    #[test]
    fn combo_index_roundtrips_all_1326() {
        for idx in 0..1326u16 {
            let (a, b) = combo_from_index(idx);
            assert!(a < b);
            assert_eq!(index_from_combo(a, b), idx);
        }
    }

    #[test]
    fn combo_index_edges() {
        // First pair: (0, 1) → 0.
        assert_eq!(combo_from_index(0), (0, 1));
        // Last pair: (50, 51) → 1325.
        assert_eq!(combo_from_index(1325), (50, 51));
        assert_eq!(index_from_combo(0, 1), 0);
        assert_eq!(index_from_combo(50, 51), 1325);
    }

    // canonical_spot --------------------------------------------------

    fn combo_idx(a: Card, b: Card) -> u16 {
        let (lo, hi) = if a.0 < b.0 { (a.0, b.0) } else { (b.0, a.0) };
        index_from_combo(lo, hi)
    }

    #[test]
    fn canonical_spot_collapses_suit_relabeling() {
        // Spot A: hero AsKs on board JhTh4c, villain 5d5h.
        let board_a = flop(
            c(Rank::Jack, Suit::Hearts),
            c(Rank::Ten, Suit::Hearts),
            c(Rank::Four, Suit::Clubs),
        );
        let hero_a = combo_idx(
            c(Rank::Ace, Suit::Spades),
            c(Rank::King, Suit::Spades),
        );
        let villain_a = combo_idx(
            c(Rank::Five, Suit::Diamonds),
            c(Rank::Five, Suit::Hearts),
        );

        // Spot B: the same structure with suits rotated — hero AhKh on
        // board JsTs4d, villain 5c5s.
        let board_b = flop(
            c(Rank::Jack, Suit::Spades),
            c(Rank::Ten, Suit::Spades),
            c(Rank::Four, Suit::Diamonds),
        );
        let hero_b = combo_idx(
            c(Rank::Ace, Suit::Hearts),
            c(Rank::King, Suit::Hearts),
        );
        let villain_b = combo_idx(
            c(Rank::Five, Suit::Clubs),
            c(Rank::Five, Suit::Spades),
        );

        let a = canonical_spot(&board_a, hero_a, villain_a);
        let b = canonical_spot(&board_b, hero_b, villain_b);
        assert_eq!(a, b);
    }

    #[test]
    fn canonical_spot_is_idempotent() {
        let board = flop(
            c(Rank::Jack, Suit::Hearts),
            c(Rank::Ten, Suit::Spades),
            c(Rank::Four, Suit::Diamonds),
        );
        let hero = combo_idx(
            c(Rank::Ace, Suit::Clubs),
            c(Rank::King, Suit::Hearts),
        );
        let villain = combo_idx(
            c(Rank::Five, Suit::Spades),
            c(Rank::Two, Suit::Diamonds),
        );

        let (b1, h1, v1) = canonical_spot(&board, hero, villain);
        let (b2, h2, v2) = canonical_spot(&b1, h1, v1);
        assert_eq!(b1, b2);
        assert_eq!(h1, h2);
        assert_eq!(v1, v2);
    }

    #[test]
    fn canonical_spot_preserves_ranks_everywhere() {
        let board = flop(
            c(Rank::Queen, Suit::Spades),
            c(Rank::Seven, Suit::Diamonds),
            c(Rank::Three, Suit::Hearts),
        );
        let hero = combo_idx(
            c(Rank::Ace, Suit::Clubs),
            c(Rank::Jack, Suit::Hearts),
        );
        let villain = combo_idx(
            c(Rank::Eight, Suit::Diamonds),
            c(Rank::Eight, Suit::Spades),
        );

        let (out_board, out_hero, out_villain) = canonical_spot(&board, hero, villain);

        // Board ranks preserved in order.
        assert_eq!(out_board.cards[0].rank(), Rank::Queen);
        assert_eq!(out_board.cards[1].rank(), Rank::Seven);
        assert_eq!(out_board.cards[2].rank(), Rank::Three);

        // Hero ranks are {A, J} regardless of sort order.
        let (hlo, hhi) = combo_from_index(out_hero);
        let mut hero_ranks = [Card(hlo).rank(), Card(hhi).rank()];
        hero_ranks.sort();
        assert_eq!(hero_ranks, [Rank::Jack, Rank::Ace]);

        // Villain ranks are {8, 8}.
        let (vlo, vhi) = combo_from_index(out_villain);
        assert_eq!(Card(vlo).rank(), Rank::Eight);
        assert_eq!(Card(vhi).rank(), Rank::Eight);
    }
}
