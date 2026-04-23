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
/// Owns the action-only game, the pre-sized [`VectorCfrTables`], and
/// an iteration counter.
pub struct CfrPlusVector<G: VectorGame> {
    game: G,
    tables: VectorCfrTables,
    /// 1-based iteration counter (incremented inside `iterate()`).
    iteration: u32,
    /// Combo-axis width cache.
    combo_width: usize,
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
        Self {
            game,
            tables,
            iteration: 0,
            combo_width,
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

        for &update_player in &[Player::Hero, Player::Villain] {
            let mut reach_hero = vec![0.0f32; self.combo_width];
            let mut reach_villain = vec![0.0f32; self.combo_width];
            self.game.initial_reach(Player::Hero, &mut reach_hero);
            self.game.initial_reach(Player::Villain, &mut reach_villain);

            let root = self.game.root();
            let mut util = vec![0.0f32; self.combo_width];
            self.walk(&root, update_player, &reach_hero, &reach_villain, &mut util);
        }
    }

    /// Vector CFR+ tree walk.
    ///
    /// - `state` — current node.
    /// - `update_player` — the player whose regrets we update on this walk.
    /// - `reach_hero` / `reach_villain` — per-combo reach vectors at
    ///   this node (initial_reach × strategy products on the path).
    /// - `out_util` — scratch buffer the walker writes `update_player`'s
    ///   per-combo subtree counterfactual value into. Counterfactual
    ///   means: the expected utility for `update_player` with combo
    ///   `c`, already reach-weighted by the opponent (NOT by
    ///   `update_player`'s own reach or by `update_player`'s past
    ///   strategy).
    fn walk(
        &mut self,
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

        // Regret-match the current strategy into per-action combo-wide
        // scratch. One SIMD-vectorized call across 1326 lanes.
        let strategy: Vec<Vec<f32>> = {
            let regret_rows = self.tables.regret_rows(idx);
            let refs: SmallVec<[&[f32]; MAX_INLINE_ACTIONS]> =
                regret_rows.iter().copied().collect();
            let mut strat_buf: Vec<Vec<f32>> = (0..num_actions)
                .map(|_| vec![0.0f32; self.combo_width])
                .collect();
            {
                let mut strat_refs: SmallVec<[&mut [f32]; MAX_INLINE_ACTIONS]> =
                    strat_buf.iter_mut().map(|v| v.as_mut_slice()).collect();
                regret_match_simd_vector(&refs, &mut strat_refs);
            }
            strat_buf
        };

        // Recurse over each action. For each, propagate the current
        // player's reach through the strategy on lane `c`:
        //   - If current == Hero: next_hero_reach[c] = reach_hero[c] * strategy[a][c]
        //   - If current == Villain: next_villain_reach[c] = reach_villain[c] * strategy[a][c]
        //
        // The other player's reach is unchanged. We recurse and the child
        // returns action_util[a][c] = counterfactual value of the subtree
        // for update_player at their combo c, which already has the
        // opponent's reach multiplied in via the terminal.
        let mut action_utils: Vec<Vec<f32>> = (0..num_actions)
            .map(|_| vec![0.0f32; self.combo_width])
            .collect();
        let mut node_util = vec![0.0f32; self.combo_width];

        let mut next_hero_reach = vec![0.0f32; self.combo_width];
        let mut next_villain_reach = vec![0.0f32; self.combo_width];

        for (i, action) in actions.iter().enumerate() {
            let next = self.game.apply(state, action);
            let p = &strategy[i];
            match current {
                Player::Hero => {
                    for (c, slot) in next_hero_reach.iter_mut().enumerate() {
                        *slot = reach_hero[c] * p[c];
                    }
                    next_villain_reach.copy_from_slice(reach_villain);
                }
                Player::Villain => {
                    for (c, slot) in next_villain_reach.iter_mut().enumerate() {
                        *slot = reach_villain[c] * p[c];
                    }
                    next_hero_reach.copy_from_slice(reach_hero);
                }
            }
            self.walk(
                &next,
                update_player,
                &next_hero_reach,
                &next_villain_reach,
                &mut action_utils[i],
            );

            // node_util aggregation:
            //   * At the UPDATE player's own node: node_util[c] =
            //     Σ_a strategy[a][c] * action_util[a][c] (i.e.,
            //     "following σ for update_player with combo c").
            //   * At the NON-update player's node: node_util[c] =
            //     Σ_a action_util[a][c] — no strategy factor, because
            //     the non-update player's strategy has already been
            //     folded into their reach propagation, and the child
            //     walks have baked it into the terminal integration
            //     via `reach_opp`. See module docs for the derivation.
            //
            // This split is the load-bearing part of the per-node
            // CFR math; a uniform `p[c] * action_util[a][c]` at the
            // non-update node would double-count the strategy factor
            // and diverge from Nash.
            if current == update_player {
                for (c, slot) in node_util.iter_mut().enumerate() {
                    *slot += p[c] * action_utils[i][c];
                }
            } else {
                for (c, slot) in node_util.iter_mut().enumerate() {
                    *slot += action_utils[i][c];
                }
            }
        }

        out_util.copy_from_slice(&node_util);

        // Regret / strategy-sum updates only at update-player nodes.
        if current == update_player {
            let own_reach: &[f32] = match update_player {
                Player::Hero => reach_hero,
                Player::Villain => reach_villain,
            };
            let linear_weight = self.iteration as f32;
            let cw = self.combo_width;

            // Regret update: regret[a][c] += action_util[a][c] -
            // node_util[c]. The opponent-reach (counterfactual) factor
            // is already baked into both action_util and node_util via
            // the terminal's `reach_opp` weighting, so no cf_reach
            // multiplication is needed here. See the module docs for
            // the derivation.
            let regret_rows = self.tables.regret_rows_mut(idx);
            for (a_idx, row) in regret_rows.into_iter().enumerate() {
                let au = &action_utils[a_idx];
                for c in 0..cw {
                    let raw = row[c] + (au[c] - node_util[c]);
                    row[c] = if raw > 0.0 { raw } else { 0.0 };
                }
            }

            // Strategy-sum: linearly-weighted, scaled by own reach and
            // the per-combo strategy probability.
            let strategy_rows = self.tables.strategy_sum_rows_mut(idx);
            for (a_idx, row) in strategy_rows.into_iter().enumerate() {
                let s = &strategy[a_idx];
                for c in 0..cw {
                    row[c] += linear_weight * own_reach[c] * s[c];
                }
            }
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
