//! Convergence metrics and best-response computation.
//!
//! Exploitability is our primary convergence signal:
//! `exploitability = (util_br_vs_hero + util_br_vs_villain) / 2`
//! where `util_br_vs_X` is the expected value a best-response opponent
//! earns against strategy X. For Nash strategies, this is 0.
//!
//! Reported in big blinds per 100 hands (bb/100) or as a fraction of pot,
//! depending on context.

// TODO (Day 2, agent A_main): implement
// pub fn exploitability<G: Game>(game: &G, strategy: &Strategy) -> f32
//
// Best-response traversal: one player plays `strategy`, the other plays
// deterministically-optimal (pick the max-utility action at every info set).
// Measure the utility gap from Nash.
