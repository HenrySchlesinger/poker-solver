//! Property-based tests for `solver-core`: regret matching and CFR+
//! convergence on the Kuhn Poker fixture.
//!
//! These tests encode invariants that must hold UNIVERSALLY — over any
//! valid regret vector, over any seed, over any iteration count in the
//! documented range. They complement the hand-picked examples in the
//! per-file `#[cfg(test)] mod tests`.
//!
//! # Determinism / reproducibility
//!
//! Every `proptest!` block uses a fixed `Config` that disables failure
//! persistence. Seeds are therefore pinned at proptest's default (which
//! is deterministic when no `PROPTEST_SEED` env var is set). To
//! reproduce a failure, run with the same default seed; to step a
//! different sequence, set `PROPTEST_SEED=<u64>`.
//!
//! # Catalogue of invariants encoded here
//!
//! Regret matching
//! 1. `regret_match` output sums to `1.0` (within 1e-5) for any f32
//!    vector — all-negative, all-zero, mixed positive/negative, all
//!    collapse to a valid probability distribution.
//! 2. No output element is NaN or negative.
//!
//! Kuhn CFR+
//! 3. For any seed and any iteration count in `100..2000`, the computed
//!    exploitability is non-negative.
//! 4. Running CFR+ twice on the same `KuhnPoker` produces bitwise-
//!    identical average strategies (the algorithm is deterministic — no
//!    seed influence).
//! 5. At ≥ 500 iterations, Kuhn CFR+ exploitability < 0.1 — a generous
//!    convergence sanity bound (the kuhn.rs benchmark test has a much
//!    tighter bound of 0.01, but that's for a specific 1000-iteration
//!    run; here we just need "the math is going in the right direction"
//!    across a window of iteration counts).

use std::collections::HashMap;

use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence};

use solver_core::matching::{regret_match, regret_match_vec};
use solver_core::{CfrPlus, Game, InfoSetId, Player};
// Alias: `solver_core::Strategy` (struct) would shadow
// `proptest::strategy::Strategy` (trait) pulled in by `prelude::*`. Both
// are used in this file, so rename the struct on import.
use solver_core::Strategy as SolverStrategy;

/// Deterministic config shared across tests.
fn cfg(cases: u32) -> Config {
    Config {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        source_file: Some("proptest_cfr.rs"),
        ..Config::default()
    }
}

// ==========================================================================
// Kuhn Poker fixture (duplicated from tests/kuhn.rs).
//
// This is a self-contained copy rather than a shared module because the
// two test files are independent translation units; `cargo test` builds
// each tests/*.rs as its own binary, so sharing source means threading
// a path through `#[path = "..."]` which is noisier than the duplication.
// The Kuhn game is small; cloning it is cheap.
// ==========================================================================

const JACK: u8 = 0;
const QUEEN: u8 = 1;
const KING: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Move {
    Check,
    Bet,
    Call,
    Fold,
}

impl Move {
    fn code(self) -> u32 {
        match self {
            Move::Check => 1,
            Move::Bet => 2,
            Move::Call => 3,
            Move::Fold => 4,
        }
    }
}

#[derive(Debug, Clone)]
struct KuhnState {
    hero_card: u8,
    villain_card: u8,
    history: Vec<Move>,
}

struct KuhnPoker;

impl KuhnPoker {
    fn deals() -> &'static [(u8, u8); 6] {
        &[
            (JACK, QUEEN),
            (JACK, KING),
            (QUEEN, JACK),
            (QUEEN, KING),
            (KING, JACK),
            (KING, QUEEN),
        ]
    }

    fn chance_roots() -> Vec<(KuhnState, f32)> {
        let p = 1.0 / 6.0;
        Self::deals()
            .iter()
            .map(|(h, v)| {
                (
                    KuhnState {
                        hero_card: *h,
                        villain_card: *v,
                        history: Vec::new(),
                    },
                    p,
                )
            })
            .collect()
    }

    fn terminal_utility(&self, state: &KuhnState) -> Option<f32> {
        use Move::*;
        let hero_wins = state.hero_card > state.villain_card;
        let h = &state.history;
        match h.as_slice() {
            [Check, Check] => Some(if hero_wins { 1.0 } else { -1.0 }),
            [Bet, Fold] => Some(1.0),
            [Bet, Call] => Some(if hero_wins { 2.0 } else { -2.0 }),
            [Check, Bet, Fold] => Some(-1.0),
            [Check, Bet, Call] => Some(if hero_wins { 2.0 } else { -2.0 }),
            _ => None,
        }
    }
}

impl Game for KuhnPoker {
    type State = KuhnState;
    type Action = Move;

    fn initial_state(&self) -> KuhnState {
        KuhnState {
            hero_card: JACK,
            villain_card: QUEEN,
            history: Vec::new(),
        }
    }

    fn is_terminal(&self, state: &KuhnState) -> bool {
        self.terminal_utility(state).is_some()
    }

    fn utility(&self, state: &KuhnState, player: Player) -> f32 {
        let hero_u = self
            .terminal_utility(state)
            .expect("utility on non-terminal state");
        match player {
            Player::Hero => hero_u,
            Player::Villain => -hero_u,
        }
    }

    fn current_player(&self, state: &KuhnState) -> Player {
        if state.history.len() % 2 == 0 {
            Player::Hero
        } else {
            Player::Villain
        }
    }

    fn legal_actions(&self, state: &KuhnState) -> Vec<Move> {
        use Move::*;
        let h = &state.history;
        match h.as_slice() {
            [] => vec![Check, Bet],
            [Check] => vec![Check, Bet],
            [Bet] => vec![Fold, Call],
            [Check, Bet] => vec![Fold, Call],
            _ => panic!("legal_actions on terminal"),
        }
    }

    fn apply(&self, state: &KuhnState, action: &Move) -> KuhnState {
        let mut next = state.clone();
        next.history.push(*action);
        next
    }

    fn info_set(&self, state: &KuhnState, player: Player) -> InfoSetId {
        let card = match player {
            Player::Hero => state.hero_card,
            Player::Villain => state.villain_card,
        };
        let mut key: u32 = card as u32;
        for m in &state.history {
            key = (key << 3) | m.code();
        }
        if matches!(player, Player::Villain) {
            key |= 1u32 << 31;
        }
        InfoSetId(key)
    }
}

// ==========================================================================
// Kuhn-specific exploitability helper.
//
// The tree-walking best-response implementation from tests/kuhn.rs, lifted
// verbatim so this test file is self-contained. Kuhn has a chance layer
// hoisted outside the Game trait; the generic BR helper in
// `solver_core::convergence` walks from the trait's `initial_state()`
// only, so we use a chance-layer-aware best response here.
// ==========================================================================

fn best_response_value(strategy: &SolverStrategy, br_player: Player) -> f32 {
    let game = KuhnPoker;
    let mut info_to_states: HashMap<u32, Vec<(KuhnState, f32)>> = HashMap::new();
    for (root, prior) in KuhnPoker::chance_roots() {
        collect_info_states(
            &game,
            strategy,
            br_player,
            &root,
            prior,
            &mut info_to_states,
        );
    }

    let mut br_policy: HashMap<u32, usize> = HashMap::new();

    let mut infos: Vec<(u32, usize)> = info_to_states
        .iter()
        .map(|(id, states)| {
            let depth = states
                .iter()
                .map(|(s, _)| s.history.len())
                .max()
                .unwrap_or(0);
            (*id, depth)
        })
        .collect();
    infos.sort_by_key(|&(_, d)| std::cmp::Reverse(d));

    for (info_id, _) in infos {
        let states = &info_to_states[&info_id];
        let n_actions = game.legal_actions(&states[0].0).len();
        let mut best_a = 0usize;
        let mut best_q = f32::NEG_INFINITY;
        for a_idx in 0..n_actions {
            let mut q = 0.0f32;
            for (state, reach) in states {
                let actions = game.legal_actions(state);
                let next = game.apply(state, &actions[a_idx]);
                q += reach * br_subtree_value(&game, strategy, br_player, &next, &br_policy);
            }
            if q > best_q {
                best_q = q;
                best_a = a_idx;
            }
        }
        br_policy.insert(info_id, best_a);
    }

    let mut total = 0.0f32;
    for (root, w) in KuhnPoker::chance_roots() {
        total += w * br_subtree_value(&game, strategy, br_player, &root, &br_policy);
    }
    total
}

fn collect_info_states(
    game: &KuhnPoker,
    strategy: &SolverStrategy,
    br_player: Player,
    state: &KuhnState,
    reach: f32,
    out: &mut HashMap<u32, Vec<(KuhnState, f32)>>,
) {
    if reach == 0.0 {
        return;
    }
    if game.is_terminal(state) {
        return;
    }
    let current = game.current_player(state);
    if current == br_player {
        let info = game.info_set(state, br_player).0;
        out.entry(info).or_default().push((state.clone(), reach));
        for a in game.legal_actions(state) {
            let next = game.apply(state, &a);
            collect_info_states(game, strategy, br_player, &next, reach, out);
        }
    } else {
        let info = game.info_set(state, current);
        let actions = game.legal_actions(state);
        let probs: Vec<f32> = match strategy.get(info) {
            Some(p) => p.to_vec(),
            None => vec![1.0 / actions.len() as f32; actions.len()],
        };
        for (a, p) in actions.iter().zip(probs.iter()) {
            if *p == 0.0 {
                continue;
            }
            let next = game.apply(state, a);
            collect_info_states(game, strategy, br_player, &next, reach * p, out);
        }
    }
}

fn br_subtree_value(
    game: &KuhnPoker,
    strategy: &SolverStrategy,
    br_player: Player,
    state: &KuhnState,
    br_policy: &HashMap<u32, usize>,
) -> f32 {
    if game.is_terminal(state) {
        return game.utility(state, br_player);
    }
    let current = game.current_player(state);
    let actions = game.legal_actions(state);
    if current == br_player {
        let info = game.info_set(state, br_player).0;
        let a_idx = match br_policy.get(&info) {
            Some(&i) => i,
            None => {
                let mut best = f32::NEG_INFINITY;
                for a in actions.iter() {
                    let next = game.apply(state, a);
                    let v = br_subtree_value(game, strategy, br_player, &next, br_policy);
                    if v > best {
                        best = v;
                    }
                }
                return if best.is_finite() { best } else { 0.0 };
            }
        };
        let next = game.apply(state, &actions[a_idx]);
        br_subtree_value(game, strategy, br_player, &next, br_policy)
    } else {
        let info = game.info_set(state, current);
        let probs: Vec<f32> = match strategy.get(info) {
            Some(p) => p.to_vec(),
            None => vec![1.0 / actions.len() as f32; actions.len()],
        };
        let mut total = 0.0f32;
        for (a, p) in actions.iter().zip(probs.iter()) {
            if *p == 0.0 {
                continue;
            }
            let next = game.apply(state, a);
            total += p * br_subtree_value(game, strategy, br_player, &next, br_policy);
        }
        total
    }
}

fn kuhn_exploitability(strategy: &SolverStrategy) -> f32 {
    let br_vs_hero = best_response_value(strategy, Player::Villain);
    let br_vs_villain = best_response_value(strategy, Player::Hero);
    (br_vs_hero + br_vs_villain) / 2.0
}

fn strategy_bitwise_equal(a: &SolverStrategy, b: &SolverStrategy) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for (id, va) in a.iter() {
        match b.get(id) {
            Some(vb) => {
                if va.len() != vb.len() {
                    return false;
                }
                for (x, y) in va.iter().zip(vb.iter()) {
                    // Compare bitwise because CFR+ is deterministic —
                    // running twice should produce byte-identical output.
                    if x.to_bits() != y.to_bits() {
                        return false;
                    }
                }
            }
            None => return false,
        }
    }
    true
}

// ==========================================================================
// Regret-matching properties.
// ==========================================================================

/// Arbitrary f32 regret vector: finite, in a sensible magnitude range.
///
/// We intentionally include negatives, zeros, and positives. Not NaN: the
/// `regret_match` docstring documents "NaN treated as <= 0", but the
/// solver's regret_sum is clamped to >= 0 by CFR+ so NaN should not
/// occur in production input. Generating NaN muddies what we're testing
/// here (it's already covered by the unit tests in matching.rs).
fn regret_vec(min_len: usize, max_len: usize) -> impl Strategy<Value = Vec<f32>> {
    (min_len..=max_len).prop_flat_map(|n| prop::collection::vec(-10.0f32..=10.0f32, n))
}

proptest! {
    #![proptest_config(cfg(2048))]

    /// Output sums to 1.0 within 1e-5 and every entry is in [0, 1].
    ///
    /// Mutation check: if `regret_match` ever forgot its uniform
    /// fallback (e.g., divided by zero on all-non-positive inputs), the
    /// output would contain NaN and this test would fail.
    #[test]
    fn regret_match_sums_to_one(regrets in regret_vec(1, 32)) {
        let strat = regret_match_vec(&regrets);
        let sum: f32 = strat.iter().sum();
        prop_assert!(
            (sum - 1.0).abs() < 1e-5,
            "regret_match output did not sum to 1: {sum} for {regrets:?}"
        );
        for (i, &p) in strat.iter().enumerate() {
            prop_assert!(!p.is_nan(), "NaN at index {i} for {regrets:?}");
            prop_assert!(p >= 0.0, "negative strategy entry {p} at index {i} for {regrets:?}");
            prop_assert!(p <= 1.0 + 1e-6, "strategy entry > 1: {p} at index {i} for {regrets:?}");
        }
    }

    /// regret_match with buffer output matches the Vec-returning variant
    /// exactly. Regression guard in case one form drifted from the other.
    #[test]
    fn regret_match_scalar_matches_buffer_form(regrets in regret_vec(1, 32)) {
        let mut out_buf = vec![0.0f32; regrets.len()];
        regret_match(&regrets, &mut out_buf);
        let out_vec = regret_match_vec(&regrets);
        prop_assert_eq!(out_buf, out_vec);
    }
}

// ==========================================================================
// Kuhn CFR+ properties.
// ==========================================================================

proptest! {
    // Heavy: each case runs CFR+ for up to 2000 iterations.
    #![proptest_config(cfg(8))]

    /// Exploitability is non-negative for any reasonable CFR+ output.
    ///
    /// Rationale: exploitability measures how far a strategy is from
    /// Nash. It is 0 at Nash and positive away from Nash; it can never
    /// be negative. If this test ever reports a negative number the
    /// BR/exploitability helper has a sign bug.
    ///
    /// Mutation check: if `kuhn_exploitability` ever returned `-(br_h +
    /// br_v) / 2` instead of the positive form, this test fails on the
    /// first iteration.
    #[test]
    fn kuhn_exploitability_nonneg(iters in 100u32..=2000) {
        let mut solver = CfrPlus::new(KuhnPoker);
        let roots = KuhnPoker::chance_roots();
        solver.run_from(&roots, iters);
        let strategy = solver.average_strategy();
        let exp = kuhn_exploitability(&strategy);
        prop_assert!(
            exp >= -1e-4,
            "exploitability negative at {iters} iters: {exp}"
        );
    }

    /// At >= 500 iterations the strategy must have converged to
    /// exploitability < 0.1 — a generous upper bound. The kuhn.rs test
    /// with 1000 iterations has a tighter 0.01 bound; this proptest
    /// just wants "if you give CFR+ at least 500 iterations on Kuhn,
    /// it's going in the right direction."
    ///
    /// Mutation check: if `regret_match` ever returned a uniform
    /// strategy regardless of regrets (accidental always-uniform bug),
    /// this test fails because convergence never happens.
    #[test]
    fn kuhn_convergence_sanity(iters in 500u32..=2000) {
        let mut solver = CfrPlus::new(KuhnPoker);
        let roots = KuhnPoker::chance_roots();
        solver.run_from(&roots, iters);
        let strategy = solver.average_strategy();
        let exp = kuhn_exploitability(&strategy);
        prop_assert!(
            exp < 0.1,
            "CFR+ failed to converge within {iters} iterations: exp = {exp}"
        );
    }
}

#[test]
fn kuhn_cfr_is_deterministic() {
    // Not a proptest because there's nothing to randomize — CFR+ is
    // purely deterministic given the game. But it belongs with the
    // other invariants and must pass or the "identical strategies on
    // reruns" claim is false.
    //
    // Mutation check: if CFR+ ever picked up a time- or thread-local
    // source of randomness on its hot path (say rayon changing iteration
    // order without accumulator determinism), this test would fail.
    let mut s1 = CfrPlus::new(KuhnPoker);
    let mut s2 = CfrPlus::new(KuhnPoker);
    let roots = KuhnPoker::chance_roots();
    s1.run_from(&roots, 500);
    s2.run_from(&roots, 500);
    let strategy_1 = s1.average_strategy();
    let strategy_2 = s2.average_strategy();
    assert!(
        strategy_bitwise_equal(&strategy_1, &strategy_2),
        "CFR+ is not deterministic: two runs produced different strategies"
    );
}
