//! CFR+ implementation.
//!
//! Vanilla Counterfactual Regret Minimization with the CFR+ modifications:
//! - Regret sums clamped to `>= 0` on each update (regret matching+)
//! - Strategy averaging weighted linearly by iteration number
//!
//! See `docs/ALGORITHMS.md` for background.

// TODO (Day 1, agent A_main): implement `CfrPlus` that takes a `Game` impl
// and runs iterations. Start with the simplest working version:
//
// struct CfrPlus<G: Game> { game: G, regret_tables: ..., strategy_sums: ... }
// impl<G: Game> CfrPlus<G> {
//     fn new(game: G) -> Self
//     fn iterate(&mut self)                     // one iteration
//     fn run(&mut self, iterations: u32)        // n iterations
//     fn average_strategy(&self) -> Strategy    // normalized from strategy_sum
//     fn exploitability(&self) -> f32           // best-response metric
// }
//
// Validate against Kuhn Poker in tests/ before moving on.
