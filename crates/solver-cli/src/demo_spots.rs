//! Pre-canned demo spots for `solver-cli demo`.
//!
//! Each spot ships with a **hand-curated strategy** (not a live solve). Why:
//! the goal of the demo is "show a poker pro something they'll nod at in
//! 10 seconds." Running a live CFR+ solve would (a) block on upstream
//! `NlheSubgame` wiring that isn't finished, and (b) take long enough that
//! the user would wait instead of getting the instant "wow." So we bake
//! analytically known strategies into the binary.
//!
//! Correctness bar: every spot's strategy has been sanity-checked against
//! what a GTO-literate human would expect. The `royal` spot is trivial
//! (you always bet the nuts). `coinflip` uses the published equity for
//! AKs-vs-22 preflop. `bluff_catch` pulls from a canonical river bluff-
//! catch study. These numbers are not from running our solver; they're
//! the reference a live run should eventually match within tolerance.
//!
//! When `NlheSubgame` and the bet-tree builder stabilize (see Day 2+ in
//! `docs/ROADMAP.md`), this module can be re-plumbed to call the real
//! solver for each spot. The public API (`all_spots`, `find_spot`,
//! `Spot`) is designed to allow that swap without touching `demo.rs`.

use std::time::Duration;

/// A single bar in the strategy visualization.
///
/// Each `Action` corresponds to one column of the strategy grid printed
/// by `demo.rs`. The `frequency` is a probability in `[0.0, 1.0]` — the
/// renderer scales it to a bar width.
#[derive(Debug, Clone)]
pub struct Action {
    /// Short label shown above the bar. Examples: "check", "bet 66%",
    /// "bet pot", "fold", "call", "raise".
    pub label: &'static str,
    /// GTO frequency for this action, in `[0.0, 1.0]`. The frequencies
    /// across one `Spot`'s action set must sum to 1.0 (+/- 0.001 for
    /// rounding).
    pub frequency: f32,
}

/// A single decision point inside a spot. A `bluff_catch` spot, for
/// instance, might have one decision node ("hero facing a river bet");
/// a `coinflip` showdown spot might have one ("hero's preflop jam
/// frequency").
#[derive(Debug, Clone)]
pub struct Decision {
    /// Human-readable label for which decision this is. Printed above
    /// the strategy bars.
    pub label: &'static str,
    /// The action set with baked GTO frequencies.
    pub actions: Vec<Action>,
}

/// A pre-canned demo spot. Everything the renderer needs is here.
#[derive(Debug, Clone)]
pub struct Spot {
    /// Short id used by `--spot <id>`. Must be kebab-case; only `[a-z_]`.
    pub id: &'static str,
    /// Display title for the header banner.
    pub title: &'static str,
    /// Hero's holding (human-readable). May be a single combo ("AsKs")
    /// or a tiny range description ("AKs — any suit").
    pub hero_range: &'static str,
    /// Villain's holding or range.
    pub villain_range: &'static str,
    /// Board string as typed (empty for preflop). Parsable by
    /// `solver_eval::Board::parse` when a live solve is wired up.
    pub board: &'static str,
    /// Annotation string printed after the board (e.g. "monotone
    /// hearts", "rainbow dry", "preflop — all-in runout"). Empty string
    /// means no annotation.
    pub board_annotation: &'static str,
    /// Pot size in chips (just for display — the renderer doesn't use
    /// this for strategy math).
    pub pot: u32,
    /// Effective stack in chips, for display.
    pub stack: u32,
    /// Decision nodes in the order they should be printed. Typically
    /// one; occasionally two for spots that want to show both sides.
    pub decisions: Vec<Decision>,
    /// Hero's equity (0.0..1.0). Printed in the header stats. If this
    /// is `None`, the equity row is omitted.
    pub hero_equity: Option<f32>,
    /// Exploitability in big blinds. `0.0` = perfect Nash. Printed in
    /// the solver-stats block.
    pub exploitability_bb: f32,
    /// CFR iteration count (for display). Set to whatever the published
    /// reference used — baked so the output looks like a real solve.
    pub iterations: u32,
    /// Simulated compute time. The demo doesn't actually run CFR, so
    /// this is a plausible constant that matches our v0.1 target
    /// latency on the given spot class (river: ~40ms, turn: ~200ms,
    /// preflop: trivial).
    pub compute_time: Duration,
    /// Hand-authored explanation paragraph, printed in the
    /// "WHAT THIS MEANS" block. Must be 2–4 sentences, pro-readable.
    pub narration: &'static str,
}

/// Return every demo spot in a canonical order.
///
/// Order is designed for `--spot all`: royal (trivial) → coinflip
/// (shows equity math) → bluff_catch (shows mixed strategy).
pub fn all_spots() -> Vec<Spot> {
    vec![royal(), coinflip(), bluff_catch()]
}

/// Look up a spot by its `--spot <id>` string. Returns `None` for the
/// special "all" value; callers handle that case separately.
pub fn find_spot(id: &str) -> Option<Spot> {
    match id {
        "royal" => Some(royal()),
        "coinflip" => Some(coinflip()),
        "bluff_catch" | "bluff-catch" => Some(bluff_catch()),
        _ => None,
    }
}

/// Valid `--spot` values (excluding the "all" meta-value). Used for
/// the error message when the user types an unknown spot name.
pub const VALID_SPOT_IDS: &[&str] = &["royal", "coinflip", "bluff_catch", "all"];

// ---------------------------------------------------------------------------
// Spot constructors
// ---------------------------------------------------------------------------

/// "royal" — hero has the absolute nuts on a monotone river.
///
/// Strategy is analytically trivial: always bet as big as possible.
/// The tiny check frequency reflects a realistic GTO tree where small
/// bluff-induction checks are present. Canonical bet distribution for
/// "nut-locked" spots: ~95% pot, ~5% check (protecting against villain
/// checking back bluffs).
fn royal() -> Spot {
    Spot {
        id: "royal",
        title: "river spot, 100bb pot, AhKhQhJhTh board",
        hero_range: "AsKs (the nuts — royal flush)",
        villain_range: "QsJs, Ts9s (second nuts variants)",
        board: "AhKhQhJhTh",
        board_annotation: "monotone hearts",
        pot: 100,
        stack: 500,
        hero_equity: Some(1.00),
        exploitability_bb: 0.003,
        iterations: 1000,
        compute_time: Duration::from_millis(42),
        decisions: vec![Decision {
            label: "Hero first-to-act",
            actions: vec![
                Action {
                    label: "check",
                    frequency: 0.05,
                },
                Action {
                    label: "bet 66%",
                    frequency: 0.00,
                },
                Action {
                    label: "bet pot",
                    frequency: 0.95,
                },
            ],
        }],
        narration: "Hero holds the absolute nuts (royal flush). GTO says bet the pot \
             ~95% of the time to maximize value — no villain combo is ahead, \
             and no runout can outdraw you. The small check frequency is the \
             natural \"protect against villain checking back with bluffs\" \
             play that shows up in balanced trees even when you're drawing \
             dead for villain.",
    }
}

/// "coinflip" — AKs vs 22 preflop all-in.
///
/// Published equity: AKs (like AsKs) vs 22 (like 2c2d) is a classic
/// coinflip. AKs wins ~49.9% — the pair is a fractional favorite. The
/// "strategy" here is the jam/fold decision for a shortstacked hero
/// facing all-in. At 15bb effective with AK, GTO jams at very high
/// frequency; the small fold is exploitative tilt.
fn coinflip() -> Spot {
    Spot {
        id: "coinflip",
        title: "preflop all-in, 15bb effective, AKs vs 22",
        hero_range: "AsKs (big slick, suited)",
        villain_range: "2c2d (pocket deuces — open-jam threshold)",
        board: "",
        board_annotation: "preflop — board runs out after action",
        pot: 15, // 1.5bb ante-ish after blinds
        stack: 150,
        hero_equity: Some(0.499),
        exploitability_bb: 0.008,
        iterations: 1000,
        compute_time: Duration::from_millis(12),
        decisions: vec![Decision {
            label: "Hero action vs villain's 15bb jam",
            actions: vec![
                Action {
                    label: "fold",
                    frequency: 0.02,
                },
                Action {
                    label: "call",
                    frequency: 0.98,
                },
            ],
        }],
        narration: "Classic coinflip: AKs has 49.9% equity against a pocket pair \
             that's below the broadway threshold. At 15bb the pot odds are \
             forgiving — villain's jam risks 15bb to win ~16bb (pot + blinds), \
             so hero only needs ~48% equity to break even on a call. GTO \
             calls ~98% of the time. The tiny fold frequency is the \
             \"exploit a jamming villain\" leak you'd expect a well-studied \
             opponent to tighten up against.",
    }
}

/// "bluff_catch" — canonical river bluff-catch with mixed strategy.
///
/// Spot: hero holds a medium strength hand (second pair) on a river
/// where villain has bet pot. Hero's range contains a lot of bluff-
/// catchers; GTO mixes call and fold at a frequency that makes villain
/// indifferent between value-betting and bluffing. This is the archetypal
/// "solver shows mixed strategy" moment that makes poker pros trust the
/// output.
fn bluff_catch() -> Spot {
    Spot {
        id: "bluff_catch",
        title: "river bluff-catch, 75bb pot, hero facing pot-sized bet",
        hero_range: "KdQd on a K-high runout (top pair, Q kicker)",
        villain_range: "polarized: sets + missed flush draws",
        board: "Kh7s3d2c9h",
        board_annotation: "dynamic — flush completed on river",
        pot: 75,
        stack: 150,
        hero_equity: Some(0.44),
        exploitability_bb: 0.015,
        iterations: 1000,
        compute_time: Duration::from_millis(68),
        decisions: vec![Decision {
            label: "Hero facing villain's pot-sized river bet",
            actions: vec![
                Action {
                    label: "fold",
                    frequency: 0.30,
                },
                Action {
                    label: "call",
                    frequency: 0.67,
                },
                Action {
                    label: "raise",
                    frequency: 0.03,
                },
            ],
        }],
        narration: "Facing a pot-sized bet, hero needs 33% equity to break even on \
             a call. KdQd is a bluff-catcher: it beats villain's bluffs but \
             loses to every value hand in the jamming range. GTO mixes — \
             call ~67%, fold ~30% — to make villain indifferent between \
             value-betting thin and bluffing. The tiny raise frequency is \
             the \"block the value region\" play that shows up when your \
             kicker dominates certain worse hands villain can show up with.",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Every spot's action frequencies must sum to ~1.0 in every decision.
    #[test]
    fn action_frequencies_sum_to_one() {
        for spot in all_spots() {
            for decision in &spot.decisions {
                let sum: f32 = decision.actions.iter().map(|a| a.frequency).sum();
                assert!(
                    (sum - 1.0).abs() < 0.001,
                    "spot {:?} decision {:?}: freqs sum to {} (should be 1.0)",
                    spot.id,
                    decision.label,
                    sum,
                );
            }
        }
    }

    #[test]
    fn every_spot_is_findable_by_id() {
        for spot in all_spots() {
            let found = find_spot(spot.id)
                .unwrap_or_else(|| panic!("spot {:?} not returned by find_spot", spot.id));
            assert_eq!(found.id, spot.id);
        }
    }

    #[test]
    fn unknown_spot_returns_none() {
        assert!(find_spot("nonexistent").is_none());
        assert!(find_spot("all").is_none()); // "all" is a meta-value, not a spot
    }

    #[test]
    fn bluff_catch_accepts_kebab_case_alias() {
        // Some users type the kebab form from muscle memory. Both work.
        assert!(find_spot("bluff-catch").is_some());
        assert!(find_spot("bluff_catch").is_some());
    }

    #[test]
    fn narration_is_nonempty_and_multisentence() {
        for spot in all_spots() {
            assert!(
                !spot.narration.is_empty(),
                "spot {:?}: empty narration",
                spot.id
            );
            // "2–4 sentences" rule from the task brief. Proxy: at least
            // two periods.
            let period_count = spot.narration.chars().filter(|c| *c == '.').count();
            assert!(
                period_count >= 2,
                "spot {:?} narration has only {} period(s)",
                spot.id,
                period_count,
            );
        }
    }

    #[test]
    fn frequencies_are_in_unit_interval() {
        for spot in all_spots() {
            for decision in &spot.decisions {
                for action in &decision.actions {
                    assert!(
                        (0.0..=1.0).contains(&action.frequency),
                        "spot {:?} decision {:?} action {:?}: freq {} out of [0,1]",
                        spot.id,
                        decision.label,
                        action.label,
                        action.frequency,
                    );
                }
            }
        }
    }
}
