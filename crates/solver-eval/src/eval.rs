//! 5-card and 7-card hand evaluator.
//!
//! Do NOT write this from scratch. Hand evaluation is a solved problem
//! with multiple high-quality open-source implementations:
//!
//! - `rs-poker` — pure Rust, MIT, probably our best option
//! - `pokerlookup` — Rust port of 2+2 lookup tables, fast
//! - `poker-eval` (C, Cactus Kev) — classic, 100M hands/s
//!
//! Pick one, wrap it in our types, profile. Only reach for a custom
//! evaluator if profiling shows it's a measurable bottleneck.

use crate::board::Board;
use crate::hand::Hand;

/// Opaque hand-rank value. Higher = stronger hand.
// TODO (Day 1, agent A2): concrete repr once the evaluator is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HandRank(pub u32);

/// Evaluate the best 5-card hand from 7 cards (2 hole + 5 community).
// TODO (Day 1, agent A2): wrap chosen crate.
pub fn eval_7(_hand: &Hand, _board: &Board) -> HandRank {
    todo!()
}
