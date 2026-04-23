//! Monte Carlo CFR (External Sampling).
//!
//! Sampling variant of CFR for trees too big to enumerate. We use
//! External Sampling: enumerate hero's actions (to get regrets for all
//! of them), sample villain's actions and chance outcomes.
//!
//! Used on turn subgames where the full enumerative tree would blow up.
//!
//! See `docs/ALGORITHMS.md` for background.

// TODO (Day 4, agent A2): implement external-sampling MCCFR.
// Seed the RNG from a parameter, not time — determinism is required
// for validation tests. Default seed = 0.
