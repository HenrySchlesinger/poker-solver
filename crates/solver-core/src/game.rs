//! The generic game trait that all solvers operate over.
//!
//! `solver-nlhe` implements this for NLHE subgames. `tests/kuhn.rs`
//! implements it for Kuhn Poker as a correctness fixture.

/// One of the two players in a two-player zero-sum game.
///
/// v0.1 is heads-up only; multi-way is a v0.3 feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Player {
    /// The player we're computing strategy for.
    Hero = 0,
    /// The opponent.
    Villain = 1,
}

impl Player {
    /// Returns the opposite player.
    pub const fn opponent(self) -> Self {
        match self {
            Player::Hero => Player::Villain,
            Player::Villain => Player::Hero,
        }
    }
}

/// Opaque identifier for an information set.
///
/// Two game-tree states share an `InfoSetId` iff they are indistinguishable
/// from the perspective of the acting player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InfoSetId(pub u32);

/// The game-agnostic interface the solver operates against.
///
/// Implement this for Kuhn Poker (tests), NLHE (solver-nlhe), or any
/// other two-player zero-sum imperfect-information game.
pub trait Game {
    /// Opaque state type representing a node in the game tree.
    type State: Clone;

    /// Action type. Must be cheaply cloneable and comparable.
    type Action: Clone + Eq + std::hash::Hash;

    /// Returns the initial state (root of the game tree).
    fn initial_state(&self) -> Self::State;

    /// Is this state terminal (showdown or forced fold)?
    fn is_terminal(&self, state: &Self::State) -> bool;

    /// At a terminal, returns the utility for `player` (in big blinds, or
    /// the game's natural unit).
    fn utility(&self, state: &Self::State, player: Player) -> f32;

    /// At a non-terminal, returns the player to act.
    ///
    /// Panics if `state` is terminal.
    fn current_player(&self, state: &Self::State) -> Player;

    /// Legal actions at `state`.
    fn legal_actions(&self, state: &Self::State) -> Vec<Self::Action>;

    /// Apply `action` to `state`, returning the successor state.
    fn apply(&self, state: &Self::State, action: &Self::Action) -> Self::State;

    /// The info-set identifier for `state` from `player`'s perspective.
    fn info_set(&self, state: &Self::State, player: Player) -> InfoSetId;
}
