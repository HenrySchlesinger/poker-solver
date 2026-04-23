//! Card representation.
//!
//! Encoding: `u8` where:
//!   - `rank = card >> 2`    (0..13, 2 = deuce, 12 = ace)
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

/// A single playing card, encoded as a u8 in 0..52.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Card(pub u8);

impl Card {
    /// Construct from rank + suit.
    pub const fn new(rank: Rank, suit: Suit) -> Self {
        Self((rank as u8) << 2 | (suit as u8))
    }

    /// Extract the rank.
    // TODO (Day 1, agent A1): implement.
    pub const fn rank(self) -> Rank {
        // Safety: we only allow construction via `new`, so value is valid.
        unsafe { std::mem::transmute(self.0 >> 2) }
    }

    /// Extract the suit.
    // TODO (Day 1, agent A1): implement.
    pub const fn suit(self) -> Suit {
        unsafe { std::mem::transmute(self.0 & 0b11) }
    }

    /// Parse from two-character notation (e.g., "Ah", "Ts", "2c").
    // TODO (Day 1, agent A1): implement.
    pub fn parse(_s: &str) -> Option<Self> {
        todo!()
    }
}

impl std::fmt::Display for Card {
    // TODO (Day 1, agent A1): implement. Output "Ah", "Ts", "2c", etc.
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}
