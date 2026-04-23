//! Board = 0 to 5 community cards.

use crate::card::Card;

/// Up to 5 community cards, plus a length byte.
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
        Self { cards: [Card(0); 5], len: 0 }
    }

    /// Flop constructor.
    // TODO (Day 1, agent A1): implement.
    pub fn flop(_a: Card, _b: Card, _c: Card) -> Self {
        todo!()
    }

    /// Parse from notation like "AhKh2s" (flop), "AhKh2sQc" (turn),
    /// "AhKh2sQc4d" (river).
    // TODO (Day 1, agent A1): implement.
    pub fn parse(_s: &str) -> Option<Self> {
        todo!()
    }
}
