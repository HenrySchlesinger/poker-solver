//! Pure poker primitives: cards, hands, boards, evaluator, equity,
//! isomorphism. Zero algorithm logic.
//!
//! This is a leaf crate — it depends on nothing else in the workspace.
//! Keep it that way.

#![warn(missing_docs)]

pub mod card;
pub mod hand;
pub mod board;
pub mod combo;
pub mod eval;
pub mod equity;
pub mod iso;
pub mod texture;

pub use card::{Card, Rank, Suit};
pub use hand::Hand;
pub use board::Board;
