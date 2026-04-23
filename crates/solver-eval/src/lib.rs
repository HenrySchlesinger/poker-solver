//! Pure poker primitives: cards, hands, boards, evaluator, equity,
//! isomorphism. Zero algorithm logic.
//!
//! This is a leaf crate — it depends on nothing else in the workspace.
//! Keep it that way.

#![warn(missing_docs)]

pub mod board;
pub mod card;
pub mod combo;
pub mod equity;
pub mod eval;
pub mod hand;
pub mod iso;
pub mod texture;

pub use board::Board;
pub use card::{Card, Rank, Suit};
pub use hand::Hand;
