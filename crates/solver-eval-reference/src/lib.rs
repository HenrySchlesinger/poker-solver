//! Reference poker-math oracle, ported from the Python backend of
//! `~/Desktop/Poker Panel/engine/rules/`. Clarity-first, speed-second.
//! Used exclusively by `#[cfg(test)]` code in `solver-eval` to
//! differentially validate the production implementation.
//!
//! # Why this exists
//!
//! Poker Panel has been running live on streams with real players and
//! real money for months. Its hand-ranking, equity, and showdown code
//! has been stress-tested against the ground truth of actual card
//! detections, side pots, and split pots. That makes it a valuable
//! oracle for the *new* Rust solver's poker math, which is in its first
//! week of existence.
//!
//! Instead of just pasting known PokerStove / Equilab reference numbers
//! and hoping, we re-implement Poker Panel's functions here in Rust —
//! structurally identical to the Python — and use random-input
//! differential testing to catch regressions in either codebase.
//!
//! # Structural mirror
//!
//! The goal is that if you open this file side-by-side with the
//! original Python, the functions should line up one-to-one with the
//! same names and the same control flow. So:
//!
//! | Poker Panel (Python)                   | This crate (Rust)                    |
//! |----------------------------------------|--------------------------------------|
//! | `holdem.evaluate_best_five_of_seven`   | `reference_eval_7`                   |
//! | `holdem.calculate_hand_odds`           | `reference_hand_vs_random_equity`    |
//! | `equity_optimized.exact_river_equity`  | `reference_exact_river_equity`       |
//! | `equity_optimized._run_monte_carlo`    | `reference_equity_monte_carlo`       |
//! | `equity_optimized.fast_enumeration_equity` | `reference_fast_enumeration_equity` |
//! | `equity_optimized.normalize_hand`      | `reference_normalize_hand_169`       |
//! | `showdown` side-pot winner finder      | `reference_showdown_winners`         |
//!
//! # Evaluator
//!
//! Poker Panel uses the `treys` Python library. We use `rs_poker` here
//! for the same reason `solver-eval` does: pure-Rust, no C deps,
//! `Rankable::rank()` / `rank_five()` on card slices. Both produce a
//! monotonic ordering ("lower score = better" for treys; variant-order
//! for `rs_poker::Rank`). We wrap both into `solver_eval::HandRank`,
//! which is what the solver's production path uses, so the oracle and
//! the production code have identical types at the differential-test
//! boundary.

#![warn(missing_docs)]
#![allow(clippy::needless_range_loop)] // "literal port" beats "idiomatic Rust" here

pub mod equity;
pub mod eval;
pub mod preflop;
pub mod showdown;

pub use equity::{
    reference_equity_monte_carlo, reference_exact_river_equity, reference_fast_enumeration_equity,
};
pub use eval::{reference_eval_5, reference_eval_7};
pub use preflop::reference_normalize_hand_169;
pub use showdown::reference_showdown_winners;
