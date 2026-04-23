//! NLHE-specific primitives: game tree, ranges, bet-tree abstraction.
//!
//! The `NlheSubgame` type implements `solver_core::Game`, connecting the
//! generic CFR machinery to the specific rules of No-Limit Hold'em.

#![warn(missing_docs)]

pub mod action;
pub mod bet_tree;
pub mod cache;
pub mod flop_cache;
pub mod preflop;
pub mod range;
pub mod subgame;
pub mod subgame_vector;

pub use action::{Action, ActionLog, Street};
pub use bet_tree::BetTree;
pub use range::Range;
pub use subgame::NlheSubgame;
pub use subgame_vector::{ActionState, NlheSubgameVector};
