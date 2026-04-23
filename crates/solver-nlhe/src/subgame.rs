//! `NlheSubgame` — implements `solver_core::Game` for NLHE.
//!
//! This is the bridge between the game-agnostic CFR algorithm and the
//! specific rules of No-Limit Hold'em. Given a hand state (board, ranges,
//! pot, stacks, bet tree, to-act player), it exposes a `Game` impl the
//! solver can iterate over.

use solver_core::Game;

/// An NLHE subgame starting from a given hand state.
// TODO (Day 2, agent A_main): build the full game-tree model.
pub struct NlheSubgame {
    // TODO: board, ranges, pot, stacks, bet_tree, to_act, action_history
}

/// Node state in the subgame. Cheap to clone — this is the frontier state
/// CFR traverses.
// TODO
#[derive(Clone)]
pub struct SubgameState {
    // TODO: street, pot, stacks, action history, cards dealt
}

impl Game for NlheSubgame {
    type State = SubgameState;
    type Action = crate::Action;

    fn initial_state(&self) -> Self::State {
        todo!()
    }

    fn is_terminal(&self, _state: &Self::State) -> bool {
        todo!()
    }

    fn utility(&self, _state: &Self::State, _player: solver_core::Player) -> f32 {
        todo!()
    }

    fn current_player(&self, _state: &Self::State) -> solver_core::Player {
        todo!()
    }

    fn legal_actions(&self, _state: &Self::State) -> Vec<Self::Action> {
        todo!()
    }

    fn apply(&self, _state: &Self::State, _action: &Self::Action) -> Self::State {
        todo!()
    }

    fn info_set(&self, _state: &Self::State, _player: solver_core::Player) -> solver_core::InfoSetId {
        todo!()
    }
}
