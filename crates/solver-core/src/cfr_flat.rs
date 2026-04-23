//! Flat-array CFR+ solver (cache-friendly variant of [`crate::cfr::CfrPlus`]).
//!
//! Functionally identical to [`crate::cfr::CfrPlus`] — same algorithm, same
//! regret-matching+ clamp, same linearly-weighted strategy averaging — but
//! the per-info-set bookkeeping lives in a flat [`RegretTables`] rather
//! than a `HashMap<InfoSetId, Vec<f32>>`. Per
//! `docs/LIMITING_FACTOR.md`, this is step #1 on the optimization ladder
//! (cache-friendly layout), ahead of SIMD and Metal work.
//!
//! # When to use this
//!
//! - For any bounded subgame where you can cheaply enumerate info sets
//!   up-front (river, turn, Kuhn).
//! - As a drop-in replacement for `CfrPlus` when optimization matters and
//!   the info-set count is known.
//!
//! # When to use the HashMap path instead
//!
//! - Early prototyping / games with a dynamically growing action set.
//! - Games where the info-set count is expensive to compute.
//!
//! Both solvers must converge to the same strategy on the same game, and
//! `tests/flat_equivalence.rs` guards that.
//!
//! # API shape
//!
//! The caller provides:
//!
//! 1. The game.
//! 2. A list of `(InfoSetId, num_actions)` pairs enumerating every info
//!    set the solver might touch. Typically produced by
//!    [`enumerate_info_sets`] below.
//! 3. (Implicitly via the enumeration) the max action count, which becomes
//!    the table's stride.
//!
//! Internally the solver builds a `HashMap<InfoSetId, usize>` mapping
//! the opaque game-level ID to a dense array index, and also remembers
//! the per-index action count (for slicing off the padding).
//!
//! The enumeration cost is paid **once** at construction and is
//! negligible compared to the savings in the inner loop: for Kuhn
//! (~12 info sets) it's a single tree walk of a few µs; for an NLHE
//! river subgame (~a few thousand info sets) it's under 1 ms and the
//! 3-5× inner-loop speedup pays for it many times over.

use std::collections::HashMap;

use smallvec::{smallvec, SmallVec};

use crate::cfr::Strategy;
use crate::convergence::exploitability_two_player_zero_sum;
use crate::game::{Game, InfoSetId, Player};
use crate::matching::regret_match;
use crate::tables::RegretTables;

/// Inline capacity for per-node scratch vectors (strategy, action utils).
/// 8 covers every bet tree we ship (max 5 actions) without a heap
/// allocation; anything larger spills to the heap via `SmallVec`.
const MAX_INLINE_ACTIONS: usize = 8;

/// One info-set descriptor as enumerated up-front.
///
/// `info_set_id` is the opaque game-level identifier; `num_actions` is how
/// many legal actions that info set has. The two together let the solver
/// size its flat tables correctly and slice off the per-info-set padding
/// that results from a global `max_actions` stride.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoSetDescriptor {
    /// Opaque game-level identifier.
    pub info_set_id: InfoSetId,
    /// Number of legal actions at this info set.
    pub num_actions: usize,
}

/// Walk the game tree starting from `state` and collect every info set the
/// walk touches, with its legal-action count.
///
/// Used to precompute the info-set universe before constructing
/// [`CfrPlusFlat`]. Deterministic: visits every branch and dedupes by
/// `InfoSetId`. For a game like Kuhn Poker the walk is a few µs; for NLHE
/// river / turn subgames it's typically well under 1 ms.
///
/// For games with a chance layer hoisted above the `Game` trait (Kuhn,
/// NLHE), call this once per chance root and merge the results. See
/// [`enumerate_info_sets_from_roots`] for the convenience form.
pub fn enumerate_info_sets<G: Game>(game: &G, state: &G::State) -> Vec<InfoSetDescriptor> {
    let mut seen: HashMap<InfoSetId, usize> = HashMap::new();
    let mut out: Vec<InfoSetDescriptor> = Vec::new();
    enumerate_walk(game, state, &mut seen, &mut out);
    out
}

/// Enumerate info sets reachable from any of `roots`.
///
/// For games where chance lives outside the `Game` trait (all of ours:
/// Kuhn, NLHE river/turn). `roots` is typically the output of a chance
/// enumeration like `KuhnPoker::chance_roots()`.
pub fn enumerate_info_sets_from_roots<G: Game>(
    game: &G,
    roots: &[(G::State, f32)],
) -> Vec<InfoSetDescriptor> {
    let mut seen: HashMap<InfoSetId, usize> = HashMap::new();
    let mut out: Vec<InfoSetDescriptor> = Vec::new();
    for (root, _) in roots {
        enumerate_walk(game, root, &mut seen, &mut out);
    }
    out
}

fn enumerate_walk<G: Game>(
    game: &G,
    state: &G::State,
    seen: &mut HashMap<InfoSetId, usize>,
    out: &mut Vec<InfoSetDescriptor>,
) {
    if game.is_terminal(state) {
        return;
    }
    let current = game.current_player(state);
    let id = game.info_set(state, current);
    let actions = game.legal_actions(state);
    let num_actions = actions.len();
    debug_assert!(
        num_actions > 0,
        "enumerate_info_sets: non-terminal node with zero legal actions"
    );

    match seen.get(&id) {
        Some(&idx) => {
            debug_assert_eq!(
                out[idx].num_actions, num_actions,
                "enumerate_info_sets: action-set size changed for info set \
                 (Game impls must return consistent action counts per info set)"
            );
        }
        None => {
            let idx = out.len();
            out.push(InfoSetDescriptor {
                info_set_id: id,
                num_actions,
            });
            seen.insert(id, idx);
        }
    }

    for a in &actions {
        let next = game.apply(state, a);
        enumerate_walk(game, &next, seen, out);
    }
}

/// CFR+ solver with flat-array bookkeeping.
///
/// Functionally identical to [`crate::cfr::CfrPlus`], but the per-info-set
/// regret and strategy buffers live in a single contiguous
/// [`RegretTables`] instead of a `HashMap<InfoSetId, Vec<f32>>`. See the
/// module docs for the "why".
pub struct CfrPlusFlat<G: Game> {
    game: G,
    tables: RegretTables,
    /// Map from opaque `InfoSetId` to dense array index.
    id_to_idx: HashMap<InfoSetId, usize>,
    /// Action count per info set, indexed by dense index. Needed to slice
    /// off the global `stride`-wide padding that `RegretTables` enforces.
    num_actions_per_info_set: Box<[usize]>,
    /// 1-based iteration counter. Post-increment inside `iterate()`.
    iteration: u32,
}

impl<G: Game> CfrPlusFlat<G> {
    /// Create a solver for `game` with pre-allocated tables sized for
    /// `descriptors`.
    ///
    /// Every info set the solver encounters during tree walks must be
    /// present in `descriptors` — a `debug_assert` in `walk` checks this.
    /// For release builds a missing info set would silently hit a map
    /// miss and panic on the `expect` below; either way it's a
    /// correctness bug in the caller's enumeration.
    ///
    /// `descriptors` is typically the output of [`enumerate_info_sets`]
    /// or [`enumerate_info_sets_from_roots`]. Duplicates (by
    /// `InfoSetId`) are *not* tolerated — each ID must appear exactly
    /// once.
    ///
    /// # Panics
    ///
    /// Panics if `descriptors` is empty or if it contains a duplicate
    /// `InfoSetId`.
    pub fn new(game: G, descriptors: &[InfoSetDescriptor]) -> Self {
        assert!(
            !descriptors.is_empty(),
            "CfrPlusFlat::new: descriptors must be non-empty"
        );
        let max_actions = descriptors
            .iter()
            .map(|d| d.num_actions)
            .max()
            .expect("non-empty checked above");
        assert!(
            max_actions > 0,
            "CfrPlusFlat::new: every info set must have > 0 actions"
        );

        let mut id_to_idx: HashMap<InfoSetId, usize> = HashMap::with_capacity(descriptors.len());
        let mut num_actions_per_info_set: Vec<usize> = Vec::with_capacity(descriptors.len());
        for (i, d) in descriptors.iter().enumerate() {
            let prev = id_to_idx.insert(d.info_set_id, i);
            assert!(
                prev.is_none(),
                "CfrPlusFlat::new: duplicate InfoSetId {:?} at dense index {}",
                d.info_set_id,
                i
            );
            num_actions_per_info_set.push(d.num_actions);
        }

        let tables = RegretTables::new(descriptors.len(), max_actions);

        Self {
            game,
            tables,
            id_to_idx,
            num_actions_per_info_set: num_actions_per_info_set.into_boxed_slice(),
            iteration: 0,
        }
    }

    /// Convenience: enumerate info sets from a single-root game and
    /// return a solver for it. Prefer [`CfrPlusFlat::from_roots`] for
    /// games with a chance layer.
    pub fn from_initial_state(game: G) -> Self
    where
        G::State: Clone,
    {
        let root = game.initial_state();
        let descriptors = enumerate_info_sets(&game, &root);
        Self::new(game, &descriptors)
    }

    /// Convenience: enumerate info sets from a chance-weighted root
    /// mixture and return a solver for it.
    pub fn from_roots(game: G, roots: &[(G::State, f32)]) -> Self
    where
        G::State: Clone,
    {
        let descriptors = enumerate_info_sets_from_roots(&game, roots);
        Self::new(game, &descriptors)
    }

    /// Borrow the underlying game.
    pub fn game(&self) -> &G {
        &self.game
    }

    /// Number of CFR+ iterations completed.
    pub fn iterations(&self) -> u32 {
        self.iteration
    }

    /// Number of info sets the tables were sized for.
    pub fn num_info_sets(&self) -> usize {
        self.tables.len()
    }

    /// Run one CFR+ iteration from the game's initial state.
    pub fn iterate(&mut self) {
        let root = self.game.initial_state();
        self.iterate_from(&[(root, 1.0)])
    }

    /// Run one CFR+ iteration starting from a chance-weighted root
    /// mixture. Semantics match [`crate::cfr::CfrPlus::iterate_from`].
    pub fn iterate_from(&mut self, roots: &[(G::State, f32)]) {
        self.iteration = self.iteration.saturating_add(1);

        for &update_player in &[Player::Hero, Player::Villain] {
            for (root, weight) in roots {
                let w = *weight;
                debug_assert!(w >= 0.0, "chance-layer weight must be non-negative");
                if w == 0.0 {
                    continue;
                }
                self.walk(root, update_player, w, w);
            }
        }
    }

    /// Run `iterations` iterations of CFR+.
    pub fn run(&mut self, iterations: u32) {
        for _ in 0..iterations {
            self.iterate();
        }
    }

    /// Run `iterations` iterations of CFR+, starting each iteration
    /// from the chance-weighted mixture `roots`.
    pub fn run_from(&mut self, roots: &[(G::State, f32)], iterations: u32) {
        for _ in 0..iterations {
            self.iterate_from(roots);
        }
    }

    /// CFR tree walk — the flat-array analogue of
    /// [`crate::cfr::CfrPlus::walk`].
    ///
    /// The algorithmic structure is identical to the HashMap path. The
    /// only meaningful differences are:
    /// - Info-set lookup goes through `id_to_idx` instead of a
    ///   `HashMap<InfoSetId, InfoSetEntry>` find.
    /// - Regret / strategy_sum / current_strategy slices come from
    ///   `RegretTables` and are indexed contiguously.
    ///
    /// Returns the utility of this subtree for `update_player`, with the
    /// `update_player`'s own reach factored out (the counterfactual
    /// form).
    fn walk(
        &mut self,
        state: &G::State,
        update_player: Player,
        reach_hero: f32,
        reach_villain: f32,
    ) -> f32 {
        if self.game.is_terminal(state) {
            return self.game.utility(state, update_player);
        }

        let current = self.game.current_player(state);
        let actions = self.game.legal_actions(state);
        let num_actions = actions.len();
        assert!(
            num_actions > 0,
            "CfrPlusFlat::walk: non-terminal node with zero legal actions"
        );

        let info_set_id = self.game.info_set(state, current);
        let idx = *self
            .id_to_idx
            .get(&info_set_id)
            .expect("CfrPlusFlat::walk: unknown info set — caller's descriptor list is incomplete");
        debug_assert_eq!(
            self.num_actions_per_info_set[idx], num_actions,
            "CfrPlusFlat::walk: action-set size changed for an info set \
             (Game impls must return consistent action counts per info set)"
        );

        // Regret matching into the scratch buffer. We only use the first
        // `num_actions` slots; the rest of the stride is padding left at
        // zero and ignored.
        //
        // We copy the strategy out into a stack-allocated SmallVec before
        // recursing so we can drop the mutable `self.tables` borrow.
        // SmallVec with inline capacity `MAX_INLINE_ACTIONS` covers every
        // bet tree we actually use in v0.1 (Kuhn is 2 actions, NLHE is
        // ≤5) without touching the heap — that matters a lot at Kuhn
        // scale where allocation dominates the per-walk cost.
        let strategy: SmallVec<[f32; MAX_INLINE_ACTIONS]> = {
            let (regrets_full, scratch_full) = self.tables.regrets_and_current_mut(idx);
            let regrets = &regrets_full[..num_actions];
            let scratch = &mut scratch_full[..num_actions];
            regret_match(regrets, scratch);
            SmallVec::from_slice(scratch)
        };

        // Recurse over each action, collecting child utilities for the
        // *update* player.
        let mut action_utils: SmallVec<[f32; MAX_INLINE_ACTIONS]> = smallvec![0.0f32; num_actions];
        let mut node_util = 0.0f32;

        for (i, action) in actions.iter().enumerate() {
            let next = self.game.apply(state, action);
            let p = strategy[i];

            let (next_hero, next_villain) = match current {
                Player::Hero => (reach_hero * p, reach_villain),
                Player::Villain => (reach_hero, reach_villain * p),
            };

            let u = self.walk(&next, update_player, next_hero, next_villain);
            action_utils[i] = u;
            node_util += p * u;
        }

        // Regret / strategy updates only on nodes where `current` is the
        // player we're updating on this walk. Identical arithmetic to
        // CfrPlus::walk.
        if current == update_player {
            let cf_reach = match update_player {
                Player::Hero => reach_villain,
                Player::Villain => reach_hero,
            };
            let own_reach = match update_player {
                Player::Hero => reach_hero,
                Player::Villain => reach_villain,
            };
            let linear_weight = self.iteration as f32;

            let regrets = &mut self.tables.regrets_mut(idx)[..num_actions];
            for i in 0..num_actions {
                let regret = action_utils[i] - node_util;
                let updated = regrets[i] + cf_reach * regret;
                regrets[i] = if updated > 0.0 { updated } else { 0.0 };
            }
            let strategy_sum = &mut self.tables.strategy_sum_mut(idx)[..num_actions];
            for i in 0..num_actions {
                strategy_sum[i] += linear_weight * own_reach * strategy[i];
            }
        }

        node_util
    }

    /// Produce the average (near-Nash) strategy across all iterations.
    ///
    /// Equivalent to [`crate::cfr::CfrPlus::average_strategy`]: the
    /// normalized `strategy_sum` per info set, with a uniform fallback
    /// for info sets whose sum is zero (never reached by the updating
    /// player).
    pub fn average_strategy(&self) -> Strategy {
        let mut out = Strategy::default();
        for (&id, &idx) in self.id_to_idx.iter() {
            let n = self.num_actions_per_info_set[idx];
            let slice = &self.tables.strategy_sum(idx)[..n];
            let sum: f32 = slice.iter().sum();
            let v = if sum > 0.0 {
                let inv = 1.0 / sum;
                slice.iter().map(|x| x * inv).collect()
            } else {
                let u = 1.0 / (n as f32);
                vec![u; n]
            };
            out.insert(id, v);
        }
        out
    }

    /// Exploitability of the current average strategy, in the game's
    /// native utility units. See
    /// [`crate::convergence::exploitability_two_player_zero_sum`].
    pub fn exploitability(&self) -> f32 {
        let strategy = self.average_strategy();
        exploitability_two_player_zero_sum(&self.game, &strategy)
    }
}
