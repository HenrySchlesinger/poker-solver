//! Hand = 2 hole cards.

use crate::card::Card;

/// Two hole cards. By convention `[0]` is the higher rank (tie-break by
/// suit value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Hand(pub [Card; 2]);

impl Hand {
    /// Construct, canonicalizing card order.
    // TODO (Day 1, agent A1): implement.
    pub fn new(_a: Card, _b: Card) -> Self {
        todo!()
    }

    /// Parse from notation like "AhKd".
    // TODO (Day 1, agent A1): implement.
    pub fn parse(_s: &str) -> Option<Self> {
        todo!()
    }
}
