//! Vector CFR+ — combo-axis-SIMD inner loop.
//!
//! The Vector CFR solver walks the **action-only game tree** once per
//! iteration, carrying hero and villain reach vectors of length
//! `combo_width` (1326 for NLHE) as it descends. At each decision node
//! we run regret-matching across combo lanes in parallel (via
//! [`regret_match_simd_vector`]); at each terminal we ask the game to
//! fill a combo-wide utility vector. The total tree walk count per CFR
//! iteration drops from `num_chance_roots` (one walk per (hero_combo,
//! villain_combo) pair — hundreds to thousands on a full river
//! subgame) to **one walk** over the action-only tree, with all the
//! combo parallelism amortized inside.
//!
//! # Why this is the v0.2 "10× speedup" path
//!
//! The scalar / flat-array path ([`crate::CfrPlus`],
//! [`crate::CfrPlusFlat`]) walks the tree once per chance root. A
//! real NLHE river subgame has ~100–2000 non-conflicting combo pairs,
//! so the scalar path does 100–2000 tree walks per CFR+ iteration.
//! Every walk hits the same set of info sets (they're keyed on
//! action history, not combo), so the data-dependency pattern is:
//! "thread a 1326-wide vector through a small tree, hitting each
//! node O(1) times, not O(combo_pairs)."
//!
//! Combined with the combo-lane-major regret-matching primitive
//! ([`regret_match_simd_vector`]), which vectorizes the 1326-wide
//! inner loop at 8 lanes per f32x8 op, this closes the gap to the
//! v0.1 target (< 300 ms @ 1000 iters on `river_canonical_spot`,
//! extrapolated from the post-A64 4.35 ms/iter).
//!
//! # Algorithmic caveat: trajectory ≠ scalar CFR+
//!
//! The scalar paths ([`crate::CfrPlus`], [`crate::CfrPlusFlat`]) run
//! `N_roots` sequential tree walks per iteration, with each walk
//! mutating regrets before the next walk starts. The Vector CFR
//! walker does one batched walk per iteration; regrets for ALL
//! (hero_combo, villain_combo) pairs are computed from the same
//! starting strategy and applied together. Per-iteration strategies
//! differ slightly between the two approaches; both converge to the
//! same Nash equilibrium, but bit-identical equivalence isn't
//! achievable. The 3-way equivalence test in
//! `tests/vector_equivalence.rs` validates Nash-level convergence
//! within a looser tolerance.
//!
//! # The `VectorGame` trait
//!
//! Generic over any two-player zero-sum game that can provide:
//!
//! 1. An **action-only state** whose identity does **not** depend on
//!    the combo (so one decision node represents all 1326 hero
//!    combos' choices at that point in history).
//! 2. A **terminal-utility hook** `fill_terminal_utility(state,
//!    update_player, reach_opp, out)` that writes per-combo
//!    counterfactual values for `update_player` into a
//!    `combo_width`-length slice. `out[my_combo] = Σ_{opp_combo}
//!    valid(my_combo, opp_combo) * reach_opp[opp_combo] *
//!    utility_for(update_player, my_combo, opp_combo, state)`. The
//!    pair-validity (Kuhn: `h != v`; NLHE: no shared cards) is the
//!    impl's responsibility.
//! 3. An **initial reach** seeder (what reach vectors to use at the
//!    root for each player). For NLHE this is the range weights
//!    (lane c = range weight of combo c); for Kuhn, uniform 1.0 per
//!    card.
//! 4. An **info-set ID** keyed on acting player + action history.

use std::collections::HashMap;

use smallvec::SmallVec;

use crate::cfr::Strategy;
use crate::game::{InfoSetId, Player};
use crate::matching_simd::regret_match_simd_vector;
use crate::tables_vector::{VectorCfrTables, VectorInfoSetDescriptor};

/// Inline capacity for per-node action scratch vectors.
const MAX_INLINE_ACTIONS: usize = 8;

/// A game the Vector CFR solver can drive.
///
/// See the module docs for the design rationale.
pub trait VectorGame {
    /// Action-only state (no combo in the state).
    type State: Clone;

    /// Action type.
    type Action: Clone;

    /// Combo-axis width. Constant for a given game (1326 for NLHE, 3
    /// for Kuhn).
    fn combo_width(&self) -> usize;

    /// Root state of the action-only tree (empty action history).
    fn root(&self) -> Self::State;

    /// Is this state terminal?
    fn is_terminal(&self, state: &Self::State) -> bool;

    /// At a non-terminal state, returns the player to act.
    fn current_player(&self, state: &Self::State) -> Player;

    /// Legal actions at `state`. Must be non-empty for non-terminals.
    fn legal_actions(&self, state: &Self::State) -> SmallVec<[Self::Action; MAX_INLINE_ACTIONS]>;

    /// Apply `action` to `state`, returning the successor.
    fn apply(&self, state: &Self::State, action: &Self::Action) -> Self::State;

    /// Opaque info-set ID keyed on `(acting_player, action_history)`.
    /// Must NOT depend on the combo.
    fn info_set_id(&self, state: &Self::State, player: Player) -> InfoSetId;

    /// Initial reach vector for `player` — the per-combo prior weight
    /// at the root of the walk.
    ///
    /// For NLHE: the range weights (scaled by chance prior as
    /// appropriate). For Kuhn: uniform 1.0 per card.
    ///
    /// The walker multiplies this lane-wise by strategy probabilities
    /// as it descends each decision branch. At terminal, the opponent's
    /// reach is what's passed to `fill_terminal_utility`, so it
    /// represents the full product (chance × opponent strategies).
    fn initial_reach(&self, player: Player, out: &mut [f32]);

    /// At a terminal, fill `out[my_combo]` with the **reach-weighted**
    /// counterfactual value for `update_player` at that combo,
    /// integrating over the opponent's combos via `reach_opp`.
    ///
    /// Formally:
    ///   `out[my_combo] = Σ_{opp_combo} valid(my_combo, opp_combo)
    ///                  * reach_opp[opp_combo]
    ///                  * utility_for(update_player, my_combo, opp_combo, state)`
    ///
    /// The pair-validity constraint (Kuhn: `h != v`; NLHE: no shared
    /// cards) MUST be respected by the impl — the walker assumes it
    /// is, so this is the single place the conflict mask applies.
    ///
    /// `out.len() == self.combo_width()`. `reach_opp.len() ==
    /// self.combo_width()`.
    fn fill_terminal_utility(
        &self,
        state: &Self::State,
        update_player: Player,
        reach_opp: &[f32],
        out: &mut [f32],
    );
}

/// Walk the action-only game tree from the root and collect every info
/// set it touches.
pub fn enumerate_vector_info_sets<G: VectorGame>(game: &G) -> Vec<VectorInfoSetDescriptor> {
    let mut seen: HashMap<InfoSetId, usize> = HashMap::new();
    let mut out: Vec<VectorInfoSetDescriptor> = Vec::new();
    let root = game.root();
    enumerate_walk(game, &root, &mut seen, &mut out);
    out
}

fn enumerate_walk<G: VectorGame>(
    game: &G,
    state: &G::State,
    seen: &mut HashMap<InfoSetId, usize>,
    out: &mut Vec<VectorInfoSetDescriptor>,
) {
    if game.is_terminal(state) {
        return;
    }
    let current = game.current_player(state);
    let id = game.info_set_id(state, current);
    let actions = game.legal_actions(state);
    let num_actions = actions.len();
    debug_assert!(
        num_actions > 0,
        "enumerate_vector_info_sets: non-terminal with zero legal actions"
    );
    match seen.get(&id) {
        Some(&idx) => {
            debug_assert_eq!(
                out[idx].num_actions, num_actions,
                "VectorGame: action-set size changed for info set"
            );
        }
        None => {
            let idx = out.len();
            out.push(VectorInfoSetDescriptor {
                info_set_id: id,
                num_actions,
            });
            seen.insert(id, idx);
        }
    }
    for a in actions.iter() {
        let next = game.apply(state, a);
        enumerate_walk(game, &next, seen, out);
    }
}

/// Vector CFR+ solver.
///
/// Owns the action-only game, the pre-sized [`VectorCfrTables`], an
/// iteration counter, and a depth-indexed scratch pool so the walker
/// doesn't allocate per-recursion.
pub struct CfrPlusVector<G: VectorGame> {
    game: G,
    tables: VectorCfrTables,
    /// 1-based iteration counter (incremented inside `iterate()`).
    iteration: u32,
    /// Combo-axis width cache.
    combo_width: usize,
    /// Depth-indexed scratch buffers. Grown on demand.
    ///
    /// At depth `d` the walker needs:
    /// - `strategy_buf[d]`: `max_actions` rows of `combo_width` floats
    ///   for the per-combo regret-matched strategy.
    /// - `action_utils_buf[d]`: `max_actions` rows of `combo_width`.
    /// - `node_util_buf[d]`: `combo_width` floats.
    /// - `next_hero_reach_buf[d]`, `next_villain_reach_buf[d]`:
    ///   `combo_width` each.
    scratch: ScratchPool,
    /// Maximum action count (stride of the `strategy_buf` /
    /// `action_utils_buf` sub-tables per depth).
    max_actions: usize,
}

/// Per-depth scratch pool. All buffers grow on demand; the recursion
/// depth for NLHE v0.1 is bounded by the bet tree (~10 levels), so
/// this amortizes to O(1) allocation over a full solve.
struct ScratchPool {
    /// `strategy[d * max_actions + a]` — `combo_width` floats.
    strategy: Vec<Vec<f32>>,
    /// `action_utils[d * max_actions + a]` — `combo_width` floats.
    action_utils: Vec<Vec<f32>>,
    /// `node_util[d]` — `combo_width` floats.
    node_util: Vec<Vec<f32>>,
    /// `next_hero_reach[d]` — `combo_width` floats.
    next_hero_reach: Vec<Vec<f32>>,
    /// `next_villain_reach[d]` — `combo_width` floats.
    next_villain_reach: Vec<Vec<f32>>,
    /// Initial reach vectors for the root; allocated once.
    root_reach_hero: Vec<f32>,
    root_reach_villain: Vec<f32>,
    /// Out buffer for the root-level walk.
    root_util: Vec<f32>,
    combo_width: usize,
    max_actions: usize,
}

impl ScratchPool {
    fn new(combo_width: usize, max_actions: usize) -> Self {
        Self {
            strategy: Vec::new(),
            action_utils: Vec::new(),
            node_util: Vec::new(),
            next_hero_reach: Vec::new(),
            next_villain_reach: Vec::new(),
            root_reach_hero: vec![0.0f32; combo_width],
            root_reach_villain: vec![0.0f32; combo_width],
            root_util: vec![0.0f32; combo_width],
            combo_width,
            max_actions,
        }
    }

    /// Ensure depth `d` has scratch. Grows on demand. Idempotent.
    fn ensure_depth(&mut self, d: usize) {
        while self.node_util.len() <= d {
            self.node_util.push(vec![0.0f32; self.combo_width]);
            self.next_hero_reach.push(vec![0.0f32; self.combo_width]);
            self.next_villain_reach.push(vec![0.0f32; self.combo_width]);
            for _ in 0..self.max_actions {
                self.strategy.push(vec![0.0f32; self.combo_width]);
                self.action_utils.push(vec![0.0f32; self.combo_width]);
            }
        }
    }
}

impl<G: VectorGame> CfrPlusVector<G> {
    /// Construct a solver. Enumerates info sets from the game root.
    pub fn new(game: G) -> Self {
        let descriptors = enumerate_vector_info_sets(&game);
        Self::with_descriptors(game, &descriptors)
    }

    /// Construct a solver with an explicit descriptor list.
    pub fn with_descriptors(game: G, descriptors: &[VectorInfoSetDescriptor]) -> Self {
        let combo_width = game.combo_width();
        let tables = VectorCfrTables::new(descriptors, combo_width);
        let max_actions = tables.max_actions();
        let scratch = ScratchPool::new(combo_width, max_actions);
        Self {
            game,
            tables,
            iteration: 0,
            combo_width,
            scratch,
            max_actions,
        }
    }

    /// Borrow the game.
    pub fn game(&self) -> &G {
        &self.game
    }

    /// Iterations completed so far.
    pub fn iterations(&self) -> u32 {
        self.iteration
    }

    /// Number of info sets the tables were sized for.
    pub fn num_info_sets(&self) -> usize {
        self.tables.len()
    }

    /// Combo-axis width.
    pub fn combo_width(&self) -> usize {
        self.combo_width
    }

    /// Run `iterations` iterations of Vector CFR+.
    pub fn run(&mut self, iterations: u32) {
        for _ in 0..iterations {
            self.iterate();
        }
    }

    /// Run one CFR+ iteration.
    pub fn iterate(&mut self) {
        self.iteration = self.iteration.saturating_add(1);

        // Seed root reach vectors (reused across walks this iteration).
        self.game
            .initial_reach(Player::Hero, &mut self.scratch.root_reach_hero);
        self.game
            .initial_reach(Player::Villain, &mut self.scratch.root_reach_villain);

        let root = self.game.root();

        for &update_player in &[Player::Hero, Player::Villain] {
            // `mem::take` the root buffers out so the walker can take
            // &mut self; restore on return.
            let reach_hero = std::mem::take(&mut self.scratch.root_reach_hero);
            let reach_villain = std::mem::take(&mut self.scratch.root_reach_villain);
            let mut util = std::mem::take(&mut self.scratch.root_util);

            self.walk(
                0,
                &root,
                update_player,
                &reach_hero,
                &reach_villain,
                &mut util,
            );

            self.scratch.root_reach_hero = reach_hero;
            self.scratch.root_reach_villain = reach_villain;
            self.scratch.root_util = util;
        }
    }

    /// Vector CFR+ tree walk.
    ///
    /// - `depth` — recursion depth; indexes into the scratch pool.
    /// - `state` — current node.
    /// - `update_player` — which player's regrets this walk updates.
    /// - `reach_hero` / `reach_villain` — per-combo reach vectors at
    ///   this node (initial_reach × strategy products on the path).
    /// - `out_util` — buffer the walker writes `update_player`'s
    ///   per-combo subtree counterfactual value into.
    fn walk(
        &mut self,
        depth: usize,
        state: &G::State,
        update_player: Player,
        reach_hero: &[f32],
        reach_villain: &[f32],
        out_util: &mut [f32],
    ) {
        debug_assert_eq!(reach_hero.len(), self.combo_width);
        debug_assert_eq!(reach_villain.len(), self.combo_width);
        debug_assert_eq!(out_util.len(), self.combo_width);

        if self.game.is_terminal(state) {
            let reach_opp = match update_player {
                Player::Hero => reach_villain,
                Player::Villain => reach_hero,
            };
            self.game
                .fill_terminal_utility(state, update_player, reach_opp, out_util);
            return;
        }

        let current = self.game.current_player(state);
        let actions = self.game.legal_actions(state);
        let num_actions = actions.len();
        assert!(
            num_actions > 0,
            "CfrPlusVector::walk: non-terminal with zero legal actions"
        );

        let info_set_id = self.game.info_set_id(state, current);
        let idx = self
            .tables
            .index_of(info_set_id)
            .expect("CfrPlusVector::walk: unknown info set");

        // Ensure this depth's scratch exists (grows on demand; amortized
        // O(1) over a full solve).
        self.scratch.ensure_depth(depth);
        let cw = self.combo_width;
        let strat_base = depth * self.max_actions;
        let au_base = depth * self.max_actions;

        // Regret-match into the scratch strategy rows.
        {
            let regret_rows = self.tables.regret_rows(idx);
            let regret_refs: SmallVec<[&[f32]; MAX_INLINE_ACTIONS]> =
                regret_rows.iter().copied().collect();
            let strat_slice = &mut self.scratch.strategy[strat_base..strat_base + num_actions];
            let mut strat_refs: SmallVec<[&mut [f32]; MAX_INLINE_ACTIONS]> = SmallVec::new();
            for row in strat_slice.iter_mut() {
                strat_refs.push(row.as_mut_slice());
            }
            regret_match_simd_vector(&regret_refs, &mut strat_refs);
        }

        // Take scratch out so we can re-borrow self mutably inside the
        // recursive calls.
        let mut strategy_take: SmallVec<[Vec<f32>; MAX_INLINE_ACTIONS]> = SmallVec::new();
        let mut au_take: SmallVec<[Vec<f32>; MAX_INLINE_ACTIONS]> = SmallVec::new();
        for i in 0..num_actions {
            strategy_take.push(std::mem::take(&mut self.scratch.strategy[strat_base + i]));
            au_take.push(std::mem::take(&mut self.scratch.action_utils[au_base + i]));
        }
        let mut node_util_take = std::mem::take(&mut self.scratch.node_util[depth]);
        let mut next_hero_take = std::mem::take(&mut self.scratch.next_hero_reach[depth]);
        let mut next_villain_take = std::mem::take(&mut self.scratch.next_villain_reach[depth]);

        for slot in node_util_take.iter_mut() {
            *slot = 0.0;
        }

        for (i, action) in actions.iter().enumerate() {
            let next = self.game.apply(state, action);
            let p = &strategy_take[i];
            match current {
                Player::Hero => {
                    for (c, slot) in next_hero_take.iter_mut().enumerate() {
                        *slot = reach_hero[c] * p[c];
                    }
                    next_villain_take.copy_from_slice(reach_villain);
                    self.walk(
                        depth + 1,
                        &next,
                        update_player,
                        &next_hero_take,
                        &next_villain_take,
                        &mut au_take[i],
                    );
                }
                Player::Villain => {
                    for (c, slot) in next_villain_take.iter_mut().enumerate() {
                        *slot = reach_villain[c] * p[c];
                    }
                    next_hero_take.copy_from_slice(reach_hero);
                    self.walk(
                        depth + 1,
                        &next,
                        update_player,
                        &next_hero_take,
                        &next_villain_take,
                        &mut au_take[i],
                    );
                }
            }

            // node_util aggregation: see module docs.
            let au = &au_take[i];
            if current == update_player {
                for (c, slot) in node_util_take.iter_mut().enumerate() {
                    *slot += p[c] * au[c];
                }
            } else {
                for (c, slot) in node_util_take.iter_mut().enumerate() {
                    *slot += au[c];
                }
            }
        }

        out_util.copy_from_slice(&node_util_take);

        // Regret + strategy_sum updates at update_player's own nodes.
        if current == update_player {
            let own_reach: &[f32] = match update_player {
                Player::Hero => reach_hero,
                Player::Villain => reach_villain,
            };
            let linear_weight = self.iteration as f32;

            let regret_rows = self.tables.regret_rows_mut(idx);
            for (a_idx, row) in regret_rows.into_iter().enumerate() {
                let au = &au_take[a_idx];
                for c in 0..cw {
                    let raw = row[c] + (au[c] - node_util_take[c]);
                    row[c] = if raw > 0.0 { raw } else { 0.0 };
                }
            }
            let strategy_rows = self.tables.strategy_sum_rows_mut(idx);
            for (a_idx, row) in strategy_rows.into_iter().enumerate() {
                let s = &strategy_take[a_idx];
                for c in 0..cw {
                    row[c] += linear_weight * own_reach[c] * s[c];
                }
            }
        }

        // Put scratch back so the next call at this depth can reuse.
        self.scratch.node_util[depth] = node_util_take;
        self.scratch.next_hero_reach[depth] = next_hero_take;
        self.scratch.next_villain_reach[depth] = next_villain_take;
        for (i, v) in au_take.into_iter().enumerate() {
            self.scratch.action_utils[au_base + i] = v;
        }
        for (i, v) in strategy_take.into_iter().enumerate() {
            self.scratch.strategy[strat_base + i] = v;
        }
    }

    /// Return the aggregate-across-combos average strategy as a
    /// `Strategy` map (scalar-per-action, sum to 1). Used for the
    /// scalar-solver equivalence tests.
    pub fn average_strategy(&self) -> Strategy {
        let mut out = Strategy::default();
        for (id, idx) in self.tables.iter_ids() {
            let rows = self.tables.strategy_sum_rows(idx);
            let n = rows.len();
            let per_action_sum: Vec<f32> = rows.iter().map(|row| row.iter().sum::<f32>()).collect();
            let total: f32 = per_action_sum.iter().sum();
            let v = if total > 0.0 {
                let inv = 1.0 / total;
                per_action_sum.iter().map(|x| x * inv).collect()
            } else {
                let u = 1.0 / (n as f32);
                vec![u; n]
            };
            out.insert(id, v);
        }
        out
    }

    /// Per-combo average strategy at info set `id`.
    ///
    /// Returns `num_actions × combo_width` matrix. `None` if the
    /// info set wasn't enumerated.
    pub fn per_combo_average_strategy(&self, id: InfoSetId) -> Option<Vec<Vec<f32>>> {
        let idx = self.tables.index_of(id)?;
        let rows = self.tables.strategy_sum_rows(idx);
        let n = rows.len();
        let cw = self.combo_width;
        let mut result: Vec<Vec<f32>> = (0..n).map(|_| vec![0.0f32; cw]).collect();
        let mut per_combo_total = vec![0.0f32; cw];
        for (a, row) in rows.iter().enumerate() {
            result[a][..cw].copy_from_slice(&row[..cw]);
            for (c, t) in per_combo_total.iter_mut().enumerate() {
                *t += row[c];
            }
        }
        let uniform = 1.0 / (n as f32);
        for (c, &t) in per_combo_total.iter().enumerate() {
            if t > 0.0 {
                let inv = 1.0 / t;
                for row in result.iter_mut() {
                    row[c] *= inv;
                }
            } else {
                for row in result.iter_mut() {
                    row[c] = uniform;
                }
            }
        }
        Some(result)
    }
}
