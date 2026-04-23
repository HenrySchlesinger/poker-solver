//! Discretized bet-size abstractions per street.
//!
//! Real NLHE allows continuous bet sizes. We discretize to a small set of
//! pot-fractions per street. When villain makes an out-of-tree bet, snap
//! to the nearest in-tree bucket by log-ratio distance.
//!
//! Default trees for v0.1 (see `docs/ALGORITHMS.md#bet-tree-abstraction`):
//! - Flop:  33%, 66%, 100% pot, all-in
//! - Turn:  50%, 100%, 200% pot, all-in
//! - River: 33%, 66%, 100%, 200% pot, all-in
//!
//! # All-in representation
//!
//! The "all-in" bucket is encoded as [`f32::INFINITY`]. Sorted ascending
//! it lives at the end of each street's sizing list. Rationale:
//!
//! - It's a valid `f32` and orders correctly with finite fractions.
//! - Log-ratio distance puts it "infinitely far" from any finite bet, so
//!   a finite observed fraction never snaps to all-in unless all-in is the
//!   only bucket. An `f32::INFINITY` observed fraction snaps to all-in
//!   exactly (distance zero).
//! - The caller is responsible for mapping the chip-level observation
//!   (bet chips, effective stack, pot) to the right fraction: compute
//!   `bet / pot` for normal bets, and pass [`f32::INFINITY`] when the
//!   bet is for the full remaining stack.
//!
//! # Snap semantics (edge cases)
//!
//! - `observed_fraction` *above* the largest finite bucket snaps to that
//!   largest finite bucket — not to all-in. Example: a 3× pot bet on a
//!   river with `{0.33, 0.66, 1.0, 2.0, INF}` snaps to 2.0.
//! - `observed_fraction` *below* the smallest bucket snaps to the smallest.
//! - `observed_fraction == f32::INFINITY` snaps to the all-in bucket iff
//!   one exists on that street.
//! - `observed_fraction <= 0` (or NaN) is a programmer error and panics.
//!   Zero is a check, not a bet — the caller must have classified that
//!   before calling `snap`.

use crate::action::Street;

/// A pot-fraction for a bet. `f32::INFINITY` represents all-in.
type Fraction = f32;

/// A discretized bet-size tree covering all four streets.
///
/// Each street holds a sorted-ascending list of pot-fractions in `(0, ∞]`.
/// `f32::INFINITY` is the canonical all-in marker. Preflop currently holds
/// the same sizings as the flop by default — v0.1 preflop is a static
/// lookup and does not really need its own tree, but having one slot per
/// [`Street`] keeps indexing simple.
#[derive(Debug, Clone, PartialEq)]
pub struct BetTree {
    /// Sizings per street, indexed by `Street as usize`.
    per_street: [Vec<Fraction>; 4],
}

impl BetTree {
    /// The default v0.1 tree.
    ///
    /// Flop: `33%, 66%, 100%` pot, all-in.
    /// Turn: `50%, 100%, 200%` pot, all-in.
    /// River: `33%, 66%, 100%, 200%` pot, all-in.
    /// Preflop: same as flop (placeholder — preflop uses a precomputed
    /// static range in v0.1 and does not currently consult this tree).
    pub fn default_v0_1() -> Self {
        let flop = vec![0.33, 0.66, 1.0, f32::INFINITY];
        let turn = vec![0.5, 1.0, 2.0, f32::INFINITY];
        let river = vec![0.33, 0.66, 1.0, 2.0, f32::INFINITY];
        let preflop = flop.clone();
        Self {
            per_street: [preflop, flop, turn, river],
        }
    }

    /// Sorted list of pot-fractions legal at this street.
    ///
    /// The returned slice is ascending and does **not** include zero
    /// (check/no-bet is not a sizing). `f32::INFINITY` at the end indicates
    /// an all-in bucket.
    pub fn sizings_for(&self, street: Street) -> &[f32] {
        &self.per_street[street as usize]
    }

    /// Snap an observed bet fraction to the nearest in-tree bucket.
    ///
    /// Distance metric is **log-ratio**: we minimize
    /// `|log2(observed) - log2(bucket)|`. That's what "nearest by ratio"
    /// means in the standard solver literature and in
    /// `docs/ALGORITHMS.md#bet-tree-abstraction`: the gap between 33% and
    /// 66% pot is the same as between 66% and 133%, rather than being
    /// "closer" just because it's smaller in absolute chips.
    ///
    /// Special cases:
    /// - `observed_fraction == f32::INFINITY` returns the all-in bucket
    ///   iff the street has one. If the street's only bucket is all-in,
    ///   any positive observation returns all-in by default.
    /// - A `f32::INFINITY` bucket is never selected for a finite
    ///   observation (log-ratio distance to it is infinite) unless it is
    ///   the only bucket on that street.
    ///
    /// # Panics
    ///
    /// - If `observed_fraction <= 0` or is NaN. A zero observation is a
    ///   check, not a bet; the caller must dispatch that elsewhere.
    /// - If the street has no configured sizings. (Not reachable via
    ///   [`Self::default_v0_1`] or [`Self::custom`], both of which
    ///   validate non-empty.)
    pub fn snap(&self, street: Street, observed_fraction: f32) -> f32 {
        assert!(
            observed_fraction.is_finite() || observed_fraction == f32::INFINITY,
            "snap: observed_fraction must not be NaN, got {observed_fraction}"
        );
        assert!(
            observed_fraction > 0.0,
            "snap: observed_fraction must be strictly positive (0.0 is a check, \
             not a bet — caller must classify); got {observed_fraction}"
        );

        let sizings = self.sizings_for(street);
        assert!(
            !sizings.is_empty(),
            "snap: street {street:?} has no configured sizings"
        );

        // Exact infinity match: only all-in snaps to all-in.
        if observed_fraction == f32::INFINITY {
            // Prefer the INF bucket if it exists; else return the largest
            // finite bucket (this can't happen under default/custom because
            // validation would reject an empty list, but it's harmless).
            if let Some(&last) = sizings.last() {
                return last;
            }
            unreachable!("sizings non-empty checked above");
        }

        // Finite observation: log-ratio distance. log2(x) for x > 0 is
        // finite, log2(inf) = inf, so an INF bucket automatically loses to
        // any finite bucket whenever a finite bucket is present.
        let obs_log = observed_fraction.log2();
        let mut best = sizings[0];
        let mut best_dist = (obs_log - best.log2()).abs();
        for &candidate in &sizings[1..] {
            let d = (obs_log - candidate.log2()).abs();
            // Strict `<` so that ties go to the smaller bucket (first hit).
            // This is an arbitrary but deterministic choice; log-ratio
            // ties only happen on synthetic inputs.
            if d < best_dist {
                best = candidate;
                best_dist = d;
            }
        }
        best
    }

    /// Build a custom tree. Validates that each street's sizing list is
    /// non-empty, strictly positive, and sorted strictly ascending.
    ///
    /// [`f32::INFINITY`] is permitted as the final entry to mark the
    /// all-in bucket.
    ///
    /// Preflop defaults to a copy of the flop list — preflop in v0.1 is a
    /// static lookup that does not currently consult the tree, but every
    /// [`Street`] slot must be populated.
    pub fn custom(flop: Vec<f32>, turn: Vec<f32>, river: Vec<f32>) -> anyhow::Result<Self> {
        validate_sizings("flop", &flop)?;
        validate_sizings("turn", &turn)?;
        validate_sizings("river", &river)?;
        let preflop = flop.clone();
        Ok(Self {
            per_street: [preflop, flop, turn, river],
        })
    }
}

impl Default for BetTree {
    fn default() -> Self {
        Self::default_v0_1()
    }
}

fn validate_sizings(name: &str, sizings: &[f32]) -> anyhow::Result<()> {
    if sizings.is_empty() {
        anyhow::bail!("{name}: sizings list is empty (need at least one bucket)");
    }
    let mut prev: Option<f32> = None;
    for (i, &f) in sizings.iter().enumerate() {
        if f.is_nan() {
            anyhow::bail!("{name}[{i}]: NaN is not a valid pot-fraction");
        }
        if f <= 0.0 {
            anyhow::bail!("{name}[{i}]: pot-fraction must be strictly positive, got {f}");
        }
        if let Some(p) = prev {
            if f <= p {
                anyhow::bail!("{name}[{i}]: sizings must be strictly ascending, got {f} after {p}");
            }
        }
        prev = Some(f);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Unit tests on the default tree -----------------------------------

    #[test]
    fn default_flop_has_exact_fractions() {
        let tree = BetTree::default_v0_1();
        let flop = tree.sizings_for(Street::Flop);
        assert_eq!(flop.len(), 4);
        assert_eq!(flop[0], 0.33);
        assert_eq!(flop[1], 0.66);
        assert_eq!(flop[2], 1.0);
        assert_eq!(flop[3], f32::INFINITY);
    }

    #[test]
    fn default_turn_has_exact_fractions() {
        let tree = BetTree::default_v0_1();
        let turn = tree.sizings_for(Street::Turn);
        assert_eq!(turn.len(), 4);
        assert_eq!(turn[0], 0.5);
        assert_eq!(turn[1], 1.0);
        assert_eq!(turn[2], 2.0);
        assert_eq!(turn[3], f32::INFINITY);
    }

    #[test]
    fn default_river_has_exact_fractions() {
        let tree = BetTree::default_v0_1();
        let river = tree.sizings_for(Street::River);
        assert_eq!(river.len(), 5);
        assert_eq!(river[0], 0.33);
        assert_eq!(river[1], 0.66);
        assert_eq!(river[2], 1.0);
        assert_eq!(river[3], 2.0);
        assert_eq!(river[4], f32::INFINITY);
    }

    #[test]
    fn default_preflop_mirrors_flop() {
        // Preflop is a placeholder in v0.1; indexing must still return a
        // non-empty slice so callers don't have to special-case the street.
        let tree = BetTree::default_v0_1();
        assert_eq!(
            tree.sizings_for(Street::Preflop),
            tree.sizings_for(Street::Flop)
        );
    }

    #[test]
    fn default_sizings_are_sorted_ascending() {
        let tree = BetTree::default_v0_1();
        for street in [Street::Preflop, Street::Flop, Street::Turn, Street::River] {
            let s = tree.sizings_for(street);
            assert!(!s.is_empty(), "street {street:?} has empty sizings");
            for w in s.windows(2) {
                assert!(
                    w[0] < w[1],
                    "street {street:?} sizings not ascending: {s:?}"
                );
            }
        }
    }

    // --- Snap semantics ---------------------------------------------------

    #[test]
    fn snap_exact_bucket_returns_self() {
        let tree = BetTree::default_v0_1();
        for street in [Street::Flop, Street::Turn, Street::River] {
            for &b in tree.sizings_for(street) {
                assert_eq!(tree.snap(street, b), b, "bucket {b} should snap to self");
            }
        }
    }

    #[test]
    fn snap_midpoint_flop() {
        // Flop: {0.33, 0.66, 1.0, INF}. A 0.5-pot bet: log-ratio
        // distances: |log2(0.5/0.33)| ≈ 0.60; |log2(0.5/0.66)| ≈ 0.40;
        // |log2(0.5/1.0)| = 1.0. Closest: 0.66.
        let tree = BetTree::default_v0_1();
        assert_eq!(tree.snap(Street::Flop, 0.5), 0.66);
    }

    #[test]
    fn snap_out_of_tree_flop_example() {
        // 47% pot observed; in {0.33, 0.66, 1.0, INF} it's closer to 0.66
        // in log-ratio: |log2(0.47/0.33)| ≈ 0.51, |log2(0.47/0.66)| ≈ 0.49.
        let tree = BetTree::default_v0_1();
        assert_eq!(tree.snap(Street::Flop, 0.47), 0.66);
    }

    #[test]
    fn snap_above_largest_finite_bucket_goes_to_largest_finite() {
        // 3× pot on the river ({0.33, 0.66, 1.0, 2.0, INF}) → 2.0, NOT INF.
        // Rationale: log-ratio to 2.0 = |log2(1.5)| ≈ 0.58; to INF = ∞.
        let tree = BetTree::default_v0_1();
        assert_eq!(tree.snap(Street::River, 3.0), 2.0);
        // Even at huge finite values like 100× pot, we snap to the largest
        // finite bucket rather than all-in (finite observations never
        // snap to all-in when a finite bucket is available).
        assert_eq!(tree.snap(Street::River, 100.0), 2.0);
    }

    #[test]
    fn snap_below_smallest_goes_to_smallest() {
        // 5% pot on the flop ({0.33, 0.66, 1.0, INF}) → 0.33.
        let tree = BetTree::default_v0_1();
        assert_eq!(tree.snap(Street::Flop, 0.05), 0.33);
        assert_eq!(tree.snap(Street::Flop, 0.001), 0.33);
    }

    #[test]
    fn snap_infinity_picks_allin_bucket() {
        let tree = BetTree::default_v0_1();
        assert_eq!(tree.snap(Street::Flop, f32::INFINITY), f32::INFINITY);
        assert_eq!(tree.snap(Street::Turn, f32::INFINITY), f32::INFINITY);
        assert_eq!(tree.snap(Street::River, f32::INFINITY), f32::INFINITY);
    }

    #[test]
    fn snap_finite_never_returns_infinity_when_finite_bucket_exists() {
        let tree = BetTree::default_v0_1();
        for street in [Street::Flop, Street::Turn, Street::River] {
            for f_scaled in 1..1000 {
                let f = f_scaled as f32 * 0.01; // 0.01 .. 9.99
                let snapped = tree.snap(street, f);
                assert!(
                    snapped.is_finite(),
                    "finite observation {f} on {street:?} snapped to INF"
                );
            }
        }
    }

    #[test]
    #[should_panic(expected = "strictly positive")]
    fn snap_zero_panics() {
        let tree = BetTree::default_v0_1();
        let _ = tree.snap(Street::Flop, 0.0);
    }

    #[test]
    #[should_panic(expected = "strictly positive")]
    fn snap_negative_panics() {
        let tree = BetTree::default_v0_1();
        let _ = tree.snap(Street::Flop, -0.5);
    }

    #[test]
    #[should_panic(expected = "must not be NaN")]
    fn snap_nan_panics() {
        let tree = BetTree::default_v0_1();
        let _ = tree.snap(Street::Flop, f32::NAN);
    }

    // --- Property-style tests (deterministic grids) -----------------------

    /// Generate a grid of positive f32 fractions in (0, 5] plus the edges
    /// we care about. 1001 finite values + INF = deterministic sweep.
    fn sweep_fractions() -> Vec<f32> {
        let mut v: Vec<f32> = (1..=1000).map(|i| i as f32 * 0.005).collect(); // 0.005..=5.0
                                                                              // Include a handful of decade boundaries and the all-in marker so
                                                                              // both extreme-low and all-in paths get exercised.
        v.extend_from_slice(&[1e-6, 1e-3, 0.01, 10.0, 100.0, f32::INFINITY]);
        v
    }

    #[test]
    fn property_snap_is_idempotent() {
        let tree = BetTree::default_v0_1();
        let fractions = sweep_fractions();
        for street in [Street::Flop, Street::Turn, Street::River] {
            for &f in &fractions {
                let once = tree.snap(street, f);
                let twice = tree.snap(street, once);
                assert_eq!(
                    once, twice,
                    "snap not idempotent on {street:?} for f={f}: snap(f)={once}, snap(snap(f))={twice}"
                );
            }
        }
    }

    #[test]
    fn property_snap_returns_value_in_sizings() {
        let tree = BetTree::default_v0_1();
        let fractions = sweep_fractions();
        for street in [Street::Preflop, Street::Flop, Street::Turn, Street::River] {
            let sizings = tree.sizings_for(street);
            for &f in &fractions {
                let snapped = tree.snap(street, f);
                assert!(
                    sizings
                        .iter()
                        .any(|&s| s == snapped || (s.is_infinite() && snapped.is_infinite())),
                    "snapped value {snapped} not in sizings {sizings:?} for {street:?}, f={f}"
                );
            }
        }
    }

    // --- Builder / validation ---------------------------------------------

    #[test]
    fn custom_builds_valid_tree() {
        let tree = BetTree::custom(
            vec![0.25, 0.75, 1.5, f32::INFINITY],
            vec![0.5, 1.0, f32::INFINITY],
            vec![0.33, 1.0, 3.0, f32::INFINITY],
        )
        .expect("valid tree should build");
        assert_eq!(tree.sizings_for(Street::Flop)[0], 0.25);
        assert_eq!(tree.sizings_for(Street::Turn).len(), 3);
        assert_eq!(tree.sizings_for(Street::River).len(), 4);
    }

    #[test]
    fn custom_rejects_empty_list() {
        let err = BetTree::custom(vec![], vec![1.0], vec![1.0])
            .expect_err("empty flop list should be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("flop") && msg.contains("empty"),
            "error message should mention empty flop: got {msg}"
        );

        assert!(BetTree::custom(vec![1.0], vec![], vec![1.0]).is_err());
        assert!(BetTree::custom(vec![1.0], vec![1.0], vec![]).is_err());
    }

    #[test]
    fn custom_rejects_non_monotonic() {
        // Descending.
        let err = BetTree::custom(vec![1.0, 0.5], vec![1.0], vec![1.0])
            .expect_err("descending list should be rejected");
        assert!(err.to_string().contains("ascending"));

        // Duplicates (not strictly ascending).
        assert!(BetTree::custom(vec![0.5, 0.5], vec![1.0], vec![1.0]).is_err());

        // Mostly-sorted with one dip.
        assert!(BetTree::custom(vec![0.33, 0.66, 0.5, 1.0], vec![1.0], vec![1.0]).is_err());
    }

    #[test]
    fn custom_rejects_non_positive() {
        assert!(BetTree::custom(vec![0.0, 0.5], vec![1.0], vec![1.0]).is_err());
        assert!(BetTree::custom(vec![-0.1, 0.5], vec![1.0], vec![1.0]).is_err());
        // NaN should also be rejected (it isn't a valid pot-fraction).
        assert!(BetTree::custom(vec![f32::NAN], vec![1.0], vec![1.0]).is_err());
    }

    #[test]
    fn custom_accepts_single_bucket() {
        // Minimum: one bucket per street.
        let tree = BetTree::custom(vec![1.0], vec![1.0], vec![1.0])
            .expect("single-bucket tree should build");
        assert_eq!(tree.snap(Street::Flop, 0.1), 1.0);
        assert_eq!(tree.snap(Street::Flop, 10.0), 1.0);
    }

    #[test]
    fn custom_accepts_allin_only_tree() {
        // All-in-only street: every bet snaps to INF. Degenerate but legal.
        let tree = BetTree::custom(
            vec![f32::INFINITY],
            vec![f32::INFINITY],
            vec![f32::INFINITY],
        )
        .expect("all-in-only tree should build");
        assert_eq!(tree.snap(Street::Flop, 0.5), f32::INFINITY);
        assert_eq!(tree.snap(Street::Flop, f32::INFINITY), f32::INFINITY);
    }

    #[test]
    fn default_equals_default_v0_1() {
        assert_eq!(BetTree::default(), BetTree::default_v0_1());
    }
}
