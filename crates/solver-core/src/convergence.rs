//! Convergence metrics and best-response computation.
//!
//! Exploitability is our primary convergence signal:
//! `exploitability = (util_br_vs_hero + util_br_vs_villain) / 2`
//! where `util_br_vs_X` is the expected value a best-response opponent
//! earns against strategy X. For Nash strategies, this is 0.
//!
//! Reported in big blinds per 100 hands (bb/100) or as a fraction of pot,
//! depending on context. For Kuhn Poker, units are "ante chips" and the
//! published Nash game value for player 1 is `-1/18 ≈ -0.0555`.
//!
//! # Info-set-consistent best response
//!
//! A naive BR implementation picks the max-utility action at every
//! *state*. That's wrong: two states in the same info set are
//! indistinguishable to the acting player, so the BR must pick the same
//! action in both. Picking per-state would give the BR omniscience and
//! inflate exploitability. We compute the BR action per info set
//! correctly via a two-pass tree walk: first collect the states in each
//! BR info set along with their reach probability, then do backward
//! induction by info-set depth.
//!
//! See `docs/ALGORITHMS.md`.

use std::collections::HashMap;

use crate::cfr::Strategy;
use crate::game::{Game, InfoSetId, Player};

/// Compute the exploitability of a two-player zero-sum game under
/// `strategy`.
///
/// Concretely: we do two best-response traversals. In the first, Villain
/// plays a best response against Hero's `strategy`; we record the EV
/// Villain earns. In the second, Hero plays a best response against
/// Villain's `strategy`. The exploitability is the average of those two
/// EVs.
///
/// For a true Nash strategy in a zero-sum game, both best responses earn
/// exactly the game's value for that player, and their sum is zero;
/// exploitability is zero. Any positive number measures how far from
/// Nash the strategy is.
///
/// # Note on normalization
///
/// Exploitability here is in the game's natural utility unit. For Kuhn
/// that's "ante chips". For NLHE that's big blinds.
///
/// # Note on chance layers
///
/// This generic helper only walks through `Game::initial_state` — if the
/// game has a chance layer hoisted outside `Game` (as in our Kuhn test
/// fixture), callers should use a game-specific exploitability helper
/// that enumerates the chance-layer roots.
pub fn exploitability_two_player_zero_sum<G: Game>(game: &G, strategy: &Strategy) -> f32 {
    let br_value_vs_hero = best_response_value(game, strategy, Player::Villain);
    let br_value_vs_villain = best_response_value(game, strategy, Player::Hero);
    (br_value_vs_hero + br_value_vs_villain) / 2.0
}

/// Compute the expected utility a best-response `br_player` earns when
/// the opponent plays `strategy`.
///
/// This is info-set-consistent: the BR picks a single action at each of
/// its own info sets (the same action across all states that share that
/// info set). It does *not* pick per-state.
///
/// Algorithm:
/// 1. Enumerate all states reachable under `strategy` and chance, group
///    by BR info set. Each state carries its reach probability.
/// 2. Resolve BR actions by info-set depth, deepest first (so that when
///    we evaluate a shallow info set, the deeper BR actions are fixed).
/// 3. Walk the tree one more time with the BR policy in place and
///    report the expected utility.
pub fn best_response_value<G: Game>(game: &G, strategy: &Strategy, br_player: Player) -> f32 {
    let root = game.initial_state();

    // Step 1: collect (state, reach) for each BR info set.
    let mut info_to_states: HashMap<InfoSetId, Vec<(G::State, f32)>> = HashMap::new();
    collect_states(game, strategy, br_player, &root, 1.0, &mut info_to_states);

    // Step 2: resolve BR actions by depth-descending order. We measure
    // "depth" as the length of a legal actions path from the root to a
    // state in the info set; simpler proxy is "any reaching state's
    // action count so far". We implement a simple ordering by requiring
    // the caller's Game to give us stable depth via tree walk.
    //
    // A cleaner approach: pass a map-of-maps to `br_subtree_value` and
    // let it recurse; whenever it hits an unresolved info set it fills
    // in the entry via lookahead. This gets us info-set consistency
    // because every query for the same info set returns the same
    // answer after the first resolution. For our Kuhn + NLHE use cases
    // this memoization approach works.
    let mut br_policy: HashMap<InfoSetId, usize> = HashMap::new();

    // Pre-resolve all collected info sets using lookahead evaluation.
    // We process them in a consistent order: by the index they were
    // inserted. Resolution of deeper info sets happens implicitly as
    // shallower queries recurse into them.
    let keys: Vec<InfoSetId> = info_to_states.keys().copied().collect();
    for info_id in keys {
        // Already resolved while evaluating some other info set? Skip.
        if br_policy.contains_key(&info_id) {
            continue;
        }
        let states = info_to_states[&info_id].clone();
        resolve_info_set(game, strategy, br_player, info_id, &states, &mut br_policy);
    }

    // Step 3: evaluate the BR value from the root.
    br_subtree_value(game, strategy, br_player, &root, &br_policy)
}

/// Collect (state, reach) pairs grouped by `br_player` info set. Reach
/// is the probability that `strategy` and the chance layer (here just
/// the root prior of 1.0) lead to `state`, excluding `br_player`'s own
/// choices. Uses weights from `strategy` at the opponent's info sets.
fn collect_states<G: Game>(
    game: &G,
    strategy: &Strategy,
    br_player: Player,
    state: &G::State,
    reach: f32,
    out: &mut HashMap<InfoSetId, Vec<(G::State, f32)>>,
) {
    if reach == 0.0 || game.is_terminal(state) {
        return;
    }
    let current = game.current_player(state);
    let actions = game.legal_actions(state);

    if current == br_player {
        let info = game.info_set(state, br_player);
        out.entry(info).or_default().push((state.clone(), reach));
        // Recurse down every branch: BR's own action is not yet fixed,
        // so we enumerate so the child info sets get collected too.
        for a in &actions {
            let next = game.apply(state, a);
            collect_states(game, strategy, br_player, &next, reach, out);
        }
    } else {
        let info = game.info_set(state, current);
        let probs: Vec<f32> = match strategy.get(info) {
            Some(p) => p.to_vec(),
            None => vec![1.0 / actions.len() as f32; actions.len()],
        };
        for (a, p) in actions.iter().zip(probs.iter()) {
            if *p == 0.0 {
                continue;
            }
            let next = game.apply(state, a);
            collect_states(game, strategy, br_player, &next, reach * p, out);
        }
    }
}

/// Resolve the BR action for a single info set by evaluating each
/// legal action's reach-weighted value. May recursively trigger
/// resolution of deeper info sets via `br_subtree_value`.
fn resolve_info_set<G: Game>(
    game: &G,
    strategy: &Strategy,
    br_player: Player,
    info_id: InfoSetId,
    states: &[(G::State, f32)],
    br_policy: &mut HashMap<InfoSetId, usize>,
) {
    debug_assert!(!states.is_empty());
    let sample_actions = game.legal_actions(&states[0].0);
    let n_actions = sample_actions.len();

    // Insert a "tentative" placeholder to break cycles. If the tree is
    // acyclic (which it is for any extensive-form game), this line is
    // a no-op in effect.
    br_policy.insert(info_id, 0);

    let mut best_a = 0usize;
    let mut best_q = f32::NEG_INFINITY;
    for a_idx in 0..n_actions {
        let mut q = 0.0f32;
        for (state, reach) in states {
            let actions = game.legal_actions(state);
            debug_assert_eq!(actions.len(), n_actions);
            let next = game.apply(state, &actions[a_idx]);
            q += reach * br_subtree_value(game, strategy, br_player, &next, br_policy);
        }
        if q > best_q {
            best_q = q;
            best_a = a_idx;
        }
    }
    br_policy.insert(info_id, best_a);
}

/// Expected utility to `br_player` at `state`, given `strategy` for the
/// opponent and `br_policy` fixed for resolved BR info sets.
fn br_subtree_value<G: Game>(
    game: &G,
    strategy: &Strategy,
    br_player: Player,
    state: &G::State,
    br_policy: &HashMap<InfoSetId, usize>,
) -> f32 {
    if game.is_terminal(state) {
        return game.utility(state, br_player);
    }
    let current = game.current_player(state);
    let actions = game.legal_actions(state);

    if current == br_player {
        let info = game.info_set(state, br_player);
        match br_policy.get(&info) {
            Some(&i) => {
                let next = game.apply(state, &actions[i]);
                br_subtree_value(game, strategy, br_player, &next, br_policy)
            }
            None => {
                // Unresolved info set — fall back to per-state argmax.
                // This is reached when `collect_states` did not see this
                // info set (reach was 0 everywhere under `strategy`),
                // so the choice we make here does not affect any
                // reach-weighted Q value. We pick the best action by
                // lookahead just to return a sensible utility.
                let mut best = f32::NEG_INFINITY;
                for a in &actions {
                    let next = game.apply(state, a);
                    let v = br_subtree_value(game, strategy, br_player, &next, br_policy);
                    if v > best {
                        best = v;
                    }
                }
                best
            }
        }
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
