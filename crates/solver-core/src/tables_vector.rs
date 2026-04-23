//! Combo-lane-major tables for the Vector CFR solver.
//!
//! Counterpart to [`crate::tables::RegretTables`] but with an extra
//! "combo" axis. For each info set:
//!
//! - `regret_sum[action][combo]` is a flat `max_actions * combo_width`
//!   run of f32s.
//! - `strategy_sum[action][combo]` has the same shape.
//!
//! We deliberately drop the `current_strategy` scratch table from
//! [`crate::tables::RegretTables`] — Vector CFR's walker computes the
//! current strategy into a single global scratch buffer per decision
//! node (one 1326-wide slice per action), so we don't need to
//! pre-allocate it per info set.
//!
//! # Layout
//!
//! Flat contiguous `Box<[f32]>`, one per bookkeeping table. For info
//! set `i`, action `a`, combo `c`:
//!
//! ```text
//! offset(i, a, c) = i * (max_actions * combo_width) + a * combo_width + c
//! ```
//!
//! Padding actions (if a specific info set has fewer than
//! `max_actions` legal actions) leave the tail rows at zero. Callers
//! must only read/write `[0..num_actions_at_i]` — the same contract
//! `RegretTables` uses.
//!
//! `combo_width` is typically `NUM_COMBOS = 1326` for NLHE, but
//! parameterized so Kuhn's 3-card lane width (or a test's small
//! values) also works.
//!
//! # Why a separate type and not extend `RegretTables`
//!
//! `RegretTables` is consumed by `CfrPlusFlat`, which stores one
//! regret scalar per (info_set, action). Adding a combo dimension
//! there would double the storage cost for callers that don't need
//! it (the classic `CfrPlus` flow, the Kuhn test fixture's full
//! three-way equivalence harness) and force an API break on
//! `RegretTables`' clean scalar shape. A sibling type composes
//! instead.

use std::collections::HashMap;

use crate::game::InfoSetId;

/// One info-set descriptor as enumerated up-front, analogous to
/// [`crate::cfr_flat::InfoSetDescriptor`]. Duplicated here (rather
/// than reused) so `tables_vector.rs` stays decoupled from the
/// scalar-flat solver's API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorInfoSetDescriptor {
    /// Opaque game-level identifier.
    pub info_set_id: InfoSetId,
    /// Number of legal actions at this info set.
    pub num_actions: usize,
}

/// Combo-lane-major CFR+ tables.
///
/// See the module docs for layout.
#[derive(Debug, Clone)]
pub struct VectorCfrTables {
    /// Cumulative regret, shape `(num_info_sets, max_actions, combo_width)`.
    regret_sum: Box<[f32]>,
    /// Linearly-weighted cumulative strategy, same shape.
    strategy_sum: Box<[f32]>,
    /// Action count per info set (indexed by dense idx). Used to slice
    /// off padding actions when the reader only wants the real ones.
    num_actions_per_info_set: Box<[u8]>,
    /// `InfoSetId → dense index` map.
    id_to_idx: HashMap<InfoSetId, usize>,
    /// Stride of a single info set's block (= `max_actions * combo_width`).
    stride_info_set: usize,
    /// Stride of a single action's combo row (= `combo_width`).
    stride_action: usize,
    /// Combo-axis width (e.g. 1326 for NLHE, 3 for Kuhn).
    combo_width: usize,
    /// Max action count — the action-axis stride multiplier.
    max_actions: usize,
    /// Number of info sets.
    num_info_sets: usize,
}

impl VectorCfrTables {
    /// Allocate zeroed tables for the given descriptor set.
    ///
    /// Every info set gets the global `max_actions` worth of storage
    /// even if its own `num_actions` is smaller. The Vector CFR walk
    /// only touches `[0..num_actions]` in each info set.
    ///
    /// # Panics
    ///
    /// Panics on an empty descriptor list, on `combo_width == 0`, on
    /// duplicate `InfoSetId`s, or on storage-size overflow.
    pub fn new(descriptors: &[VectorInfoSetDescriptor], combo_width: usize) -> Self {
        assert!(
            !descriptors.is_empty(),
            "VectorCfrTables::new: descriptors must be non-empty"
        );
        assert!(
            combo_width > 0,
            "VectorCfrTables::new: combo_width must be > 0"
        );

        let max_actions = descriptors
            .iter()
            .map(|d| d.num_actions)
            .max()
            .expect("non-empty checked above");
        assert!(
            max_actions > 0,
            "VectorCfrTables::new: every info set must have > 0 actions"
        );
        assert!(
            max_actions <= u8::MAX as usize,
            "VectorCfrTables::new: max_actions {max_actions} > 255"
        );

        let stride_action = combo_width;
        let stride_info_set = max_actions
            .checked_mul(stride_action)
            .expect("stride overflow");
        let total = descriptors
            .len()
            .checked_mul(stride_info_set)
            .expect("total overflow");

        let mut id_to_idx: HashMap<InfoSetId, usize> = HashMap::with_capacity(descriptors.len());
        let mut num_actions_per_info_set: Vec<u8> = Vec::with_capacity(descriptors.len());
        for (i, d) in descriptors.iter().enumerate() {
            let prev = id_to_idx.insert(d.info_set_id, i);
            assert!(
                prev.is_none(),
                "VectorCfrTables::new: duplicate InfoSetId {:?} at dense index {}",
                d.info_set_id,
                i
            );
            num_actions_per_info_set.push(d.num_actions as u8);
        }

        Self {
            regret_sum: vec![0.0f32; total].into_boxed_slice(),
            strategy_sum: vec![0.0f32; total].into_boxed_slice(),
            num_actions_per_info_set: num_actions_per_info_set.into_boxed_slice(),
            id_to_idx,
            stride_info_set,
            stride_action,
            combo_width,
            max_actions,
            num_info_sets: descriptors.len(),
        }
    }

    /// Number of info sets the tables were sized for.
    pub fn len(&self) -> usize {
        self.num_info_sets
    }

    /// `true` if there are zero info sets.
    pub fn is_empty(&self) -> bool {
        self.num_info_sets == 0
    }

    /// Width of the combo axis (e.g. 1326 for NLHE).
    pub fn combo_width(&self) -> usize {
        self.combo_width
    }

    /// Max action count across all info sets — the action-axis stride.
    pub fn max_actions(&self) -> usize {
        self.max_actions
    }

    /// Dense index for an info-set id, or `None` if not present.
    pub fn index_of(&self, id: InfoSetId) -> Option<usize> {
        self.id_to_idx.get(&id).copied()
    }

    /// Action count at dense index `idx`.
    pub fn num_actions_at(&self, idx: usize) -> usize {
        self.num_actions_per_info_set[idx] as usize
    }

    /// Action count by `InfoSetId`.
    pub fn num_actions_for(&self, id: InfoSetId) -> Option<usize> {
        self.index_of(id).map(|i| self.num_actions_at(i))
    }

    /// Iterate `(InfoSetId, dense_idx)` pairs.
    pub fn iter_ids(&self) -> impl Iterator<Item = (InfoSetId, usize)> + '_ {
        self.id_to_idx.iter().map(|(k, v)| (*k, *v))
    }

    /// Borrow action-major regret rows for info set `idx`, one combo
    /// slice per action.
    pub fn regret_rows(&self, idx: usize) -> Vec<&[f32]> {
        let n = self.num_actions_at(idx);
        let base = idx * self.stride_info_set;
        (0..n)
            .map(|a| {
                let lo = base + a * self.stride_action;
                &self.regret_sum[lo..lo + self.combo_width]
            })
            .collect()
    }

    /// Mutable action-major regret rows for info set `idx`.
    pub fn regret_rows_mut(&mut self, idx: usize) -> Vec<&mut [f32]> {
        let n = self.num_actions_at(idx);
        let base = idx * self.stride_info_set;
        // Carve up the mutable slice into `n` disjoint action rows.
        // We use `split_at_mut` iteratively so each action gets a
        // distinct `&mut [f32]`.
        let end = base + n * self.stride_action;
        let region: &mut [f32] = &mut self.regret_sum[base..end];
        split_into_rows_mut(region, self.stride_action, n)
    }

    /// Mutable action-major strategy-sum rows for info set `idx`.
    pub fn strategy_sum_rows_mut(&mut self, idx: usize) -> Vec<&mut [f32]> {
        let n = self.num_actions_at(idx);
        let base = idx * self.stride_info_set;
        let end = base + n * self.stride_action;
        let region: &mut [f32] = &mut self.strategy_sum[base..end];
        split_into_rows_mut(region, self.stride_action, n)
    }

    /// Immutable action-major strategy-sum rows for info set `idx`.
    pub fn strategy_sum_rows(&self, idx: usize) -> Vec<&[f32]> {
        let n = self.num_actions_at(idx);
        let base = idx * self.stride_info_set;
        (0..n)
            .map(|a| {
                let lo = base + a * self.stride_action;
                &self.strategy_sum[lo..lo + self.combo_width]
            })
            .collect()
    }
}

/// Carve `region` into `n_rows` equal-width mutable slices of length
/// `row_len`. Used to get `n` disjoint `&mut [f32]` out of a single
/// `&mut [f32]` without `unsafe`.
fn split_into_rows_mut(region: &mut [f32], row_len: usize, n_rows: usize) -> Vec<&mut [f32]> {
    debug_assert_eq!(region.len(), row_len * n_rows);
    let mut out: Vec<&mut [f32]> = Vec::with_capacity(n_rows);
    let mut rest: &mut [f32] = region;
    for _ in 0..n_rows {
        let (head, tail) = rest.split_at_mut(row_len);
        out.push(head);
        rest = tail;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(id: u32, n: usize) -> VectorInfoSetDescriptor {
        VectorInfoSetDescriptor {
            info_set_id: InfoSetId(id),
            num_actions: n,
        }
    }

    #[test]
    fn zero_inits_and_stride_is_correct() {
        let descs = [desc(1, 2), desc(2, 3)];
        let t = VectorCfrTables::new(&descs, 4);
        assert_eq!(t.len(), 2);
        assert_eq!(t.combo_width(), 4);
        assert_eq!(t.max_actions(), 3);
        for (id, idx) in t.iter_ids() {
            assert!(id.0 == 1 || id.0 == 2);
            for row in t.regret_rows(idx) {
                for &v in row {
                    assert_eq!(v, 0.0);
                }
            }
            for row in t.strategy_sum_rows(idx) {
                for &v in row {
                    assert_eq!(v, 0.0);
                }
            }
        }
    }

    #[test]
    fn rows_are_contiguous_and_action_sized() {
        let descs = [desc(7, 5)];
        let t = VectorCfrTables::new(&descs, 1326);
        let idx = t.index_of(InfoSetId(7)).unwrap();
        let rows = t.regret_rows(idx);
        assert_eq!(rows.len(), 5);
        for row in rows {
            assert_eq!(row.len(), 1326);
        }
    }

    #[test]
    fn writes_land_in_the_correct_slot() {
        let descs = [desc(1, 2), desc(2, 2)];
        let mut t = VectorCfrTables::new(&descs, 8);
        let idx1 = t.index_of(InfoSetId(1)).unwrap();
        let idx2 = t.index_of(InfoSetId(2)).unwrap();
        {
            let rows = t.regret_rows_mut(idx1);
            for (a, row) in rows.into_iter().enumerate() {
                for (c, slot) in row.iter_mut().enumerate() {
                    *slot = (a * 10 + c) as f32;
                }
            }
        }
        // InfoSet 1 now has the pattern; InfoSet 2 is untouched.
        let s1 = t.regret_rows(idx1);
        for (a, row) in s1.iter().enumerate() {
            for (c, &v) in row.iter().enumerate() {
                assert_eq!(v, (a * 10 + c) as f32);
            }
        }
        for row in t.regret_rows(idx2) {
            for &v in row {
                assert_eq!(v, 0.0);
            }
        }
        // Strategy sum of InfoSet 1 must also be zero — the tables are
        // independent.
        for row in t.strategy_sum_rows(idx1) {
            for &v in row {
                assert_eq!(v, 0.0);
            }
        }
    }

    #[test]
    fn regret_and_strategy_rows_are_independent() {
        let descs = [desc(1, 2)];
        let mut t = VectorCfrTables::new(&descs, 4);
        let idx = 0;
        {
            let rows = t.regret_rows_mut(idx);
            for row in rows {
                for slot in row.iter_mut() {
                    *slot = 99.0;
                }
            }
        }
        for row in t.strategy_sum_rows(idx) {
            for &v in row {
                assert_eq!(v, 0.0, "strategy table should not alias regret table");
            }
        }
    }

    #[test]
    fn num_actions_accessors_reflect_descriptors() {
        let descs = [desc(1, 2), desc(2, 5), desc(3, 1)];
        let t = VectorCfrTables::new(&descs, 1326);
        assert_eq!(t.num_actions_for(InfoSetId(1)), Some(2));
        assert_eq!(t.num_actions_for(InfoSetId(2)), Some(5));
        assert_eq!(t.num_actions_for(InfoSetId(3)), Some(1));
        assert_eq!(t.num_actions_for(InfoSetId(99)), None);
    }

    #[test]
    #[should_panic(expected = "must be non-empty")]
    fn empty_descriptors_panics() {
        let _ = VectorCfrTables::new(&[], 1326);
    }

    #[test]
    #[should_panic(expected = "combo_width")]
    fn zero_combo_width_panics() {
        let descs = [desc(1, 2)];
        let _ = VectorCfrTables::new(&descs, 0);
    }

    #[test]
    #[should_panic(expected = "duplicate InfoSetId")]
    fn duplicate_info_set_id_panics() {
        let descs = [desc(1, 2), desc(1, 2)];
        let _ = VectorCfrTables::new(&descs, 1326);
    }
}
