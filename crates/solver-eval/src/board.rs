//! Board = 0 to 5 community cards.

use crate::card::Card;

/// Up to 5 community cards, plus a length byte.
///
/// Valid `len` values: 0 (preflop), 3 (flop), 4 (turn), 5 (river). Two
/// community cards is not a legal NLHE state; the constructors refuse it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Board {
    /// The 5 card slots. Values beyond `len` are undefined.
    pub cards: [Card; 5],
    /// Number of valid cards (0 = preflop, 3 = flop, 4 = turn, 5 = river).
    pub len: u8,
}

impl Board {
    /// Empty board (preflop).
    pub const fn empty() -> Self {
        Self {
            cards: [Card(0); 5],
            len: 0,
        }
    }

    /// Flop constructor.
    ///
    /// Panics in debug if any two cards are identical. Community cards
    /// are kept in deal order; we do NOT sort them here (dealing order
    /// matters for some solver uses; canonicalization lives in `iso`).
    pub fn flop(a: Card, b: Card, c: Card) -> Self {
        debug_assert!(
            a.0 != b.0 && a.0 != c.0 && b.0 != c.0,
            "flop cards must be distinct"
        );
        Self {
            cards: [a, b, c, Card(0), Card(0)],
            len: 3,
        }
    }

    /// Turn constructor: flop + one turn card.
    pub fn turn(a: Card, b: Card, c: Card, d: Card) -> Self {
        debug_assert!(
            a.0 != b.0 && a.0 != c.0 && a.0 != d.0 && b.0 != c.0 && b.0 != d.0 && c.0 != d.0,
            "turn cards must be distinct"
        );
        Self {
            cards: [a, b, c, d, Card(0)],
            len: 4,
        }
    }

    /// River constructor: all 5 community cards.
    pub fn river(a: Card, b: Card, c: Card, d: Card, e: Card) -> Self {
        let all = [a, b, c, d, e];
        debug_assert!(
            {
                // All 5 cards distinct.
                let mut ok = true;
                for i in 0..5 {
                    for j in (i + 1)..5 {
                        if all[i].0 == all[j].0 {
                            ok = false;
                        }
                    }
                }
                ok
            },
            "river cards must be distinct"
        );
        Self { cards: all, len: 5 }
    }

    /// Valid cards as a slice.
    pub fn as_slice(&self) -> &[Card] {
        &self.cards[..self.len as usize]
    }

    /// Parse from notation like "AhKh2s" (flop), "AhKh2sQc" (turn),
    /// "AhKh2sQc4d" (river), or empty string for preflop.
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty() {
            return Some(Board::empty());
        }
        if s.len() % 2 != 0 {
            return None;
        }
        let n = s.len() / 2;
        if !matches!(n, 3..=5) {
            return None;
        }
        let mut cards = [Card(0); 5];
        let bytes = s.as_bytes();
        // Each card is 2 ASCII chars; byte-indexing is safe for ASCII.
        for (i, slot) in cards.iter_mut().enumerate().take(n) {
            // Ensure the window is all ASCII so splitting on byte boundary
            // is safe.
            let start = i * 2;
            let end = start + 2;
            if !s.is_char_boundary(start) || !s.is_char_boundary(end) {
                return None;
            }
            let slice = std::str::from_utf8(&bytes[start..end]).ok()?;
            *slot = Card::parse(slice)?;
        }
        // Check distinctness.
        for i in 0..n {
            for j in (i + 1)..n {
                if cards[i].0 == cards[j].0 {
                    return None;
                }
            }
        }
        Some(Self {
            cards,
            len: n as u8,
        })
    }
}

impl std::fmt::Display for Board {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for c in self.as_slice() {
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_board_is_len_zero() {
        let b = Board::empty();
        assert_eq!(b.len, 0);
        assert_eq!(b.as_slice().len(), 0);
        assert_eq!(format!("{b}"), "");
    }

    #[test]
    fn parse_flop_roundtrip() {
        let b = Board::parse("AhKh2s").unwrap();
        assert_eq!(b.len, 3);
        assert_eq!(format!("{b}"), "AhKh2s");
    }

    #[test]
    fn parse_turn_roundtrip() {
        let b = Board::parse("AhKh2sQc").unwrap();
        assert_eq!(b.len, 4);
        assert_eq!(format!("{b}"), "AhKh2sQc");
    }

    #[test]
    fn parse_river_roundtrip() {
        let b = Board::parse("AhKh2sQc4d").unwrap();
        assert_eq!(b.len, 5);
        assert_eq!(format!("{b}"), "AhKh2sQc4d");
    }

    #[test]
    fn parse_empty_is_preflop() {
        let b = Board::parse("").unwrap();
        assert_eq!(b.len, 0);
    }

    #[test]
    fn parse_rejects_two_cards() {
        // Two community cards is not a valid street.
        assert!(Board::parse("AhKh").is_none());
    }

    #[test]
    fn parse_rejects_one_card() {
        assert!(Board::parse("Ah").is_none());
    }

    #[test]
    fn parse_rejects_six_cards() {
        assert!(Board::parse("AhKh2sQc4d5h").is_none());
    }

    #[test]
    fn parse_rejects_odd_length() {
        assert!(Board::parse("AhK").is_none());
        assert!(Board::parse("AhKh2").is_none());
    }

    #[test]
    fn parse_rejects_duplicate_card() {
        assert!(Board::parse("AhAh2s").is_none());
        assert!(Board::parse("AhKh2sAh").is_none());
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(Board::parse("ahkh2s"), Board::parse("AhKh2s"));
    }

    #[test]
    fn flop_constructor_matches_parse() {
        let parsed = Board::parse("AhKh2s").unwrap();
        let built = Board::flop(
            Card::parse("Ah").unwrap(),
            Card::parse("Kh").unwrap(),
            Card::parse("2s").unwrap(),
        );
        assert_eq!(parsed, built);
    }

    #[test]
    fn turn_and_river_constructors_produce_correct_len() {
        let t = Board::turn(
            Card::parse("Ah").unwrap(),
            Card::parse("Kh").unwrap(),
            Card::parse("2s").unwrap(),
            Card::parse("Qc").unwrap(),
        );
        assert_eq!(t.len, 4);

        let r = Board::river(
            Card::parse("Ah").unwrap(),
            Card::parse("Kh").unwrap(),
            Card::parse("2s").unwrap(),
            Card::parse("Qc").unwrap(),
            Card::parse("4d").unwrap(),
        );
        assert_eq!(r.len, 5);
    }
}
