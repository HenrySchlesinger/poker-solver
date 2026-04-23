//! Game-agnostic CFR+/MCCFR/Vector CFR solver core.
//!
//! This crate knows nothing about poker. It operates on any extensive-form
//! game with imperfect information that implements [`Game`].
//!
//! Kuhn Poker is the reference test fixture — if Kuhn converges, the
//! algorithm implementation is correct. NLHE is implemented in
//! `solver-nlhe` on top of these primitives.
//!
//! See `docs/ALGORITHMS.md` for the algorithmic background.

#![warn(missing_docs)]

pub mod cfr;
pub mod mccfr;
pub mod matching;
pub mod convergence;
pub mod game;

pub use game::{Game, InfoSetId, Player};

/// Error type surfaced by the solver core.
#[derive(Debug, thiserror::Error)]
pub enum SolverError {
    /// The game was malformed (e.g., a decision node with zero actions).
    #[error("invalid game structure: {0}")]
    InvalidGame(String),

    /// The solver ran out of its iteration budget without converging.
    #[error("failed to converge within {iterations} iterations (final exploitability: {exploitability})")]
    DidNotConverge {
        /// How many iterations we ran.
        iterations: u32,
        /// The exploitability at termination.
        exploitability: f32,
    },
}

/// Convenience result alias.
pub type SolverResult<T> = Result<T, SolverError>;
