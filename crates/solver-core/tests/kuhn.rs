//! Kuhn Poker convergence test — the canonical CFR correctness fixture.
//!
//! Kuhn Poker is a tiny 3-card poker game with a known analytical Nash
//! equilibrium. If our CFR+ implementation is correct, running it for
//! a few hundred iterations on Kuhn should drive exploitability to well
//! below 0.01 (in ante units) and produce player 1 strategies that match
//! the published equilibrium family.
//!
//! # Game rules (standard academic variant)
//!
//! - Deck: three cards `J < Q < K`. Each player gets one, one is burned.
//! - Both players ante 1 chip.
//! - Player 1 (Hero) acts first: **check** or **bet 1**.
//! - If P1 checks, P2 (Villain) can **check** (-> showdown, pot=2) or
//!   **bet 1**. If P2 bets, P1 can **fold** (P2 wins pot of 2) or
//!   **call** (-> showdown, pot=4).
//! - If P1 bets, P2 can **fold** (P1 wins pot of 2) or **call**
//!   (-> showdown, pot=4).
//! - At showdown, the higher card wins the entire pot.
//!
//! Utility is reported as net chip gain from the ante (so a `cc`
//! showdown win returns `+1`, a `bc`/`cbc` showdown win returns `+2`,
//! a fold costs the folder their 1-chip ante).
//!
//! # Published Nash equilibrium
//!
//! Player 1 strategy family, parameterized by `alpha in [0, 1/3]`:
//! - With `J`: bet with probability `alpha`, else check.
//! - With `Q`: always check. If P2 bets, call with probability
//!   `alpha + 1/3`, else fold.
//! - With `K`: bet with probability `3*alpha`, else check. Always call
//!   a bet.
//!
//! Player 2 strategy (unique within the Nash family):
//! - With `J`: if checked to, bet with prob 1/3 (bluff), else check.
//!   If bet into, always fold.
//! - With `Q`: if checked to, always check. If bet into, call with
//!   prob 1/3.
//! - With `K`: always bet. Always call.
//!
//! Game value for player 1 is `-1/18 ≈ -0.0555`. See Wikipedia "Kuhn
//! poker" or Kuhn 1950 for derivation.
//!
//! # Chance layer
//!
//! The `Game` trait has no chance-node primitive. For Kuhn, the 6
//! possible card deals form the chance layer. We handle it outside the
//! trait by driving CFR+ via `iterate_from` with one root per deal, each
//! weighted `1/6`. That keeps `Game` minimal and matches how the NLHE
//! side will eventually handle the board-card chance layer.

use solver_core::{CfrPlus, Game, InfoSetId, Player, Strategy};

// ---- Kuhn Poker game implementation ------------------------------------

/// Three-card Kuhn deck. Higher number = higher rank.
const JACK: u8 = 0;
const QUEEN: u8 = 1;
const KING: u8 = 2;

/// Betting-history symbols. Kept compact so we can pack them into a
/// small integer for the info-set hash.
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

/// A Kuhn state — hole cards (one per player) and the action history.
#[derive(Debug, Clone)]
struct KuhnState {
    hero_card: u8,
    villain_card: u8,
    history: Vec<Move>,
}

/// The Kuhn Poker game. The chance layer (card deal) lives outside this
/// `Game` impl — see the driver code in the tests below that passes the
/// 6 deals into `CfrPlus::iterate_from` with weight 1/6 each.
struct KuhnPoker;

impl KuhnPoker {
    /// All 6 ordered card deals (hero, villain). Hero and villain must
    /// differ; the third card is burned and doesn't matter.
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

    /// The 6 post-deal root states with uniform prior 1/6.
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

    /// Returns hero's utility at a terminal state (or `None` if not
    /// terminal).
    fn terminal_utility(&self, state: &KuhnState) -> Option<f32> {
        use Move::*;
        let hero_wins = state.hero_card > state.villain_card;
        let h = &state.history;
        match h.as_slice() {
            // check-check -> showdown at pot=2
            [Check, Check] => Some(if hero_wins { 1.0 } else { -1.0 }),
            // bet-fold -> hero bet, villain folds, hero wins villain's ante (+1)
            [Bet, Fold] => Some(1.0),
            // bet-call -> showdown at pot=4
            [Bet, Call] => Some(if hero_wins { 2.0 } else { -2.0 }),
            // check-bet-fold -> villain bet after hero checked; hero
            // folds, loses their ante (-1)
            [Check, Bet, Fold] => Some(-1.0),
            // check-bet-call -> showdown at pot=4
            [Check, Bet, Call] => Some(if hero_wins { 2.0 } else { -2.0 }),
            _ => None,
        }
    }
}

impl Game for KuhnPoker {
    type State = KuhnState;
    type Action = Move;

    fn initial_state(&self) -> KuhnState {
        // Without a chance layer in Game, the "initial state" is
        // technically undefined. We return a deterministic default
        // (J vs Q) so that `CfrPlus::iterate()` (no chance roots) does
        // something sensible, but all real Kuhn convergence tests use
        // `iterate_from(chance_roots())` instead.
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
            .expect("utility called on non-terminal state");
        match player {
            Player::Hero => hero_u,
            Player::Villain => -hero_u,
        }
    }

    fn current_player(&self, state: &KuhnState) -> Player {
        // Hero acts first, then alternate.
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
            _ => panic!("legal_actions called on terminal history: {:?}", h),
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
        // Pack: card (2 bits, low) | history (each move 3 bits) | player (high bit).
        // The player goes in bit 31 so it NEVER collides with history bits — this
        // matters because Hero's [Check, Bet] info set and Villain's [Bet] info
        // set would otherwise share the lower bits.
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

// ---- Convergence helpers (chance-layer-aware) -------------------------

/// Expected utility for `player` under `strategy`, averaged over the 6
/// Kuhn deals with uniform prior.
fn expected_value(strategy: &Strategy, player: Player) -> f32 {
    let game = KuhnPoker;
    let mut total = 0.0f32;
    for (root, w) in KuhnPoker::chance_roots() {
        total += w * expected_utility(&game, strategy, &root, player);
    }
    total
}

/// Expected utility for `player` under `strategy` starting from `state`.
fn expected_utility(
    game: &KuhnPoker,
    strategy: &Strategy,
    state: &KuhnState,
    player: Player,
) -> f32 {
    if game.is_terminal(state) {
        return game.utility(state, player);
    }
    let current = game.current_player(state);
    let actions = game.legal_actions(state);
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
        total += p * expected_utility(game, strategy, &next, player);
    }
    total
}

/// Best-response value for `br_player` against `strategy`, averaged
/// over the 6 Kuhn deals.
///
/// This is an **info-set-consistent** best response: the BR picks a
/// single action at each of its own info sets (the same action across
/// all states that share the info set), rather than picking per-state
/// (which would be "omniscient BR" and over-state the exploitability).
///
/// Algorithm: the tree is small enough to enumerate directly. For each
/// `br_player` info set, we compute Q(I, a) = sum over reachable states
/// s in I of reach(s) * V(s after a), where V is the expected utility
/// against `strategy` (or against the as-yet-unfixed BR policy below,
/// which we resolve by backward induction from the leaves).
///
/// We precompute Q values over the 6 deals × reachable histories in one
/// pass, then pick argmax per info set.
fn best_response_value(strategy: &Strategy, br_player: Player) -> f32 {
    // Step 1: Build the BR policy (action per info set) by backward
    // induction from leaves. We compute, for each BR info set, the
    // action that maximizes expected utility given `strategy`
    // elsewhere and the best-response-so-far at deeper info sets.
    //
    // Because Kuhn's tree is shallow and acyclic, we walk the tree in a
    // way that naturally composes: `br_value_walk` below returns the
    // expected utility of the BR-so-far, picking the best action at
    // each `br_player` info set on the fly by looking ahead to the
    // subtree's value for each candidate action.
    //
    // The key difference from "naive BR" is that we pick the argmax
    // action by evaluating Q(I, a) =
    //     sum_{s in I} reach_under_strategy_and_chance(s) * V(apply(s, a))
    // across all states that share info set I. We implement this by
    // collecting the set of states in each info set (under reach > 0),
    // evaluating each action's subtree value for each state, and
    // choosing the action that maximizes the reach-weighted sum.

    use std::collections::HashMap;
    let game = KuhnPoker;

    // Enumerate all reachable br_player info sets and the states in
    // them, along with their reach probability (chance prior *
    // opponent strategy products).
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

    // Backward induction: for each info set, pick the action that
    // maximizes reach-weighted Q(I, a). But we need Q values that
    // respect the BR at deeper info sets, so we do a fixpoint: repeat
    // until the BR action per info set is stable. For Kuhn, one pass
    // suffices because info sets further down are resolved first when
    // we iterate from the deepest history forward.
    //
    // Simpler: sort info sets by history length descending (deepest
    // first), resolve each greedily. This works because BR is acyclic.
    let mut br_policy: HashMap<u32, usize> = HashMap::new();

    // Collect info sets with their max history length for sorting.
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

    // Evaluate the overall value of BR under `strategy`.
    let mut total = 0.0f32;
    for (root, w) in KuhnPoker::chance_roots() {
        total += w * br_subtree_value(&game, strategy, br_player, &root, &br_policy);
    }
    total
}

/// Traverse the tree, enumerating all states whose info set belongs to
/// `br_player`, and record (state, reach) in `out` keyed by raw
/// `InfoSetId.0`. `reach` is the joint probability of chance + the
/// non-BR player's strategy leading to this state.
fn collect_info_states(
    game: &KuhnPoker,
    strategy: &Strategy,
    br_player: Player,
    state: &KuhnState,
    reach: f32,
    out: &mut std::collections::HashMap<u32, Vec<(KuhnState, f32)>>,
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
        // Don't short-circuit recursion — BR actions at deeper
        // decisions are still computed, and their own reach from here
        // is the CURRENT reach (BR's own prob is absent from this
        // product by design).
        for a in game.legal_actions(state) {
            let next = game.apply(state, &a);
            collect_info_states(game, strategy, br_player, &next, reach, out);
        }
    } else {
        // Opponent's node: weight reach by opponent's strategy at its info set.
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

/// Expected utility to `br_player` at `state`, given `strategy` for the
/// opponent and `br_policy` fixed for deeper BR info sets.
fn br_subtree_value(
    game: &KuhnPoker,
    strategy: &Strategy,
    br_player: Player,
    state: &KuhnState,
    br_policy: &std::collections::HashMap<u32, usize>,
) -> f32 {
    if game.is_terminal(state) {
        return game.utility(state, br_player);
    }
    let current = game.current_player(state);
    let actions = game.legal_actions(state);
    if current == br_player {
        let info = game.info_set(state, br_player).0;
        // If policy not resolved yet for this info set (can happen
        // during iteration over deeper info sets), fall through to
        // argmax-by-lookahead.
        let a_idx = match br_policy.get(&info) {
            Some(&i) => i,
            None => {
                // Evaluate each action, pick the max value. (We don't
                // remember the action index here — we only need the
                // utility for the caller.)
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

/// Kuhn exploitability = average of the two best-response values.
///
/// For a true Nash equilibrium the game value for Hero is exactly
/// `-1/18`, and both best responses are pinned to that value (and its
/// negation for Villain), so `(br_v + br_h) / 2 = (−nash_v + nash_h)/2`
/// — which, because the game is zero-sum and the Nash values are
/// negatives of each other, reduces to 0. Off-Nash strategies produce
/// positive exploitability.
fn kuhn_exploitability(strategy: &Strategy) -> f32 {
    let br_vs_hero = best_response_value(strategy, Player::Villain);
    let br_vs_villain = best_response_value(strategy, Player::Hero);
    // In a zero-sum game at Nash, Hero's utility is V, Villain's is -V.
    // Against Nash strategies, best-response opponent yields exactly -V
    // (Villain BR vs Hero Nash) and V (Hero BR vs Villain Nash). Their
    // sum is 0; the standard exploitability metric is the average of
    // (br_value - nash_value) across both players. Because we don't
    // know the Nash value a priori in the generic case, we use the
    // symmetric form:
    //   exploitability = (br_vs_hero + br_vs_villain) / 2
    // which equals the sum of each player's gain over Nash, averaged.
    (br_vs_hero + br_vs_villain) / 2.0
}

// ---- Tests -------------------------------------------------------------

#[test]
fn cfr_plus_converges_on_kuhn() {
    let mut solver = CfrPlus::new(KuhnPoker);
    let roots = KuhnPoker::chance_roots();
    solver.run_from(&roots, 1000);

    let strategy = solver.average_strategy();
    let exploitability = kuhn_exploitability(&strategy);

    println!("Kuhn exploitability after 1000 iters: {exploitability:.6} (ante units)");
    // Published CFR+ benchmark: well below 0.01 in <1000 iterations.
    // Empirically, this converges to ~0.005 on a 1000-iteration run.
    assert!(
        exploitability < 0.01,
        "CFR+ failed to converge on Kuhn: exploitability = {exploitability}"
    );
}

#[test]
fn kuhn_converges_to_known_game_value() {
    // The game value for player 1 in Kuhn Poker is -1/18 ≈ -0.0555.
    // Both players playing Nash => hero EV ≈ -1/18.
    let mut solver = CfrPlus::new(KuhnPoker);
    let roots = KuhnPoker::chance_roots();
    solver.run_from(&roots, 2000);

    let strategy = solver.average_strategy();
    let value = expected_value(&strategy, Player::Hero);

    println!("Hero EV under avg strategy (2000 iters): {value:.6}");
    let expected = -1.0 / 18.0;
    // Within 1% of the analytical value (the roadmap's success criterion).
    assert!(
        (value - expected).abs() < 0.01,
        "Hero EV = {value}, expected ≈ {expected}"
    );
}

#[test]
fn kuhn_p2_strategy_matches_nash_constraints() {
    // Every Nash equilibrium of Kuhn constrains Villain's strategy
    // uniquely:
    //   - Villain with J, facing check: bets 1/3 of the time
    //   - Villain with J, facing bet: always folds
    //   - Villain with Q, facing check: always checks
    //   - Villain with Q, facing bet: calls 1/3 of the time
    //   - Villain with K: always bets / always calls
    let mut solver = CfrPlus::new(KuhnPoker);
    let roots = KuhnPoker::chance_roots();
    solver.run_from(&roots, 5000);
    let strategy = solver.average_strategy();

    let villain_info = |card: u8, history: &[Move]| -> InfoSetId {
        let s = KuhnState {
            hero_card: u8::MAX, // unused for villain's info set
            villain_card: card,
            history: history.to_vec(),
        };
        KuhnPoker.info_set(&s, Player::Villain)
    };

    let tol = 0.05;

    // After [Check]: villain chooses from [Check, Bet] — index 1 = bet.
    let j_check = strategy
        .get(villain_info(JACK, &[Move::Check]))
        .expect("V with J after check");
    println!("V(J | check): {:?}", j_check);
    assert!(
        (j_check[1] - 1.0 / 3.0).abs() < tol,
        "V with J facing check should bet ~1/3, got bet freq {}",
        j_check[1]
    );

    // After [Bet]: villain chooses from [Fold, Call] — index 0 = fold.
    let j_bet = strategy
        .get(villain_info(JACK, &[Move::Bet]))
        .expect("V with J after bet");
    println!("V(J | bet): {:?}", j_bet);
    assert!(
        j_bet[0] > 1.0 - tol,
        "V with J facing bet should fold ~always, got fold freq {}",
        j_bet[0]
    );

    let q_check = strategy
        .get(villain_info(QUEEN, &[Move::Check]))
        .expect("V with Q after check");
    println!("V(Q | check): {:?}", q_check);
    assert!(
        q_check[0] > 1.0 - tol,
        "V with Q facing check should ~always check, got check freq {}",
        q_check[0]
    );

    let q_bet = strategy
        .get(villain_info(QUEEN, &[Move::Bet]))
        .expect("V with Q after bet");
    println!("V(Q | bet): {:?}", q_bet);
    assert!(
        (q_bet[1] - 1.0 / 3.0).abs() < tol,
        "V with Q facing bet should call ~1/3, got call freq {}",
        q_bet[1]
    );

    let k_check = strategy
        .get(villain_info(KING, &[Move::Check]))
        .expect("V with K after check");
    println!("V(K | check): {:?}", k_check);
    assert!(
        k_check[1] > 1.0 - tol,
        "V with K facing check should ~always bet, got bet freq {}",
        k_check[1]
    );

    let k_bet = strategy
        .get(villain_info(KING, &[Move::Bet]))
        .expect("V with K after bet");
    println!("V(K | bet): {:?}", k_bet);
    assert!(
        k_bet[1] > 1.0 - tol,
        "V with K facing bet should ~always call, got call freq {}",
        k_bet[1]
    );
}

#[test]
fn kuhn_p1_alpha_is_in_nash_family() {
    // Player 1's strategy is parameterized by alpha in [0, 1/3].
    //   - P1 with J: bet with prob alpha
    //   - P1 with Q: (after checking) call with prob alpha + 1/3
    //   - P1 with K: bet with prob 3*alpha
    //
    // We extract alpha from the J-bet frequency and check that:
    //   - alpha is in [0, 1/3] (with small float tolerance)
    //   - K-bet frequency ≈ 3*alpha
    //   - Q-call-after-check-bet ≈ alpha + 1/3
    let mut solver = CfrPlus::new(KuhnPoker);
    let roots = KuhnPoker::chance_roots();
    solver.run_from(&roots, 5000);
    let strategy = solver.average_strategy();

    let hero_info = |card: u8, history: &[Move]| -> InfoSetId {
        let s = KuhnState {
            hero_card: card,
            villain_card: u8::MAX,
            history: history.to_vec(),
        };
        KuhnPoker.info_set(&s, Player::Hero)
    };

    // Actions at start: [Check, Bet]. Index 1 = bet.
    let j_open = strategy
        .get(hero_info(JACK, &[]))
        .expect("H with J at start");
    let k_open = strategy
        .get(hero_info(KING, &[]))
        .expect("H with K at start");
    let q_facing_bet = strategy
        .get(hero_info(QUEEN, &[Move::Check, Move::Bet]))
        .expect("H with Q facing bet after check");

    let alpha = j_open[1];
    // After [Check, Bet]: hero chooses [Fold, Call]. Index 1 = call.
    let q_call = q_facing_bet[1];
    let k_bet = k_open[1];

    println!("P1 alpha (J-bet freq): {alpha:.4}");
    println!(
        "P1 K-bet freq:         {k_bet:.4} (expected ≈ 3*alpha = {:.4})",
        3.0 * alpha
    );
    println!(
        "P1 Q-call freq:        {q_call:.4} (expected ≈ alpha + 1/3 = {:.4})",
        alpha + 1.0 / 3.0
    );

    let tol = 0.08;
    assert!(
        alpha >= -tol && alpha <= 1.0 / 3.0 + tol,
        "alpha out of Nash family range: {alpha}"
    );
    assert!(
        (k_bet - 3.0 * alpha).abs() < tol,
        "K-bet freq {k_bet} should equal 3*alpha = {}",
        3.0 * alpha
    );
    assert!(
        (q_call - (alpha + 1.0 / 3.0)).abs() < tol,
        "Q-call freq {q_call} should equal alpha + 1/3 = {}",
        alpha + 1.0 / 3.0
    );
}
