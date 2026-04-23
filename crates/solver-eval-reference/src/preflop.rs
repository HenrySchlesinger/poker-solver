//! Ported from `engine/rules/equity_optimized.py::normalize_hand` and
//! the canonical 169-hand enumeration in
//! `scripts/generate_preflop_table.py::build_canonical_hands`.
//!
//! "169 hands" is the classic preflop abstraction: a concrete hand
//! (52·51/2 = 1326 combos) collapses to one of 169 canonical classes —
//! 13 pocket pairs, 78 suited hands, 78 offsuit hands. The solver's
//! preflop range format ships this way (a 169-key dict), so having
//! a clean port of Poker Panel's canonicalizer lets us cross-check
//! the `combo_index` → "169-class" mapping if one gets added.

use solver_eval::card::Rank;
use solver_eval::Hand;

/// Canonical string representation of a 169-class hand.
///
/// * Pair: two identical rank chars, e.g. `"AA"`, `"22"`.
/// * Suited: two distinct rank chars (higher first) + `"s"`, e.g. `"AKs"`.
/// * Offsuit: two distinct rank chars (higher first) + `"o"`, e.g. `"AKo"`.
///
/// Matches the output shape of Poker Panel's `normalize_hand(cards)`
/// except:
/// * We return an owned `String` rather than mutating in place.
/// * We take a typed `Hand` rather than `List[str]` — so the "parse
///   card strings" step is already done by `Hand::parse`.
pub fn reference_normalize_hand_169(hand: &Hand) -> String {
    // Python:
    //     rank1, suit1 = card1[:-1], card1[-1]
    //     rank2, suit2 = card2[:-1], card2[-1]
    //     if idx1 > idx2: swap   # put higher-ranked card first
    //
    // Our `Hand::new` already puts the higher-u8 card at index 0.
    // In our encoding (rank<<2|suit) the u8 is monotonic in rank
    // (ignoring suit, which is in the low bits), so `hand.0[0]` is
    // the same-or-higher rank as `hand.0[1]`. That matches the
    // Python's post-swap invariant for rank ordering. The one edge:
    // if both cards have the *same* rank (pocket pair), they'll be
    // sorted by suit, but that doesn't change the output category
    // (both branches go through the "pair" case).
    let a = hand.0[0];
    let b = hand.0[1];
    let r_a: Rank = a.rank();
    let r_b: Rank = b.rank();

    let ch_a = r_a.to_char();
    let ch_b = r_b.to_char();

    if r_a == r_b {
        // Pair. Python: `return rank1 + rank2`.
        let mut s = String::with_capacity(2);
        s.push(ch_a);
        s.push(ch_b);
        s
    } else if a.suit() == b.suit() {
        // Suited.
        let mut s = String::with_capacity(3);
        s.push(ch_a);
        s.push(ch_b);
        s.push('s');
        s
    } else {
        // Offsuit.
        let mut s = String::with_capacity(3);
        s.push(ch_a);
        s.push(ch_b);
        s.push('o');
        s
    }
}

/// All 169 canonical hand strings. Ported from
/// `scripts/generate_preflop_table.py::build_canonical_hands`:
///
/// ```python
/// RANKS = "AKQJT98765432"
/// for i, r1 in enumerate(RANKS):
///     for j, r2 in enumerate(RANKS):
///         if i < j:   hands += [r1+r2+'s', r1+r2+'o']
///         elif i == j: hands += [r1+r2]
/// ```
///
/// Note: the Python uses Ace-high first (`"AKQJT98765432"`). Our
/// `Rank` enum uses Ace-high *last* (0=Two, 12=Ace). We iterate in
/// the same order the Python does (Ace first) so the resulting vector
/// matches index-for-index.
pub fn reference_build_canonical_hands_169() -> Vec<String> {
    const RANK_CHARS: [char; 13] = [
        'A', 'K', 'Q', 'J', 'T', '9', '8', '7', '6', '5', '4', '3', '2',
    ];
    let mut hands = Vec::with_capacity(169);
    for i in 0..13 {
        for j in 0..13 {
            use std::cmp::Ordering;
            match i.cmp(&j) {
                Ordering::Less => {
                    // suited then offsuit, matching Python's order
                    let mut s = String::new();
                    s.push(RANK_CHARS[i]);
                    s.push(RANK_CHARS[j]);
                    s.push('s');
                    hands.push(s);
                    let mut o = String::new();
                    o.push(RANK_CHARS[i]);
                    o.push(RANK_CHARS[j]);
                    o.push('o');
                    hands.push(o);
                }
                Ordering::Equal => {
                    let mut p = String::new();
                    p.push(RANK_CHARS[i]);
                    p.push(RANK_CHARS[j]);
                    hands.push(p);
                }
                Ordering::Greater => {
                    // Skipped in Python: `if i > j: nothing` — lower-
                    // ranked first card is redundant with upper-
                    // triangle entries we already emitted.
                }
            }
        }
    }
    hands
}

#[cfg(test)]
mod tests {
    use super::*;
    use solver_eval::Card;

    #[test]
    fn pocket_aces_normalize_to_aa() {
        let h = Hand::parse("AsAc").unwrap();
        assert_eq!(reference_normalize_hand_169(&h), "AA");
    }

    #[test]
    fn ak_suited_hearts() {
        let h = Hand::parse("AhKh").unwrap();
        assert_eq!(reference_normalize_hand_169(&h), "AKs");
    }

    #[test]
    fn ak_offsuit() {
        let h = Hand::parse("AhKs").unwrap();
        assert_eq!(reference_normalize_hand_169(&h), "AKo");
    }

    #[test]
    fn order_is_higher_first_regardless_of_input() {
        // Hand::parse canonicalizes internally.
        let h1 = Hand::parse("2hAd").unwrap();
        let h2 = Hand::parse("Ad2h").unwrap();
        assert_eq!(reference_normalize_hand_169(&h1), "A2o");
        assert_eq!(reference_normalize_hand_169(&h2), "A2o");
    }

    #[test]
    fn canonical_hands_has_exactly_169_entries() {
        let hs = reference_build_canonical_hands_169();
        assert_eq!(hs.len(), 169);
        // 13 pairs + 78 suited + 78 offsuit = 169.
        let pairs = hs.iter().filter(|s| s.len() == 2).count();
        let suited = hs.iter().filter(|s| s.ends_with('s')).count();
        let offsuit = hs.iter().filter(|s| s.ends_with('o')).count();
        assert_eq!(pairs, 13);
        assert_eq!(suited, 78);
        assert_eq!(offsuit, 78);
    }

    #[test]
    fn every_concrete_combo_maps_to_some_canonical_class() {
        let canonical: std::collections::HashSet<String> =
            reference_build_canonical_hands_169().into_iter().collect();
        for a in 0..52u8 {
            for b in (a + 1)..52 {
                let h = Hand::new(Card(a), Card(b));
                let class = reference_normalize_hand_169(&h);
                assert!(
                    canonical.contains(&class),
                    "class {class} not in canonical set for hand {h}",
                );
            }
        }
    }
}
