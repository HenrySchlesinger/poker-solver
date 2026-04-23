//! NLHE actions and action history.
//!
//! See `docs/POKER.md` for domain context.

/// The four streets of a NLHE hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Street {
    /// Before any community cards.
    Preflop = 0,
    /// After the 3-card flop.
    Flop = 1,
    /// After the turn (4th community card).
    Turn = 2,
    /// After the river (5th community card).
    River = 3,
}

/// A player action at a decision node.
///
/// Bet amounts are in chips, not pot fractions — the bet tree translates
/// pot-fractions to absolute chips when the subgame is constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Give up the hand.
    Fold,
    /// Pass (no bet to face).
    Check,
    /// Match the current bet.
    Call,
    /// Bet a specific chip amount (must correspond to a bet-tree bucket).
    Bet(u32),
    /// Raise to a specific chip amount.
    Raise(u32),
    /// All-in (bet effective stack).
    AllIn,
}

/// An ordered sequence of actions taken so far in a hand.
///
/// Used to reconstruct pot/stack state and to uniquely identify decision
/// nodes for info-set lookup.
// TODO (Day 2, agent A2): implement.
pub struct ActionLog {
    // TODO: SmallVec<[(Street, Action); 16]> or similar.
}
