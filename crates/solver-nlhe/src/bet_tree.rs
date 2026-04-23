//! Discretized bet-size abstractions per street.
//!
//! Real NLHE allows continuous bet sizes. We discretize to a small set
//! per street. When villain makes an out-of-tree bet, snap to the nearest
//! bucket by pot-fraction ratio.
//!
//! Default trees for v0.1 (see `docs/ALGORITHMS.md#bet-tree-abstraction`):
//! - Flop:  33%, 66%, 100% pot, all-in
//! - Turn:  50%, 100%, 200% pot, all-in
//! - River: 33%, 66%, 100%, 200% pot, all-in

use crate::action::Street;

/// A bet-tree for one street: a sorted list of pot-fractions in [0, ∞).
// TODO (Day 2, agent A1): flesh out struct + builder.
pub struct BetTree {
    // TODO: per-street Vec<f32> of pot fractions.
}

impl BetTree {
    /// Returns the v0.1 default tree.
    // TODO: implement with the values above.
    pub fn default_v0_1() -> Self {
        Self {}
    }

    /// Returns the discretized pot fractions legal at this street.
    // TODO
    pub fn sizings_for(&self, _street: Street) -> &[f32] {
        &[]
    }

    /// Snap a villain-bet fraction to the nearest in-tree bucket.
    // TODO
    pub fn snap(&self, _street: Street, _observed_fraction: f32) -> f32 {
        0.0
    }
}
