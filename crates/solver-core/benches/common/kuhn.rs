//! Kuhn Poker Game impl — shared between multiple bench files.
//!
//! This is a bench-only copy of the Game impl from
//! `crates/solver-core/tests/kuhn.rs`. We can't depend on the test file
//! directly because cargo compiles each bench as a separate binary with
//! no access to the `tests/` tree, and we don't want to expose Kuhn as
//! public API of `solver-core` (it'd leak a test fixture into the crate's
//! surface).
//!
//! Keep this in sync with `tests/kuhn.rs` if semantics change. The only
//! thing that matters for benches is determinism — all six card deals are
//! enumerated with uniform 1/6 prior and CFR+ is driven via
//! `iterate_from`.
//!
//! Included via `#[path = "common/kuhn.rs"] mod kuhn;` inside each bench.

#![allow(dead_code)] // each including bench may only use a subset

use solver_core::{Game, InfoSetId, Player};

pub const JACK: u8 = 0;
pub const QUEEN: u8 = 1;
pub const KING: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Move {
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
pub struct KuhnState {
    pub hero_card: u8,
    pub villain_card: u8,
    pub history: Vec<Move>,
}

#[derive(Clone)]
pub struct KuhnPoker;

impl KuhnPoker {
    pub fn deals() -> &'static [(u8, u8); 6] {
        &[
            (JACK, QUEEN),
            (JACK, KING),
            (QUEEN, JACK),
            (QUEEN, KING),
            (KING, JACK),
            (KING, QUEEN),
        ]
    }

    pub fn chance_roots() -> Vec<(KuhnState, f32)> {
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
