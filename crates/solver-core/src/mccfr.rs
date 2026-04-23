//! Monte Carlo CFR — External Sampling variant.
//!
//! Sampling-based CFR for games where enumerating the full tree every
//! iteration is prohibitive. The turn subgame in NLHE is the primary
//! motivating case: a vanilla CFR iteration there has to walk ~46 river
//! subgames, one per possible river card, per iteration — millions of
//! leaves. External Sampling replaces that with one sampled river per
//! iteration, dramatically cheaper per step at the cost of higher
//! per-iteration variance. Empirically turn spots converge well within
//! ~500 MCCFR iterations.
//!
//! # External Sampling — the shape
//!
//! On each iteration we pick an "update player" (Hero or Villain) and
//! walk the tree from a root. At every decision node:
//!
//! * If it is the update player's: **enumerate** all actions, recurse
//!   into each, and accumulate regrets against the weighted node value.
//!   This is the same shape as vanilla CFR — we need per-action utilities
//!   to get regrets.
//! * If it is the opponent's: **sample one action** weighted by the
//!   opponent's current regret-matched strategy, and recurse into just
//!   that branch. No regret update happens on the opponent's info set
//!   during this walk; we'll update it on the walk that has the opponent
//!   as update player.
//!
//! Chance nodes are the reason we care about MCCFR in the first place.
//! This implementation keeps the [`Game`] trait free of chance primitives
//! (matching the simplification in the docs), so chance has to happen
//! outside the trait. Two supported patterns:
//!
//! 1. **Sample the root before the iteration.** Pass a closure to
//!    [`MCCfr::iterate_with`] that draws one root state per iteration
//!    (e.g., picks one river card). The iteration walks a chance-free
//!    tree from that root. This is the pattern the NLHE turn subgame
//!    uses.
//! 2. **Hide chance in `Game::apply`.** The Game implementation carries
//!    its own RNG and returns a sampled successor state at an internal
//!    chance node. MCCFR stays ignorant. Works if the Game impl is
//!    allowed to have internal mutable state; we don't use this today
//!    because the turn subgame is cheaper to express via pattern 1.
//!
//! # CFR+ add-ons
//!
//! This sampler uses the CFR+ modifications (regret clamp to `>= 0`,
//! linear strategy averaging by iteration index). In the literature
//! "External Sampling MCCFR" sometimes refers to a CFR-based variant
//! without the `+`; we're pragmatic and use the `+` form because the
//! rest of the crate is CFR+. A cross-check test in solver-nlhe
//! confirms that on a chance-free game (a river subgame) this
//! implementation converges to the same strategies as the enumerative
//! `CfrPlus` at high iteration counts — within MCCFR sampling variance.
//!
//! # Determinism
//!
//! The PRNG is `rand_xoshiro::Xoshiro256StarStar`, seeded at construction.
//! Default seed is `0` so a "fresh" `MCCfr::new(game, 0)` is reproducible
//! byte-for-byte across runs. Tests lean on this: running the same seed
//! twice should produce bit-identical regret and strategy vectors.
//!
//! See `docs/ALGORITHMS.md`.

use std::collections::HashMap;

use rand::Rng;
use rand::SeedableRng;
use rand_xoshiro::Xoshiro256StarStar;

use crate::cfr::Strategy;
use crate::convergence::exploitability_two_player_zero_sum;
use crate::game::{Game, InfoSetId, Player};
use crate::matching::regret_match;

/// Per-info-set bookkeeping for MCCFR. Same layout as
/// `cfr::InfoSetEntry`; kept private here so the two solvers can
/// evolve independently.
#[derive(Debug, Clone)]
struct InfoSetEntry {
    /// Cumulative positive regret per action (CFR+ clamp to `>= 0`).
    regret_sum: Vec<f32>,
    /// Linearly-weighted cumulative strategy contribution per action.
    strategy_sum: Vec<f32>,
    /// Scratch buffer for the current iteration's regret-matched strategy.
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

/// External Sampling MCCFR solver.
///
/// Generic over [`Game`] just like [`crate::CfrPlus`]. Chance nodes, if
/// present, live outside the `Game` trait — pass them in at iteration
/// time via [`MCCfr::iterate_with`] / [`MCCfr::run_with`], or rely on
/// the default [`MCCfr::iterate`] which uses `Game::initial_state()`.
///
/// # Usage
///
/// ```ignore
/// // Chance-free game (e.g., a river subgame):
/// let mut solver = MCCfr::new(river_game, /*seed=*/ 0);
/// solver.run(500);
/// let strat = solver.average_strategy();
///
/// // With a chance layer (e.g., a turn subgame sampling one river card):
/// let mut solver = MCCfr::new(turn_game, /*seed=*/ 0);
/// solver.run_with(500, |rng| turn_game_sample_root(rng));
/// ```
pub struct MCCfr<G: Game> {
    game: G,
    entries: HashMap<InfoSetId, InfoSetEntry>,
    rng: Xoshiro256StarStar,
    /// 1-based iteration counter. Post-increment inside `iterate*`.
    iteration: u32,
}

impl<G: Game> MCCfr<G> {
    /// Create a solver for `game`, seeded with `seed`. No iterations run
    /// yet; info-set entries are allocated lazily on first visit.
    pub fn new(game: G, seed: u64) -> Self {
        Self {
            game,
            entries: HashMap::new(),
            rng: Xoshiro256StarStar::seed_from_u64(seed),
            iteration: 0,
        }
    }

    /// Borrow the game.
    pub fn game(&self) -> &G {
        &self.game
    }

    /// Number of iterations completed.
    pub fn iterations(&self) -> u32 {
        self.iteration
    }

    /// Number of info sets visited.
    pub fn num_info_sets(&self) -> usize {
        self.entries.len()
    }

    /// Run a single iteration starting from `Game::initial_state()`.
    ///
    /// For games without a chance layer above the root this is all you
    /// need. A chance-layered game (turn subgame in NLHE) wants
    /// [`Self::iterate_with`] instead.
    pub fn iterate(&mut self) {
        let root = self.game.initial_state();
        self.iterate_from_root(root);
    }

    /// Run a single iteration starting from a root sampled by `sample`.
    ///
    /// `sample` is invoked exactly once with the solver's PRNG, and
    /// whatever state it returns is used as the iteration root. Use this
    /// pattern to push a chance layer above the `Game` trait: the
    /// closure picks one chance outcome (turn subgame: one river card)
    /// and builds the successor state from it.
    pub fn iterate_with<F>(&mut self, mut sample: F)
    where
        F: FnMut(&mut Xoshiro256StarStar) -> G::State,
    {
        let root = sample(&mut self.rng);
        self.iterate_from_root(root);
    }

    /// Internal: bump the iteration counter and walk the tree twice
    /// (once with each player as update target), both from the same
    /// sampled root.
    fn iterate_from_root(&mut self, root: G::State) {
        self.iteration = self.iteration.saturating_add(1);

        // Walk twice per iteration: Hero-update then Villain-update.
        // Same root both times — within a single iteration, sampling
        // randomness on the opponent side is independent between the two
        // walks (pulled fresh from the RNG). This matches the standard
        // ES-MCCFR treatment: each player's regret update sees the same
        // chance outcome (the root we sampled) but independent opponent
        // samples. That's necessary because each walk updates a
        // different player's regrets.
        for &update_player in &[Player::Hero, Player::Villain] {
            self.walk(&root, update_player);
        }
    }

    /// Run `iterations` iterations from the game's deterministic root.
    ///
    /// Equivalent to calling [`Self::iterate`] `iterations` times. For
    /// chance-layered games use [`Self::run_with`].
    pub fn run(&mut self, iterations: u32) {
        for _ in 0..iterations {
            self.iterate();
        }
    }

    /// Run `iterations` iterations, sampling a fresh root each time via
    /// `sample`. See [`Self::iterate_with`].
    pub fn run_with<F>(&mut self, iterations: u32, mut sample: F)
    where
        F: FnMut(&mut Xoshiro256StarStar) -> G::State,
    {
        for _ in 0..iterations {
            let root = sample(&mut self.rng);
            self.iterate_from_root(root);
        }
    }

    /// External Sampling tree walk.
    ///
    /// Returns the utility of `state` for `update_player` under the
    /// current strategy profile, where opponent actions along this walk
    /// have been sampled (one-per-node) and chance was pushed above the
    /// root by the caller.
    ///
    /// At the update player's info sets, enumerates actions and updates
    /// regrets with counterfactual reach = 1 (external sampling
    /// normalizes that out: we sampled the opponent's reach exactly, so
    /// no weighting is needed in expectation).
    ///
    /// Transcription of the pseudocode in Lanctot et al. 2009, adapted
    /// to CFR+ (regret clamp + linear averaging).
    fn walk(&mut self, state: &G::State, update_player: Player) -> f32 {
        if self.game.is_terminal(state) {
            return self.game.utility(state, update_player);
        }

        let current = self.game.current_player(state);
        let actions = self.game.legal_actions(state);
        let num_actions = actions.len();
        assert!(
            num_actions > 0,
            "MCCfr::walk: non-terminal node with zero legal actions"
        );

        let info_set_id = self.game.info_set(state, current);

        // Pull the regret-matched strategy for this info set.
        let strategy = {
            let entry = self
                .entries
                .entry(info_set_id)
                .or_insert_with(|| InfoSetEntry::with_num_actions(num_actions));
            debug_assert_eq!(
                entry.regret_sum.len(),
                num_actions,
                "MCCfr::walk: action-set size changed for an info set \
                 (Game impls must return consistent action counts per info set)"
            );
            regret_match(&entry.regret_sum, &mut entry.current_strategy);
            entry.current_strategy.clone()
        };

        if current == update_player {
            // Enumerate branch: compute per-action utilities.
            let mut action_utils = vec![0.0f32; num_actions];
            let mut node_util = 0.0f32;

            for (i, action) in actions.iter().enumerate() {
                let next = self.game.apply(state, action);
                let u = self.walk(&next, update_player);
                action_utils[i] = u;
                node_util += strategy[i] * u;
            }

            // Regret update. External sampling has already integrated
            // the opponent's reach into the returned utilities, so the
            // counterfactual reach here is 1 and we just accumulate
            // `action_utils[i] - node_util` directly. Linear strategy
            // averaging uses the iteration index as the weight (CFR+).
            let linear_weight = self.iteration as f32;
            let entry = self
                .entries
                .get_mut(&info_set_id)
                .expect("info set inserted above");
            for i in 0..num_actions {
                let regret = action_utils[i] - node_util;
                let updated = entry.regret_sum[i] + regret;
                // CFR+ clamp.
                entry.regret_sum[i] = if updated > 0.0 { updated } else { 0.0 };
                entry.strategy_sum[i] += linear_weight * strategy[i];
            }

            node_util
        } else {
            // Opponent node: sample a single action weighted by the
            // opponent's current strategy and recurse only into that
            // branch. No regret update here — this info set will be
            // updated on the walk where `current` is the update player.
            let sampled_idx = sample_from(&strategy, &mut self.rng);
            let next = self.game.apply(state, &actions[sampled_idx]);
            self.walk(&next, update_player)
        }
    }

    /// Produce the average (near-Nash) strategy across all iterations.
    ///
    /// Normalized `strategy_sum` per info set. Info sets that were never
    /// visited under the update player fall back to uniform, matching
    /// [`crate::CfrPlus::average_strategy`].
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
    /// natural utility units.
    ///
    /// Delegates to
    /// [`crate::convergence::exploitability_two_player_zero_sum`], which
    /// walks the game tree from `Game::initial_state()` using the
    /// average strategy and computes both players' best-response value.
    ///
    /// **Caveat:** for a chance-layered MCCfr run this helper only
    /// considers the single deterministic `initial_state` root. Callers
    /// who pushed chance above the root (`iterate_with` / `run_with`)
    /// should compute exploitability themselves by averaging over the
    /// chance layer — see `solver-nlhe` turn tests for the pattern.
    pub fn exploitability(&self) -> f32 {
        let strategy = self.average_strategy();
        exploitability_two_player_zero_sum(&self.game, &strategy)
    }
}

/// Sample one index from a probability distribution using `rng`.
///
/// Returns `probs.len() - 1` as a fallback if numerical rounding leaves
/// a tiny deficit between the cumulative sum and `r` — matches the
/// "last bucket absorbs rounding" convention used in most probabilistic
/// sampling code.
///
/// Assumes `probs` is non-empty and sums to approximately 1.
fn sample_from<R: Rng + ?Sized>(probs: &[f32], rng: &mut R) -> usize {
    debug_assert!(!probs.is_empty());
    let r: f32 = rng.gen();
    let mut acc = 0.0f32;
    for (i, &p) in probs.iter().enumerate() {
        acc += p;
        if r < acc {
            return i;
        }
    }
    probs.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal matching-pennies-like game used to sanity-check MCCFR
    /// without pulling in a full NLHE subgame. Both players pick
    /// simultaneously; in an extensive-form encoding the second player
    /// doesn't see the first's move (info sets collapse over the first
    /// player's action).
    ///
    /// Payoffs (Hero's utility):
    ///   Hero:H, Villain:H → -1
    ///   Hero:H, Villain:T → +1
    ///   Hero:T, Villain:H → +1
    ///   Hero:T, Villain:T → -1
    ///
    /// Nash is (1/2, 1/2) for both players.
    struct MatchingPennies;

    #[derive(Clone)]
    struct MpState {
        /// Indexed by player: 0 = Hero, 1 = Villain. Value: 0 = H, 1 = T,
        /// 255 = not chosen.
        chosen: [u8; 2],
    }

    impl Game for MatchingPennies {
        type State = MpState;
        type Action = u8; // 0 = H, 1 = T

        fn initial_state(&self) -> MpState {
            MpState { chosen: [255, 255] }
        }

        fn is_terminal(&self, state: &MpState) -> bool {
            state.chosen[0] != 255 && state.chosen[1] != 255
        }

        fn utility(&self, state: &MpState, player: Player) -> f32 {
            let match_ = state.chosen[0] == state.chosen[1];
            let hero_u = if match_ { -1.0 } else { 1.0 };
            match player {
                Player::Hero => hero_u,
                Player::Villain => -hero_u,
            }
        }

        fn current_player(&self, state: &MpState) -> Player {
            if state.chosen[0] == 255 {
                Player::Hero
            } else {
                Player::Villain
            }
        }

        fn legal_actions(&self, _state: &MpState) -> Vec<u8> {
            vec![0, 1]
        }

        fn apply(&self, state: &MpState, action: &u8) -> MpState {
            let mut next = state.clone();
            if next.chosen[0] == 255 {
                next.chosen[0] = *action;
            } else {
                next.chosen[1] = *action;
            }
            next
        }

        fn info_set(&self, _state: &MpState, player: Player) -> InfoSetId {
            // Villain's info set must not depend on Hero's action (it's
            // a simultaneous game). So both players' info sets hash to
            // distinct-per-player constants.
            InfoSetId(match player {
                Player::Hero => 0,
                Player::Villain => 1,
            })
        }
    }

    #[test]
    fn mccfr_converges_on_matching_pennies() {
        let mut s = MCCfr::new(MatchingPennies, 0);
        s.run(5000);
        let strat = s.average_strategy();
        let hero_strat = strat.get(InfoSetId(0)).expect("hero info set");
        let vil_strat = strat.get(InfoSetId(1)).expect("villain info set");
        // Nash is (1/2, 1/2) for both. Loose tolerance for the
        // stochastic run.
        let tol = 0.10;
        assert!(
            (hero_strat[0] - 0.5).abs() < tol,
            "hero H freq: {}",
            hero_strat[0]
        );
        assert!(
            (vil_strat[0] - 0.5).abs() < tol,
            "villain H freq: {}",
            vil_strat[0]
        );
    }

    #[test]
    fn mccfr_is_deterministic_for_fixed_seed() {
        let mut a = MCCfr::new(MatchingPennies, 42);
        let mut b = MCCfr::new(MatchingPennies, 42);
        a.run(100);
        b.run(100);
        let sa = a.average_strategy();
        let sb = b.average_strategy();
        for id in 0..2u32 {
            let xa = sa.get(InfoSetId(id)).unwrap();
            let xb = sb.get(InfoSetId(id)).unwrap();
            assert_eq!(xa, xb, "determinism broke at info set {id}");
        }
    }

    #[test]
    fn mccfr_different_seeds_diverge_in_finite_iters() {
        // At low iteration counts, different seeds give different
        // strategy vectors (the averages haven't converged yet).
        let mut a = MCCfr::new(MatchingPennies, 1);
        let mut b = MCCfr::new(MatchingPennies, 2);
        a.run(20);
        b.run(20);
        let sa = a.average_strategy();
        let sb = b.average_strategy();
        let xa = sa.get(InfoSetId(0)).unwrap();
        let xb = sb.get(InfoSetId(0)).unwrap();
        assert_ne!(
            xa, xb,
            "seeds 1 and 2 should produce different early trajectories"
        );
    }

    #[test]
    fn sample_from_distribution_respects_probabilities() {
        // Uniform: roughly 50/50.
        let mut rng = Xoshiro256StarStar::seed_from_u64(7);
        let probs = [0.5, 0.5];
        let mut counts = [0u32; 2];
        for _ in 0..10_000 {
            counts[sample_from(&probs, &mut rng)] += 1;
        }
        let p0 = counts[0] as f32 / 10_000.0;
        assert!((p0 - 0.5).abs() < 0.02, "sampler biased: p0 = {p0}");

        // Skewed: 0.9 / 0.1.
        let probs = [0.9, 0.1];
        let mut counts = [0u32; 2];
        for _ in 0..10_000 {
            counts[sample_from(&probs, &mut rng)] += 1;
        }
        let p0 = counts[0] as f32 / 10_000.0;
        assert!((p0 - 0.9).abs() < 0.02, "skewed sampler off: p0 = {p0}");
    }
}
