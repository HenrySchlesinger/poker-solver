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
pub mod cfr_flat;
pub mod convergence;
pub mod game;
pub mod matching;
pub mod matching_simd;
pub mod mccfr;
pub mod tables;

// Opt-in Metal GPU backend for regret matching. Only compiled when the
// `metal` feature is on AND the target OS is macOS. See `src/metal/mod.rs`
// for the rationale, the shader loading strategy, and the `build.rs`
// graceful-fallback contract.
#[cfg(all(feature = "metal", target_os = "macos"))]
pub mod metal;

pub use cfr::{CfrPlus, Strategy};
pub use cfr_flat::{
    enumerate_info_sets, enumerate_info_sets_from_roots, CfrPlusFlat, InfoSetDescriptor,
};
pub use convergence::{best_response_value, exploitability_two_player_zero_sum};
pub use game::{Game, InfoSetId, Player};
pub use matching::{regret_match, regret_match_vec};
pub use matching_simd::regret_match_simd;
pub use tables::RegretTables;

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
