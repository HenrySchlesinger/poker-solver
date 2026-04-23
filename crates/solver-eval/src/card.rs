//! Card representation.
//!
//! Encoding: `u8` where:
//!   - `rank = card >> 2`    (0..13, 0 = deuce, 12 = ace)
//!   - `suit = card & 0b11`  (0..4, see `Suit`)
//!
//! Valid range: 0..52. Use `Card::new(rank, suit)` to construct safely.

/// A playing-card rank, 2 through Ace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Rank {
    /// 2
    Two = 0,
    /// 3
    Three = 1,
    /// 4
    Four = 2,
    /// 5
    Five = 3,
    /// 6
    Six = 4,
    /// 7
    Seven = 5,
    /// 8
    Eight = 6,
    /// 9
    Nine = 7,
    /// T
    Ten = 8,
    /// J
    Jack = 9,
    /// Q
    Queen = 10,
    /// K
    King = 11,
    /// A
    Ace = 12,
}

impl Rank {
    /// Parse from a single character: '2'..'9', 'T', 'J', 'Q', 'K', 'A'
    /// (case-insensitive).
    pub fn parse(c: char) -> Option<Self> {
        let r = match c {
            '2' => Rank::Two,
            '3' => Rank::Three,
            '4' => Rank::Four,
            '5' => Rank::Five,
            '6' => Rank::Six,
            '7' => Rank::Seven,
            '8' => Rank::Eight,
            '9' => Rank::Nine,
            'T' | 't' => Rank::Ten,
            'J' | 'j' => Rank::Jack,
            'Q' | 'q' => Rank::Queen,
            'K' | 'k' => Rank::King,
            'A' | 'a' => Rank::Ace,
            _ => return None,
        };
        Some(r)
    }

    /// Convert to its canonical uppercase character.
    pub const fn to_char(self) -> char {
        match self {
            Rank::Two => '2',
            Rank::Three => '3',
            Rank::Four => '4',
            Rank::Five => '5',
            Rank::Six => '6',
            Rank::Seven => '7',
            Rank::Eight => '8',
            Rank::Nine => '9',
            Rank::Ten => 'T',
            Rank::Jack => 'J',
            Rank::Queen => 'Q',
            Rank::King => 'K',
            Rank::Ace => 'A',
        }
    }
}

/// A playing-card suit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Suit {
    /// ♣
    Clubs = 0,
    /// ♦
    Diamonds = 1,
    /// ♥
    Hearts = 2,
    /// ♠
    Spades = 3,
}

impl Suit {
    /// Parse from a single character: 'c', 'd', 'h', 's' (case-insensitive).
    pub fn parse(c: char) -> Option<Self> {
        let s = match c {
            'c' | 'C' => Suit::Clubs,
            'd' | 'D' => Suit::Diamonds,
            'h' | 'H' => Suit::Hearts,
            's' | 'S' => Suit::Spades,
            _ => return None,
        };
        Some(s)
    }

    /// Convert to canonical lowercase character.
    pub const fn to_char(self) -> char {
        match self {
            Suit::Clubs => 'c',
            Suit::Diamonds => 'd',
            Suit::Hearts => 'h',
            Suit::Spades => 's',
        }
    }
}

/// A single playing card, encoded as a u8 in 0..52.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Card(pub u8);

impl Card {
    /// Construct from rank + suit.
    pub const fn new(rank: Rank, suit: Suit) -> Self {
        Self(((rank as u8) << 2) | (suit as u8))
    }

    /// Extract the rank.
    pub const fn rank(self) -> Rank {
        // Safety: we only allow construction via `new` or `parse`, which
        // guarantee the rank nibble is in 0..13.
        unsafe { std::mem::transmute(self.0 >> 2) }
    }

    /// Extract the suit.
    pub const fn suit(self) -> Suit {
        // Safety: the low 2 bits of any u8 are always in 0..4.
        unsafe { std::mem::transmute(self.0 & 0b11) }
    }

    /// Parse from two-character notation (e.g., "Ah", "Ts", "2c").
    ///
    /// Both cases accepted. Returns `None` if the string is not exactly
    /// two characters of the expected form.
    pub fn parse(s: &str) -> Option<Self> {
        let mut chars = s.chars();
        let rank_c = chars.next()?;
        let suit_c = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        let rank = Rank::parse(rank_c)?;
        let suit = Suit::parse(suit_c)?;
        Some(Card::new(rank, suit))
    }
}

impl std::fmt::Display for Card {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.rank().to_char(), self.suit().to_char())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All 52 canonical card strings, in (rank, suit) order matching the
    /// u8 encoding. Used by several roundtrip tests.
    fn all_card_strings() -> Vec<String> {
        let mut out = Vec::with_capacity(52);
        for r in 0..13u8 {
            for s in 0..4u8 {
                let rank: Rank = unsafe { std::mem::transmute(r) };
                let suit: Suit = unsafe { std::mem::transmute(s) };
                out.push(format!("{}{}", rank.to_char(), suit.to_char()));
            }
        }
        out
    }

    #[test]
    fn encoding_matches_spec() {
        // Two of Clubs = 0
        assert_eq!(Card::new(Rank::Two, Suit::Clubs).0, 0);
        // Ace of Spades = 51 (12 << 2 | 3)
        assert_eq!(Card::new(Rank::Ace, Suit::Spades).0, 51);
    }

    #[test]
    fn rank_suit_extraction_roundtrips() {
        for r in 0..13u8 {
            for s in 0..4u8 {
                let rank: Rank = unsafe { std::mem::transmute(r) };
                let suit: Suit = unsafe { std::mem::transmute(s) };
                let c = Card::new(rank, suit);
                assert_eq!(c.rank(), rank);
                assert_eq!(c.suit(), suit);
                assert_eq!(c.0, (r << 2) | s);
            }
        }
    }

    #[test]
    fn parse_display_roundtrip_all_52() {
        for s in all_card_strings() {
            let c = Card::parse(&s).unwrap_or_else(|| panic!("failed to parse {s}"));
            assert_eq!(format!("{c}"), s);
        }
    }

    #[test]
    fn parse_accepts_both_cases() {
        assert_eq!(Card::parse("Ah"), Card::parse("ah"));
        assert_eq!(Card::parse("Ts"), Card::parse("tS"));
        assert_eq!(Card::parse("2c"), Card::parse("2C"));
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(Card::parse("").is_none());
        assert!(Card::parse("A").is_none());
        assert!(Card::parse("Ahx").is_none());
        assert!(Card::parse("1h").is_none()); // rank 1 not valid
        assert!(Card::parse("Ax").is_none()); // suit x not valid
        assert!(Card::parse("Xh").is_none());
    }

    #[test]
    fn all_52_encodings_are_distinct() {
        let mut seen = std::collections::HashSet::new();
        for r in 0..13u8 {
            for s in 0..4u8 {
                let rank: Rank = unsafe { std::mem::transmute(r) };
                let suit: Suit = unsafe { std::mem::transmute(s) };
                assert!(seen.insert(Card::new(rank, suit).0));
            }
        }
        assert_eq!(seen.len(), 52);
    }
}
