//! Equivalence test: `CfrPlusFlat` and `CfrPlus` must converge to the
//! same strategy on the same game.
//!
//! This is the load-bearing test that proves the flat-array layout is a
//! pure performance refactor and not a behavior change. Both solvers run
//! on Kuhn Poker for 10k iterations from the same chance roots; for every
//! info set reachable by both, the strategy vectors must match within a
//! tight tolerance.
//!
//! Kuhn is small enough that 10k iterations is milliseconds but large
//! enough that arithmetic ordering artifacts show up. The tolerance is
//! `1e-6`: if the two paths ever drift further than that it signals a
//! real divergence (different regret update order, different averaging
//! weight, an off-by-one on the iteration counter, etc).
//!
//! The Kuhn `Game` impl is copied wholesale from `tests/kuhn.rs`. We
//! duplicate rather than factor out because `tests/kuhn.rs` is a sibling
//! test file, not a public module of the crate, and the alternative
//! (exposing Kuhn as public API of `solver-core`) would leak a test
//! fixture into the crate surface.

use solver_core::{enumerate_info_sets_from_roots, CfrPlus, CfrPlusFlat, Game, InfoSetId, Player};

// ---- Kuhn Poker game implementation ------------------------------------
// This is a straight copy of the relevant bits of `tests/kuhn.rs`. Keep in
// sync if Kuhn semantics ever change. (They won't — Kuhn's rules are
// fixed.)

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

#[derive(Clone)]
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
            .expect("utility called on non-terminal state");
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
            _ => panic!("legal_actions on terminal: {:?}", h),
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

// ---- Tests -------------------------------------------------------------

/// Both solvers must converge to the same strategy on Kuhn Poker.
///
/// Tolerance is 1e-6 per entry. Anything looser would hide real
/// arithmetic-ordering divergences. Anything tighter is unrealistic for
/// f32 after 10k iterations of accumulation.
#[test]
fn cfr_plus_flat_matches_hashmap_on_kuhn_10k_iters() {
    const ITERS: u32 = 10_000;
    const TOL: f32 = 1e-6;

    let roots = KuhnPoker::chance_roots();

    // HashMap path.
    let mut hash_solver = CfrPlus::new(KuhnPoker);
    hash_solver.run_from(&roots, ITERS);
    let hash_strategy = hash_solver.average_strategy();

    // Flat path.
    let mut flat_solver = CfrPlusFlat::from_roots(KuhnPoker, &roots);
    flat_solver.run_from(&roots, ITERS);
    let flat_strategy = flat_solver.average_strategy();

    // Every info set seen by the HashMap path must also exist in the flat
    // path, and the strategies must match entrywise.
    for (id, hash_s) in hash_strategy.iter() {
        let flat_s = flat_strategy.get(id).unwrap_or_else(|| {
            panic!("flat strategy missing InfoSetId({:#x})", id.0);
        });
        assert_eq!(
            hash_s.len(),
            flat_s.len(),
            "strategy length mismatch for InfoSetId({:#x}): hash={} flat={}",
            id.0,
            hash_s.len(),
            flat_s.len()
        );
        for (i, (&h, &f)) in hash_s.iter().zip(flat_s.iter()).enumerate() {
            let diff = (h - f).abs();
            assert!(
                diff < TOL,
                "InfoSetId({:#x}) action {}: hash={} flat={} diff={} > {}",
                id.0,
                i,
                h,
                f,
                diff,
                TOL
            );
        }
    }

    // And every info set the flat path enumerated should have a HashMap
    // counterpart. (The flat path enumerates statically; the HashMap path
    // lazily inserts on first visit. Under the chance-root driver they
    // should match, because every info set is reachable.)
    for (id, flat_s) in flat_strategy.iter() {
        let hash_s = hash_strategy.get(id).unwrap_or_else(|| {
            panic!("hash strategy missing InfoSetId({:#x})", id.0);
        });
        assert_eq!(hash_s.len(), flat_s.len());
    }
}

/// Info-set enumeration should find exactly the info sets the HashMap
/// solver ends up creating, no more and no less. This is a structural
/// check that pre-pays for the equivalence test.
#[test]
fn enumerate_info_sets_matches_hashmap_discovery() {
    let roots = KuhnPoker::chance_roots();

    // Drive the HashMap solver for one iteration so it materializes every
    // info set it can reach.
    let mut hash_solver = CfrPlus::new(KuhnPoker);
    hash_solver.run_from(&roots, 1);
    let expected_count = hash_solver.num_info_sets();

    let descriptors = enumerate_info_sets_from_roots(&KuhnPoker, &roots);
    assert_eq!(
        descriptors.len(),
        expected_count,
        "enumerate_info_sets_from_roots returned {} info sets, HashMap \
         solver found {}",
        descriptors.len(),
        expected_count
    );

    // All descriptors must have a non-zero action count.
    for d in &descriptors {
        assert!(
            d.num_actions > 0,
            "descriptor {:?} has zero actions",
            d.info_set_id
        );
    }

    // Max action count for Kuhn is 2. Sanity check.
    let max_a = descriptors.iter().map(|d| d.num_actions).max().unwrap();
    assert_eq!(max_a, 2, "expected Kuhn max_actions=2, got {}", max_a);
}

/// Exploitability must converge with the flat path too — if we only
/// checked strategies, a bug that cancels on paper but moves exploitability
/// would slip through.
#[test]
fn flat_converges_on_kuhn() {
    let roots = KuhnPoker::chance_roots();
    let mut solver = CfrPlusFlat::from_roots(KuhnPoker, &roots);
    solver.run_from(&roots, 1000);

    let strategy = solver.average_strategy();
    // We don't have a Kuhn-aware chance-layer exploitability helper in
    // this test file (tests/kuhn.rs has one but it's not public). Use
    // the generic per-game helper; for Kuhn the "game value" it's
    // implicitly using is wrong because the chance layer is hoisted, so
    // we check just that the hero EV lands near -1/18 using the strategy
    // helper from the enumeration.
    let value = expected_value(&strategy, Player::Hero);
    let expected = -1.0f32 / 18.0;
    assert!(
        (value - expected).abs() < 0.01,
        "Hero EV after 1000 flat iters = {value}, expected ≈ {expected}"
    );
}

// Copied from tests/kuhn.rs — chance-aware EV under `strategy`.
fn expected_value(strategy: &solver_core::Strategy, player: Player) -> f32 {
    let game = KuhnPoker;
    let mut total = 0.0f32;
    for (root, w) in KuhnPoker::chance_roots() {
        total += w * expected_utility(&game, strategy, &root, player);
    }
    total
}

fn expected_utility(
    game: &KuhnPoker,
    strategy: &solver_core::Strategy,
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
