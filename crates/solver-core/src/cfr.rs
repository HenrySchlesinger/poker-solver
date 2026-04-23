//! CFR+ implementation.
//!
//! Vanilla Counterfactual Regret Minimization with the CFR+ modifications:
//! - Regret sums clamped to `>= 0` on each update (regret matching+)
//! - Strategy averaging weighted linearly by iteration number
//!
//! The implementation is deliberately simple and game-agnostic: it walks
//! the game tree via the [`Game`] trait on every iteration, accumulating
//! regrets and (linearly-weighted) strategy contributions at each info
//! set for each (state, action) pair.
//!
//! For correctness validation, see `tests/kuhn.rs`. Kuhn Poker is the
//! standard CFR correctness fixture: its Nash equilibrium is known
//! analytically, so we can verify the math converges to the right place.
//!
//! See `docs/ALGORITHMS.md` for background.

use std::collections::HashMap;

use crate::convergence::exploitability_two_player_zero_sum;
use crate::game::{Game, InfoSetId, Player};
use crate::matching::regret_match;

/// Per-info-set bookkeeping used by CFR+.
///
/// Info sets are lazily created the first time the tree walk reaches
/// them. This keeps the solver honest about which states are actually
/// reachable under the game rules, and it lets the same `CfrPlus` be
/// reused across games of different shapes.
#[derive(Debug, Clone)]
struct InfoSetEntry {
    /// Cumulative positive regret per action (CFR+ clamps `>= 0`).
    regret_sum: Vec<f32>,
    /// Linearly-weighted cumulative strategy contribution per action.
    strategy_sum: Vec<f32>,
    /// Scratch buffer for the current iteration's regret-matched strategy.
    /// Held on the entry so we don't reallocate in the inner loop.
    current_strategy: Vec<f32>,
}

impl InfoSetEntry {
    fn with_num_actions(n: usize) -> Self {
        Self {
            regret_sum: vec![0.0; n],
            strategy_sum: vec![0.0; n],
            current_strategy: vec![0.0; n],
        }
    }
}

/// Average (near-Nash) strategy for a game, indexed by `InfoSetId`.
///
/// Each entry is a probability distribution over that info set's actions,
/// in the same order as the game's [`Game::legal_actions`] returns them.
///
/// # Normalization
///
/// Entries sum to `1.0` (or to `0.0` if the info set was never visited,
/// which only happens when an info set is unreachable under every
/// strategy — a pathological case caused by an ill-formed game).
#[derive(Debug, Clone, Default)]
pub struct Strategy {
    per_info_set: HashMap<InfoSetId, Vec<f32>>,
}

impl Strategy {
    /// Returns the strategy vector for `info_set`, or `None` if the info
    /// set was never visited during training.
    pub fn get(&self, info_set: InfoSetId) -> Option<&[f32]> {
        self.per_info_set.get(&info_set).map(|v| v.as_slice())
    }

    /// Number of info sets with recorded strategies.
    pub fn len(&self) -> usize {
        self.per_info_set.len()
    }

    /// True if the strategy has no recorded info sets.
    pub fn is_empty(&self) -> bool {
        self.per_info_set.is_empty()
    }

    /// Iterate `(InfoSetId, strategy)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (InfoSetId, &[f32])> {
        self.per_info_set.iter().map(|(k, v)| (*k, v.as_slice()))
    }

    /// Insert or replace the strategy for a given info set. Intended for
    /// internal use and for tests that want to construct a strategy by
    /// hand (e.g. to compare against an analytical Nash answer).
    pub fn insert(&mut self, info_set: InfoSetId, strategy: Vec<f32>) {
        self.per_info_set.insert(info_set, strategy);
    }
}

/// CFR+ solver, generic over any two-player zero-sum game implementing
/// [`Game`].
///
/// # Usage
///
/// ```ignore
/// let mut solver = CfrPlus::new(kuhn);
/// solver.run(1000);
/// let strategy = solver.average_strategy();
/// let exploitability = solver.exploitability();
/// ```
///
/// `iterate()` is also exposed for tests that want to inspect state after
/// each iteration.
pub struct CfrPlus<G: Game> {
    game: G,
    entries: HashMap<InfoSetId, InfoSetEntry>,
    /// 1-based iteration counter. Post-increment inside `iterate()`.
    iteration: u32,
}

impl<G: Game> CfrPlus<G> {
    /// Create a solver for `game`. No iterations run yet; info-set
    /// bookkeeping is allocated lazily on first visit.
    pub fn new(game: G) -> Self {
        Self {
            game,
            entries: HashMap::new(),
            iteration: 0,
        }
    }

    /// Borrow the underlying game. Tests use this to query structure
    /// (e.g., to enumerate info sets for an analytical comparison).
    pub fn game(&self) -> &G {
        &self.game
    }

    /// Number of CFR+ iterations completed.
    pub fn iterations(&self) -> u32 {
        self.iteration
    }

    /// Number of info sets seen at least once.
    pub fn num_info_sets(&self) -> usize {
        self.entries.len()
    }

    /// Run a single CFR+ iteration: one full tree walk from each
    /// player's perspective, with regrets updated for the traversing
    /// player.
    ///
    /// This implementation does the classic "walk twice per iteration"
    /// formulation: once with Hero as the update-target and once with
    /// Villain. Each walk carries reach probabilities for both players
    /// and returns the updating player's counterfactual utility at each
    /// node. This matches the textbook CFR presentation (Zinkevich et
    /// al. 2008) and keeps the code straightforward; the sampling
    /// variants in `mccfr.rs` can diverge from this structure when they
    /// need to.
    pub fn iterate(&mut self) {
        self.iteration = self.iteration.saturating_add(1);

        // Two traversals per iteration — one per updating player.
        let root = self.game.initial_state();
        for &update_player in &[Player::Hero, Player::Villain] {
            self.walk(&root, update_player, 1.0, 1.0);
        }
    }

    /// Run `iterations` iterations of CFR+.
    pub fn run(&mut self, iterations: u32) {
        for _ in 0..iterations {
            self.iterate();
        }
    }

    /// CFR tree walk.
    ///
    /// * `update_player` — the player whose regrets and strategy_sum we
    ///   update on this walk.
    /// * `reach_hero`, `reach_villain` — product of action probabilities
    ///   that led here, for each player.
    ///
    /// Returns the utility of this subtree for `update_player`, weighted
    /// by the reach probability of the *other* player (i.e., the
    /// "counterfactual" value — the value `update_player` gets at this
    /// node, with the opponent reaching it per the current strategy but
    /// with `update_player` reaching it with probability 1). This is the
    /// form the regret update wants.
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
            "CfrPlus::walk: non-terminal node with zero legal actions"
        );

        let info_set_id = self.game.info_set(state, current);

        // Pull out the current strategy from the regret-matched entry.
        // We copy into a local buffer to drop the &mut self borrow before
        // recursing into child nodes.
        let strategy = {
            let entry = self
                .entries
                .entry(info_set_id)
                .or_insert_with(|| InfoSetEntry::with_num_actions(num_actions));
            debug_assert_eq!(
                entry.regret_sum.len(),
                num_actions,
                "CfrPlus::walk: action-set size changed for an info set \
                 (Game impls must return consistent action counts per info set)"
            );
            regret_match(&entry.regret_sum, &mut entry.current_strategy);
            entry.current_strategy.clone()
        };

        // Recurse over each action, collecting child utilities for the
        // *update* player.
        let mut action_utils = vec![0.0f32; num_actions];
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
        // player we're updating on this walk.
        if current == update_player {
            // Counterfactual reach: product of reach probabilities for
            // players OTHER than `update_player`. In 2p, that's just the
            // opponent's reach.
            let cf_reach = match update_player {
                Player::Hero => reach_villain,
                Player::Villain => reach_hero,
            };
            let own_reach = match update_player {
                Player::Hero => reach_hero,
                Player::Villain => reach_villain,
            };

            // Linear averaging weight: iteration `t` contributes with
            // weight `t`. See Tammelin 2014.
            let linear_weight = self.iteration as f32;

            let entry = self
                .entries
                .get_mut(&info_set_id)
                .expect("info set was inserted above");
            for i in 0..num_actions {
                let regret = action_utils[i] - node_util;
                // CFR+ clamp: regret_sum must stay >= 0.
                let updated = entry.regret_sum[i] + cf_reach * regret;
                entry.regret_sum[i] = if updated > 0.0 { updated } else { 0.0 };
                // Linear strategy averaging (the "+" in CFR+).
                entry.strategy_sum[i] += linear_weight * own_reach * strategy[i];
            }
        }

        node_util
    }

    /// Produce the average (near-Nash) strategy across all iterations.
    ///
    /// For each info set, returns the normalized `strategy_sum`. An info
    /// set with `strategy_sum == 0` (never reached by the updating
    /// player) falls back to a uniform distribution, since *something*
    /// needs to be returned and a malformed impl would otherwise make
    /// downstream best-response computations panic on `0/0`.
    pub fn average_strategy(&self) -> Strategy {
        let mut out = Strategy::default();
        for (&id, entry) in self.entries.iter() {
            let sum: f32 = entry.strategy_sum.iter().sum();
            let n = entry.strategy_sum.len();
            let v = if sum > 0.0 {
                let inv = 1.0 / sum;
                entry.strategy_sum.iter().map(|x| x * inv).collect()
            } else {
                let u = 1.0 / (n as f32);
                vec![u; n]
            };
            out.insert(id, v);
        }
        out
    }

    /// Exploitability of the current average strategy, in the game's
    /// native utility units.
    ///
    /// Delegates to [`crate::convergence::exploitability_two_player_zero_sum`].
    pub fn exploitability(&self) -> f32 {
        let strategy = self.average_strategy();
        exploitability_two_player_zero_sum(&self.game, &strategy)
    }
}
