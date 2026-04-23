//! Cache-friendly packed storage for per-info-set CFR+ bookkeeping.
//!
//! The naive layout `HashMap<InfoSetId, {regret_sum: Vec<f32>, ...}>` chases
//! heap pointers on every regret update. At the river inner loop that's
//! pointer chase → random-access `HashMap` lookup → another indirection
//! through each `Vec`'s heap pointer per info set. Per
//! `docs/LIMITING_FACTOR.md`, this is step #1 on the optimization ladder:
//! replace with flat, contiguous storage so the inner loop streams through
//! memory sequentially.
//!
//! # Layout
//!
//! Three parallel `Box<[f32]>` buffers, one each for `regret_sum`,
//! `strategy_sum`, and `current_strategy` (the scratch buffer). Each is
//! sized `num_info_sets * stride`, where `stride = max_actions`. Info set
//! `i`'s slot lives at `[i * stride .. i * stride + num_actions_at_i]`.
//! Info sets with fewer than `max_actions` legal actions simply leave the
//! tail slots at zero; callers pass the actual action count when reading /
//! writing so nothing downstream sees the padding.
//!
//! # Why `Box<[f32]>` and not `Vec<f32>`
//!
//! `Vec<f32>` carries a capacity field that `Box<[f32]>` doesn't. The table
//! never grows after construction, so capacity is pure overhead (16 bytes
//! per table × 3 tables, plus a slightly more pessimistic aliasing story
//! for the optimizer). More importantly, `Box<[f32]>` encodes "fixed size"
//! in the type, which matches the invariant.
//!
//! # Cache sizing
//!
//! `stride = max_actions` is the stride-1 dimension. For an NLHE bet tree
//! with 5 actions, a single info set's regret_sum is 20 bytes — five info
//! sets fit in a cache line. The HashMap path, by contrast, pays the cost
//! of a hash + bucket lookup on every access, and each bucket's `Vec`
//! lives wherever the allocator chose, not next to its neighbors.
//!
//! # Indexing conventions
//!
//! - `info_set_idx` is a **dense index** in `[0, num_info_sets)`, not the
//!   opaque `InfoSetId` from `game.rs`. Callers maintain their own
//!   `InfoSetId → usize` map if they need one. For `CfrPlusFlat`, that
//!   map lives inside the solver.
//! - `regrets(i)` returns exactly `num_actions_at(i)` entries. The
//!   `num_actions` slice length is stored alongside in `CfrPlusFlat`, not
//!   here — `RegretTables` is intentionally dumb storage.

/// Cache-friendly packed storage for per-info-set CFR+ bookkeeping.
///
/// Layout: one `Box<[f32]>` for `regret_sum`, one for `strategy_sum`, one
/// for `current_strategy` (scratch). Sized at construction time based on
/// the info-set count and max-actions. Indexing is
/// `[info_set_idx][action_idx]` with a fixed stride.
///
/// See the module-level docs for the "why".
#[derive(Debug, Clone)]
pub struct RegretTables {
    regret_sum: Box<[f32]>,
    strategy_sum: Box<[f32]>,
    current_strategy: Box<[f32]>,
    stride: usize,
    num_info_sets: usize,
}

impl RegretTables {
    /// Construct a fresh zeroed set of tables sized for `num_info_sets`
    /// info sets, each allocated `max_actions` slots (the stride).
    ///
    /// # Panics
    ///
    /// Panics if `max_actions == 0` — an info set with no legal actions
    /// is a malformed game, not a storage concern; we want the panic
    /// early rather than a silent divide-by-zero downstream.
    pub fn new(num_info_sets: usize, max_actions: usize) -> Self {
        assert!(
            max_actions > 0,
            "RegretTables::new: max_actions must be > 0"
        );
        let total = num_info_sets
            .checked_mul(max_actions)
            .expect("RegretTables::new: num_info_sets * max_actions overflow");
        // `vec![0.0; total].into_boxed_slice()` zero-inits in one
        // allocation. Not using uninit + later fill because (a) zero is
        // the correct CFR+ initial state and (b) uninit-then-write is a
        // footgun for f32 (NaN leak risk) that buys us nothing here.
        Self {
            regret_sum: vec![0.0f32; total].into_boxed_slice(),
            strategy_sum: vec![0.0f32; total].into_boxed_slice(),
            current_strategy: vec![0.0f32; total].into_boxed_slice(),
            stride: max_actions,
            num_info_sets,
        }
    }

    /// Number of info sets this table was sized for.
    pub fn len(&self) -> usize {
        self.num_info_sets
    }

    /// True if the table has zero info sets (an empty subgame).
    pub fn is_empty(&self) -> bool {
        self.num_info_sets == 0
    }

    /// Stride in `f32`s between consecutive info sets' slots. Equals the
    /// `max_actions` passed to `new`.
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// The full `stride`-wide slice of regret sums for `info_set_idx`.
    ///
    /// Callers typically only want the first `num_actions_at(i)` entries;
    /// see `regrets_with_len`.
    pub fn regrets(&self, info_set_idx: usize) -> &[f32] {
        let lo = info_set_idx * self.stride;
        &self.regret_sum[lo..lo + self.stride]
    }

    /// Mutable form of [`RegretTables::regrets`].
    pub fn regrets_mut(&mut self, info_set_idx: usize) -> &mut [f32] {
        let lo = info_set_idx * self.stride;
        &mut self.regret_sum[lo..lo + self.stride]
    }

    /// The full `stride`-wide slice of cumulative strategy sums for
    /// `info_set_idx`.
    pub fn strategy_sum(&self, info_set_idx: usize) -> &[f32] {
        let lo = info_set_idx * self.stride;
        &self.strategy_sum[lo..lo + self.stride]
    }

    /// Mutable form of [`RegretTables::strategy_sum`].
    pub fn strategy_sum_mut(&mut self, info_set_idx: usize) -> &mut [f32] {
        let lo = info_set_idx * self.stride;
        &mut self.strategy_sum[lo..lo + self.stride]
    }

    /// Current-iteration scratch slice for `info_set_idx`.
    pub fn current_strategy(&self, info_set_idx: usize) -> &[f32] {
        let lo = info_set_idx * self.stride;
        &self.current_strategy[lo..lo + self.stride]
    }

    /// Mutable form of [`RegretTables::current_strategy`].
    pub fn current_strategy_mut(&mut self, info_set_idx: usize) -> &mut [f32] {
        let lo = info_set_idx * self.stride;
        &mut self.current_strategy[lo..lo + self.stride]
    }

    /// Borrow the regret and scratch slices for `info_set_idx` in one
    /// call, so the regret-matching step can read regrets and write the
    /// current strategy without the caller juggling two `&mut self`
    /// borrows.
    pub fn regrets_and_current_mut(&mut self, info_set_idx: usize) -> (&[f32], &mut [f32]) {
        let lo = info_set_idx * self.stride;
        let hi = lo + self.stride;
        (&self.regret_sum[lo..hi], &mut self.current_strategy[lo..hi])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_zero_inits_all_buffers() {
        let t = RegretTables::new(4, 3);
        for i in 0..4 {
            for &r in t.regrets(i) {
                assert_eq!(r, 0.0);
            }
            for &s in t.strategy_sum(i) {
                assert_eq!(s, 0.0);
            }
            for &c in t.current_strategy(i) {
                assert_eq!(c, 0.0);
            }
        }
    }

    #[test]
    fn stride_and_len_reflect_construction() {
        let t = RegretTables::new(7, 5);
        assert_eq!(t.len(), 7);
        assert_eq!(t.stride(), 5);
        assert!(!t.is_empty());
        assert_eq!(t.regrets(0).len(), 5);
        assert_eq!(t.regrets(6).len(), 5);
    }

    #[test]
    fn empty_table_reports_empty() {
        let t = RegretTables::new(0, 4);
        assert_eq!(t.len(), 0);
        assert!(t.is_empty());
    }

    #[test]
    fn writes_land_in_the_correct_slot() {
        // Fill row 2 with a pattern; make sure other rows are untouched.
        let mut t = RegretTables::new(4, 3);
        for (i, v) in t.regrets_mut(2).iter_mut().enumerate() {
            *v = 10.0 + i as f32;
        }
        assert_eq!(t.regrets(2), &[10.0, 11.0, 12.0]);
        for other in [0, 1, 3] {
            assert_eq!(t.regrets(other), &[0.0, 0.0, 0.0]);
        }
    }

    #[test]
    fn writes_to_each_buffer_are_independent() {
        // Writing to regret_sum[1] must not affect strategy_sum[1] or
        // current_strategy[1]. This would fail if we were accidentally
        // aliasing the three buffers.
        let mut t = RegretTables::new(3, 2);
        t.regrets_mut(1)[0] = 1.0;
        t.regrets_mut(1)[1] = 2.0;
        assert_eq!(t.strategy_sum(1), &[0.0, 0.0]);
        assert_eq!(t.current_strategy(1), &[0.0, 0.0]);

        t.strategy_sum_mut(1)[0] = 3.0;
        assert_eq!(t.regrets(1), &[1.0, 2.0]);
        assert_eq!(t.current_strategy(1), &[0.0, 0.0]);

        t.current_strategy_mut(1)[1] = 4.0;
        assert_eq!(t.regrets(1), &[1.0, 2.0]);
        assert_eq!(t.strategy_sum(1), &[3.0, 0.0]);
    }

    #[test]
    fn regrets_and_current_mut_exposes_both_slices() {
        let mut t = RegretTables::new(2, 4);
        for (i, v) in t.regrets_mut(0).iter_mut().enumerate() {
            *v = i as f32;
        }
        let (r, c) = t.regrets_and_current_mut(0);
        assert_eq!(r, &[0.0, 1.0, 2.0, 3.0]);
        assert_eq!(c.len(), 4);
        for (i, slot) in c.iter_mut().enumerate() {
            *slot = r[i] * 2.0;
        }
        assert_eq!(t.current_strategy(0), &[0.0, 2.0, 4.0, 6.0]);
    }

    #[test]
    #[should_panic(expected = "max_actions must be > 0")]
    fn zero_max_actions_panics() {
        let _ = RegretTables::new(5, 0);
    }
}
