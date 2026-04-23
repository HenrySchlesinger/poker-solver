//! Hand = 2 hole cards.

use crate::card::Card;

/// Two hole cards. By convention `[0]` is the higher rank (tie-break by
/// suit value), i.e., `cards[0].0 > cards[1].0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Hand(pub [Card; 2]);

impl Hand {
    /// Construct, canonicalizing card order.
    ///
    /// The resulting hand has `cards[0].0 > cards[1].0`. Panics in debug
    /// if `a == b` (two cards of the same identity cannot coexist).
    pub fn new(a: Card, b: Card) -> Self {
        debug_assert_ne!(a.0, b.0, "hand cannot contain two of the same card");
        if a.0 > b.0 {
            Hand([a, b])
        } else {
            Hand([b, a])
        }
    }

    /// Parse from notation like "AhKd".
    pub fn parse(s: &str) -> Option<Self> {
        if s.len() < 4 {
            return None;
        }
        // Cards are two single-byte ASCII chars each; split at byte index 2.
        if !s.is_char_boundary(2) {
            return None;
        }
        let (a, b) = s.split_at(2);
        let ca = Card::parse(a)?;
        let cb = Card::parse(b)?;
        if ca.0 == cb.0 {
            return None;
        }
        Some(Hand::new(ca, cb))
    }
}

impl std::fmt::Display for Hand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.0[0], self.0[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};

    #[test]
    fn canonicalization_sorts_higher_first() {
        let ah = Card::new(Rank::Ace, Suit::Hearts);
        let kd = Card::new(Rank::King, Suit::Diamonds);
        let h = Hand::new(kd, ah);
        assert_eq!(h.0[0], ah);
        assert_eq!(h.0[1], kd);
    }

    #[test]
    fn hand_canonicalization_ahkd_eq_kdah() {
        let ah = Card::new(Rank::Ace, Suit::Hearts);
        let kd = Card::new(Rank::King, Suit::Diamonds);
        assert_eq!(Hand::new(ah, kd), Hand::new(kd, ah));
    }

    #[test]
    fn parse_roundtrip_ahkd() {
        let h = Hand::parse("AhKd").unwrap();
        assert_eq!(format!("{h}"), "AhKd");
    }

    #[test]
    fn parse_roundtrip_reverse() {
        // "KdAh" parses and canonicalizes to "AhKd".
        let h = Hand::parse("KdAh").unwrap();
        assert_eq!(format!("{h}"), "AhKd");
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(Hand::parse("ahkd"), Hand::parse("AhKd"));
        assert_eq!(Hand::parse("AHKD"), Hand::parse("AhKd"));
    }

    #[test]
    fn parse_rejects_duplicate_card() {
        assert!(Hand::parse("AhAh").is_none());
    }

    #[test]
    fn parse_rejects_short_and_long_strings() {
        assert!(Hand::parse("").is_none());
        assert!(Hand::parse("Ah").is_none());
        assert!(Hand::parse("AhK").is_none());
        assert!(Hand::parse("AhKdQ").is_none());
    }

    #[test]
    fn pocket_pair_different_suits_ok() {
        // AhAs is a legal hand; only the identical u8 is rejected.
        let h = Hand::parse("AhAs").unwrap();
        // Spades > Hearts in our encoding (suit 3 > suit 2).
        assert_eq!(format!("{h}"), "AsAh");
    }
}
