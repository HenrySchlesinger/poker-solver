//! Combo indexing: bijection between unordered `(Card, Card)` pairs and
//! the range `0..1326`.
//!
//! A "combo" is one specific 2-card hand (e.g., AhKh). There are
//! `C(52, 2) = 1326` of them. A `Range` in `solver-nlhe` is a `[f32; 1326]`
//! indexed by combo number, so this bijection is load-bearing for every
//! range lookup in the solver.
//!
//! # Convention
//!
//! For any pair `(a, b)` we canonicalize to `a.0 < b.0` (i.e., the smaller
//! u8 is the "first" card). Then:
//!
//! ```text
//! combo_index(a, b) = 51 * a - a * (a - 1) / 2 + (b - a - 1)
//! ```
//!
//! Derivation: the number of pairs whose first card has value `< a` is
//! `sum_{i=0..a} (51 - i) = 51*a - a*(a-1)/2`. Within a fixed first card
//! `a`, the pair with second card `b` occupies slot `b - a - 1` (so the
//! very next value above `a` is slot 0).
//!
//! The first pair (0, 1) maps to index 0; the last pair (50, 51) maps
//! to 1325.
//!
//! This is a direct "lower-triangular packing" of the 52×52 matrix with
//! diagonal and lower triangle excluded.
//!
//! # Note on the task-brief formula
//!
//! The original task brief cited the formula
//! `a*51 - a*(a+1)/2 + b - a - 1`, which collides (e.g., `(0, 51)` and
//! `(1, 2)` both map to 50). The correct formula is
//! `a*(a-1)/2`. We use the corrected version; it is the formula that
//! satisfies the bijection success criterion.

use crate::card::Card;

/// Total number of distinct 2-card combos (C(52, 2)).
pub const NUM_COMBOS: usize = 1326;

/// Map an unordered pair of distinct cards to a combo index in
/// `0..NUM_COMBOS`. Input ordering does not matter; `combo_index(a, b) ==
/// combo_index(b, a)`.
///
/// Panics in debug if `a == b`.
#[inline]
pub fn combo_index(a: Card, b: Card) -> usize {
    debug_assert_ne!(a.0, b.0, "combo requires two distinct cards");
    let (lo, hi) = if a.0 < b.0 {
        (a.0 as usize, b.0 as usize)
    } else {
        (b.0 as usize, a.0 as usize)
    };
    lo * 51 - lo * lo.wrapping_sub(1) / 2 + (hi - lo - 1)
}

/// Inverse: given a combo index in `0..NUM_COMBOS`, recover the canonical
/// `(low, high)` card pair.
///
/// Panics if `idx >= NUM_COMBOS`.
#[inline]
pub fn index_to_combo(idx: usize) -> (Card, Card) {
    assert!(idx < NUM_COMBOS, "combo index out of range");
    // For each `a` in 0..51, the slice of indices with first-card == a
    // starts at `51*a - a*(a-1)/2` and has length `51 - a`. We walk
    // outward until we find the containing slice.
    //
    // 52 iterations max; branchless search is not worth it here.
    let mut base: usize = 0;
    for a in 0..51usize {
        let row_len = 51 - a;
        if idx < base + row_len {
            let offset = idx - base;
            let b = a + 1 + offset;
            return (Card(a as u8), Card(b as u8));
        }
        base += row_len;
    }
    unreachable!("idx < NUM_COMBOS should have been caught above");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn num_combos_is_1326() {
        assert_eq!(NUM_COMBOS, 52 * 51 / 2);
    }

    #[test]
    fn first_and_last_index_match_spec() {
        assert_eq!(combo_index(Card(0), Card(1)), 0);
        assert_eq!(combo_index(Card(50), Card(51)), NUM_COMBOS - 1);
    }

    #[test]
    fn index_is_symmetric_in_arguments() {
        for a in 0..52u8 {
            for b in (a + 1)..52 {
                assert_eq!(combo_index(Card(a), Card(b)), combo_index(Card(b), Card(a)));
            }
        }
    }

    #[test]
    fn combo_bijection_over_all_indices() {
        // Round-trip: index -> combo -> index is the identity on
        // 0..1326.
        for i in 0..NUM_COMBOS {
            let (a, b) = index_to_combo(i);
            assert!(a.0 < b.0, "index_to_combo must return (lo, hi)");
            assert_eq!(combo_index(a, b), i);
        }
    }

    #[test]
    fn combo_injectivity_over_all_pairs() {
        // Every canonical (a, b) with a < b maps to a distinct index in
        // the full 0..1326 range. This is the "no collisions" check
        // called out in the task brief.
        let mut seen = vec![false; NUM_COMBOS];
        let mut count = 0usize;
        for a in 0..52u8 {
            for b in (a + 1)..52 {
                let idx = combo_index(Card(a), Card(b));
                assert!(idx < NUM_COMBOS, "index {idx} out of range");
                assert!(!seen[idx], "collision at index {idx}");
                seen[idx] = true;
                count += 1;
            }
        }
        assert_eq!(count, NUM_COMBOS);
        assert!(seen.iter().all(|x| *x));
    }
}
