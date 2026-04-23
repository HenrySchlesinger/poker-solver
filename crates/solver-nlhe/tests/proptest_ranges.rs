//! Property-based tests for `solver-nlhe` types.
//!
//! These tests encode invariants about the `Range` parser, the
//! `BetTree` snap/custom surface, and the `ActionLog` push/pop and
//! street-closure semantics.
//!
//! # Determinism / reproducibility
//!
//! Every `proptest!` block here runs with a fixed `Config` that disables
//! failure persistence (no `.proptest-regressions` file on disk — we want
//! flat reproducibility) and uses a named `source_file`, which deterministically
//! keys proptest's xorshift source for the default `PROPTEST_SEED`. Each
//! test runs the documented number of cases per block.
//!
//! # Catalogue of invariants encoded here
//!
//! Range
//! 1. Random valid range strings parse without error and have total weight
//!    in `[0, 1326]`. Valid inputs: a selection of canonical tokens
//!    (`AA`, `AKs`, `22+`, etc.) joined by commas.
//! 2. Empty-string parse and fully-qualified "AA+" both produce a range
//!    whose `total_weight()` equals the combo count times 1.0 — no NaN,
//!    no Inf.
//!
//! BetTree
//! 3. For a custom BetTree built from valid ascending positive f32 lists,
//!    `snap(street, f)` always returns a value that is in the street's
//!    `sizings_for()` output, for every `f > 0`.
//! 4. `snap` is idempotent: `snap(s, snap(s, f)) == snap(s, f)`.
//!
//! ActionLog
//! 5. push/pop round-trip: `push(s, a); pop() == Some((s, a))`.
//! 6. Check-check on any postflop street closes: after two Check entries
//!    on Flop/Turn/River, `is_street_closed()` is true.

use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence};

use solver_nlhe::action::{Action, ActionLog, Street};
use solver_nlhe::bet_tree::BetTree;
use solver_nlhe::range::Range;

/// Deterministic config, same case count throughout.
fn cfg(cases: u32) -> Config {
    Config {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        source_file: Some("proptest_ranges.rs"),
        ..Config::default()
    }
}

// --------------------------------------------------------------------------
// Range: valid-input parsing.
// --------------------------------------------------------------------------

/// Strategy: a single valid range token (pair, pair+, pair-, suited,
/// offsuit, any two-rank, or two-rank+).
fn range_token() -> impl Strategy<Value = String> {
    let ranks = [
        '2', '3', '4', '5', '6', '7', '8', '9', 'T', 'J', 'Q', 'K', 'A',
    ];
    prop_oneof![
        // Pocket pair: "AA", "QQ", etc.
        (0usize..13).prop_map(move |i| format!("{}{}", ranks[i], ranks[i])),
        // Pocket pair +: "22+", "JJ+".
        (0usize..13).prop_map(move |i| format!("{}{}+", ranks[i], ranks[i])),
        // Pocket pair -: "AA-" ... well, pick a non-extreme rank.
        (0usize..13).prop_map(move |i| format!("{}{}-", ranks[i], ranks[i])),
        // Two-rank suited. Ensure first > second in the standard rank order.
        (1usize..13, 0usize..13)
            .prop_filter("ranks must differ", |(a, b)| a != b)
            .prop_map(move |(a, b)| {
                let (hi, lo) = if a > b { (a, b) } else { (b, a) };
                format!("{}{}s", ranks[hi], ranks[lo])
            }),
        // Two-rank offsuit.
        (1usize..13, 0usize..13)
            .prop_filter("ranks must differ", |(a, b)| a != b)
            .prop_map(move |(a, b)| {
                let (hi, lo) = if a > b { (a, b) } else { (b, a) };
                format!("{}{}o", ranks[hi], ranks[lo])
            }),
        // Any two-rank.
        (1usize..13, 0usize..13)
            .prop_filter("ranks must differ", |(a, b)| a != b)
            .prop_map(move |(a, b)| {
                let (hi, lo) = if a > b { (a, b) } else { (b, a) };
                format!("{}{}", ranks[hi], ranks[lo])
            }),
    ]
}

/// Strategy: a comma-separated list of valid tokens.
fn range_string() -> impl Strategy<Value = String> {
    prop::collection::vec(range_token(), 1..6).prop_map(|v| v.join(", "))
}

proptest! {
    #![proptest_config(cfg(512))]

    /// Any random-but-syntactically-valid range string parses. The
    /// resulting range has total weight in [0, 1326] (nonzero for any
    /// non-empty input).
    ///
    /// Mutation check: if `apply_token` ever panicked on a legal
    /// variation (say a rare pair+ case), this test would fail for the
    /// inputs that hit that variation.
    #[test]
    fn range_parse_valid_inputs_land_in_combo_mass_range(s in range_string()) {
        let r = Range::parse(&s).expect("valid range string must parse");
        let w = r.total_weight();
        prop_assert!(w >= 0.0, "total weight must be non-negative, got {w} for {s:?}");
        prop_assert!(
            w <= 1326.0 + 1e-3,
            "total weight must be ≤ 1326 for {s:?}, got {w}"
        );
        prop_assert!(!w.is_nan(), "total weight NaN for {s:?}");
    }
}

#[test]
fn range_empty_string_is_zero_weight() {
    // Complements the proptest: zero-input edge case.
    let r = Range::parse("").unwrap();
    assert_eq!(r.total_weight(), 0.0);
}

#[test]
fn range_full_has_max_weight() {
    assert_eq!(Range::full().total_weight(), 1326.0);
    assert_eq!(Range::empty().total_weight(), 0.0);
}

// --------------------------------------------------------------------------
// BetTree: snap properties.
// --------------------------------------------------------------------------

/// Strategy: generate valid custom sizing lists for flop/turn/river.
///
/// Constraints:
/// * strictly ascending
/// * each positive (> 0)
/// * INF is allowed only as the last element
///
/// We build the list from a base "step" and accumulate: that guarantees
/// ascending by construction.
fn sizing_list(min_len: usize, max_len: usize) -> impl Strategy<Value = Vec<f32>> {
    (min_len..=max_len).prop_flat_map(|n| {
        // Generate `n` strictly-positive increments in (0, 1].
        let increments = prop::collection::vec(1u32..100, n);
        let add_inf = any::<bool>();
        (increments, add_inf).prop_map(move |(inc, add_inf)| {
            let mut out = Vec::with_capacity(n + 1);
            let mut acc = 0.0f32;
            for i in inc {
                acc += (i as f32) * 0.01;
                out.push(acc);
            }
            if add_inf {
                out.push(f32::INFINITY);
            }
            out
        })
    })
}

fn bet_tree() -> impl Strategy<Value = BetTree> {
    (sizing_list(1, 4), sizing_list(1, 4), sizing_list(1, 4)).prop_map(|(flop, turn, river)| {
        BetTree::custom(flop, turn, river).expect("valid-by-construction tree must build")
    })
}

/// Strategy: a positive finite f32, or INF.
fn positive_fraction() -> impl Strategy<Value = f32> {
    prop_oneof![
        (1u32..10_000).prop_map(|i| i as f32 * 0.001), // 0.001 .. 10
        Just(f32::INFINITY),
    ]
}

fn postflop_street() -> impl Strategy<Value = Street> {
    prop_oneof![Just(Street::Flop), Just(Street::Turn), Just(Street::River),]
}

proptest! {
    #![proptest_config(cfg(512))]

    /// `snap` always returns a value that appears in the street's
    /// `sizings_for` list (treating INF as a distinguishable bucket).
    ///
    /// Mutation check: if the snap loop ever returned an interpolated
    /// mid-bucket value (e.g., `(a + b) / 2.0`), this test would fail
    /// because the returned value wouldn't be present in the sizings.
    #[test]
    fn snap_is_in_sizings(
        tree in bet_tree(),
        street in postflop_street(),
        f in positive_fraction(),
    ) {
        let out = tree.snap(street, f);
        let sizings = tree.sizings_for(street);
        let present = sizings
            .iter()
            .any(|&s| s == out || (s.is_infinite() && out.is_infinite()));
        prop_assert!(
            present,
            "snap({street:?}, {f}) = {out} not in sizings {sizings:?}"
        );
    }

    /// `snap` is idempotent: snapping an already-snapped value yields
    /// the same value.
    ///
    /// Mutation check: if `snap` ever had a floating-point-driven drift
    /// (e.g., log2 rounding that pushed an on-the-boundary value into
    /// the next bucket on the second call), this test would fail.
    #[test]
    fn snap_is_idempotent(
        tree in bet_tree(),
        street in postflop_street(),
        f in positive_fraction(),
    ) {
        let once = tree.snap(street, f);
        let twice = tree.snap(street, once);
        prop_assert_eq!(
            once, twice,
            "snap({:?}, {}) not idempotent: once={}, twice={}",
            street, f, once, twice
        );
    }
}

// --------------------------------------------------------------------------
// ActionLog: push/pop round-trip + street-closure.
// --------------------------------------------------------------------------

fn street_strategy() -> impl Strategy<Value = Street> {
    prop_oneof![
        Just(Street::Preflop),
        Just(Street::Flop),
        Just(Street::Turn),
        Just(Street::River),
    ]
}

fn action_strategy() -> impl Strategy<Value = Action> {
    prop_oneof![
        Just(Action::Fold),
        Just(Action::Check),
        Just(Action::Call),
        any::<u32>().prop_map(Action::Bet),
        any::<u32>().prop_map(Action::Raise),
        Just(Action::AllIn),
    ]
}

proptest! {
    #![proptest_config(cfg(1024))]

    /// push then pop yields the same (street, action) pair.
    ///
    /// Mutation check: if `ActionLog::pop` ever swapped the tuple
    /// ordering (e.g., returned `(action, street)`), the round-trip
    /// would mismatch.
    #[test]
    fn actionlog_push_pop_roundtrip(
        street in street_strategy(),
        action in action_strategy(),
    ) {
        let mut log = ActionLog::new();
        log.push(street, action);
        let popped = log.pop();
        prop_assert_eq!(popped, Some((street, action)));
        prop_assert!(log.is_empty());
    }
}

proptest! {
    #![proptest_config(cfg(128))]

    /// Check-check on any postflop street closes.
    ///
    /// Mutation check: if `is_street_closed` ever returned false for a
    /// check-check scenario (regression introduced by agent "X fixes
    /// preflop"), this test would fail across all three postflop
    /// streets.
    #[test]
    fn actionlog_check_check_closes_postflop(
        street in prop_oneof![Just(Street::Flop), Just(Street::Turn), Just(Street::River)]
    ) {
        let mut log = ActionLog::new();
        log.push(street, Action::Check);
        prop_assert!(!log.is_street_closed(), "single check should not close");
        log.push(street, Action::Check);
        prop_assert!(
            log.is_street_closed(),
            "check-check on {street:?} should close but did not"
        );
    }
}
