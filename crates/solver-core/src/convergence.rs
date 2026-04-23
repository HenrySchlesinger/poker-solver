//! Convergence metrics and best-response computation.
//!
//! Exploitability is our primary convergence signal:
//! `exploitability = (util_br_vs_hero + util_br_vs_villain) / 2`
//! where `util_br_vs_X` is the expected value a best-response opponent
//! earns against strategy X. For Nash strategies, this is 0.
//!
//! Reported in big blinds per 100 hands (bb/100) or as a fraction of pot,
//! depending on context. For Kuhn Poker, units are "ante chips" (1/pot-ish)
//! and published Nash values for player 1 are around `-1/18 ≈ -0.0555`
//! (a slight disadvantage for the first player).
//!
//! See `docs/ALGORITHMS.md`.

use crate::cfr::Strategy;
use crate::game::{Game, Player};

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
/// exactly the game's value for that player, and the exploitability is
/// zero. Any positive number measures how far from Nash the strategy is.
///
/// # Note on normalization
///
/// Exploitability here is in the game's natural utility unit. For Kuhn
/// that's "ante chips". For NLHE that's big blinds (or whatever unit
/// `Game::utility` returns).
pub fn exploitability_two_player_zero_sum<G: Game>(game: &G, strategy: &Strategy) -> f32 {
    let br_value_vs_hero = best_response_value(game, strategy, Player::Villain);
    let br_value_vs_villain = best_response_value(game, strategy, Player::Hero);
    // Zero-sum: the BR value for each opponent measures exactly how much
    // they gain vs. Nash. Average is the standard exploitability metric.
    (br_value_vs_hero + br_value_vs_villain) / 2.0
}

/// Compute the expected utility a best-response `br_player` earns when
/// the opponent plays `strategy`.
///
/// Traversal: at each decision node, if the acting player is the
/// opponent, they play the `strategy` distribution over actions. If the
/// acting player is the `br_player`, they pick the action that maximizes
/// their expected utility — i.e., the best response. Terminal nodes
/// report utility via `Game::utility`.
///
/// In the zero-sum setting, this is the classic BR traversal. It runs in
/// a single pass over the game tree (`O(|tree|)`).
pub fn best_response_value<G: Game>(game: &G, strategy: &Strategy, br_player: Player) -> f32 {
    let root = game.initial_state();
    br_walk(game, strategy, br_player, &root)
}

fn br_walk<G: Game>(
    game: &G,
    strategy: &Strategy,
    br_player: Player,
    state: &G::State,
) -> f32 {
    if game.is_terminal(state) {
        return game.utility(state, br_player);
    }

    let current = game.current_player(state);
    let actions = game.legal_actions(state);
    debug_assert!(
        !actions.is_empty(),
        "best response: non-terminal node with zero actions"
    );

    if current == br_player {
        // Best response: take the max-utility action.
        let mut best = f32::NEG_INFINITY;
        for action in &actions {
            let next = game.apply(state, action);
            let v = br_walk(game, strategy, br_player, &next);
            if v > best {
                best = v;
            }
        }
        best
    } else {
        // Opponent plays `strategy` at their info set.
        let info_set = game.info_set(state, current);
        let probs = match strategy.get(info_set) {
            Some(p) => p,
            None => {
                // Info set was never visited in training — fall back to
                // uniform. A correct Game impl will have all info sets
                // visited, so this is only a safety net.
                let n = actions.len();
                let u = 1.0 / (n as f32);
                let mut total = 0.0;
                for action in &actions {
                    let next = game.apply(state, action);
                    total += u * br_walk(game, strategy, br_player, &next);
                }
                return total;
            }
        };
        assert_eq!(
            probs.len(),
            actions.len(),
            "strategy action count mismatch: info sets must report a consistent \
             number of legal actions each time they are visited"
        );
        let mut total = 0.0f32;
        for (action, &p) in actions.iter().zip(probs.iter()) {
            if p == 0.0 {
                continue;
            }
            let next = game.apply(state, action);
            total += p * br_walk(game, strategy, br_player, &next);
        }
        total
    }
}
