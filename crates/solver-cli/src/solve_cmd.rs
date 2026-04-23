//! `solve` subcommand: solve a single NLHE spot and print JSON to stdout.
//!
//! This is Henry's dev harness. He types a spot, gets JSON back.
//!
//! ```text
//! solver-cli solve \
//!     --board AhKh2s \
//!     --hero-range "AA,KK,AKs" \
//!     --villain-range "22+,AJs+,KQs" \
//!     --pot 100 --stack 1000 --iterations 1000
//! ```
//!
//! ## Not-yet-implemented upstream
//!
//! As of Day 2 of the sprint, `solver-nlhe::NlheSubgame` and
//! `solver-nlhe::BetTree::default_v0_1` are in varying states of
//! completeness. This subcommand is written so that:
//!
//! 1. Argument parsing + string parsing (board, range, bet-tree name) work
//!    independently — errors there surface immediately, without depending
//!    on downstream solver plumbing. The `solver-cli solve --help` output
//!    works. The unit tests for parsing work.
//! 2. When the solver code calls into an unimplemented stub, we catch the
//!    panic at the FFI-to-upstream boundary and emit a useful error
//!    ("solver-nlhe::NlheSubgame is not yet implemented — run this after
//!    Day 2 main path lands") instead of letting `todo!()` bubble out as a
//!    raw panic. Exit status is non-zero.
//!
//! When the upstream work lands, the `build_subgame` function below is
//! the single place that needs editing.

use std::io::Write;
use std::panic::{self, AssertUnwindSafe};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use solver_core::CfrPlus;
use solver_eval::Board;
use solver_nlhe::{BetTree, NlheSubgame, Range};

/// The solver version string emitted in JSON output. Keep in sync with
/// `Cargo.toml`'s workspace version until we expose it via a build-time
/// constant.
pub const SOLVER_VERSION: &str = "0.1.0-wip";

/// Parsed + validated arguments for `solver-cli solve`.
///
/// Separated from the `clap` struct in `main.rs` so the solve logic can
/// be unit-tested without going through argument parsing.
#[derive(Debug)]
pub struct SolveArgs {
    /// Board string as typed on the CLI (for echo-back in JSON input block).
    pub board_raw: String,
    /// Hero range string, for echo-back.
    pub hero_range_raw: String,
    /// Villain range string, for echo-back.
    pub villain_range_raw: String,
    /// Pot size in chips.
    pub pot: u32,
    /// Effective stack in chips.
    pub stack: u32,
    /// CFR iteration count.
    pub iterations: u32,
    /// Bet-tree profile name. Only `"default"` is recognized at v0.1-wip.
    pub bet_tree: String,
}

/// Parsed/validated inputs after the string-parsing stage. Owned so the
/// solve loop can consume them.
///
/// Most fields will read as "dead" until `build_subgame` is wired up to
/// the real `NlheSubgame::new`; we allow dead_code here rather than in
/// the file so it's obvious at the definition site.
#[allow(dead_code)]
pub struct ParsedInputs {
    pub board: Board,
    pub hero: Range,
    pub villain: Range,
    pub bet_tree: BetTree,
    pub pot: u32,
    pub stack: u32,
    pub iterations: u32,
}

/// Parse the raw string inputs into domain types. Does not touch the solver.
///
/// Errors surface here rather than from the solver, so a typo in a range
/// or board string is caught before we even try to build a subgame.
pub fn parse_inputs(args: &SolveArgs) -> Result<ParsedInputs> {
    let board =
        Board::parse(&args.board_raw).ok_or_else(|| anyhow!("invalid board: {:?}", args.board_raw))?;

    let hero = Range::parse(&args.hero_range_raw)
        .with_context(|| format!("invalid hero range: {:?}", args.hero_range_raw))?;

    let villain = Range::parse(&args.villain_range_raw)
        .with_context(|| format!("invalid villain range: {:?}", args.villain_range_raw))?;

    let bet_tree = match args.bet_tree.as_str() {
        "default" => BetTree::default_v0_1(),
        other => anyhow::bail!(
            "unknown bet-tree profile: {:?} (known: \"default\")",
            other
        ),
    };

    if args.iterations == 0 {
        anyhow::bail!("--iterations must be > 0");
    }
    if args.pot == 0 {
        anyhow::bail!("--pot must be > 0");
    }
    if args.stack == 0 {
        anyhow::bail!("--stack must be > 0");
    }

    Ok(ParsedInputs {
        board,
        hero,
        villain,
        bet_tree,
        pot: args.pot,
        stack: args.stack,
        iterations: args.iterations,
    })
}

/// Entry point for the `solve` subcommand. Writes JSON to `out` (typically
/// stdout) and returns `Ok(())` iff the solve completed and the JSON was
/// flushed.
///
/// If an upstream stub (`NlheSubgame`, bet-tree construction, CFR tree
/// walk) is unimplemented, returns an error with a message pointing at the
/// specific unfinished piece. The caller (`main`) converts that into a
/// non-zero exit status.
pub fn run_solve(args: &SolveArgs, mut out: impl Write) -> Result<()> {
    let parsed = parse_inputs(args)?;

    let input_block = json!({
        "board": args.board_raw,
        "hero_range": args.hero_range_raw,
        "villain_range": args.villain_range_raw,
        "pot": args.pot,
        "stack": args.stack,
        "iterations": args.iterations,
        "bet_tree": args.bet_tree,
    });

    let result_block = solve_to_json(&parsed)?;

    let doc = json!({
        "input": input_block,
        "result": result_block,
        "solver_version": SOLVER_VERSION,
    });

    // Pretty-printed JSON. Henry reads this by eye from the terminal; the
    // extra bytes are free.
    let text = serde_json::to_string_pretty(&doc)?;
    writeln!(out, "{text}")?;
    out.flush()?;
    Ok(())
}

/// The solver-facing half of `run_solve`. Takes fully parsed inputs,
/// returns the `result` JSON object (without the `input`/`solver_version`
/// wrapper).
///
/// Catches any panic from unimplemented upstream (`todo!()`) and converts
/// it to a structured error message rather than letting the process abort.
fn solve_to_json(parsed: &ParsedInputs) -> Result<Value> {
    let start = Instant::now();

    let outcome = panic::catch_unwind(AssertUnwindSafe(|| run_cfr(parsed)));
    match outcome {
        Ok(Ok(summary)) => {
            let compute_ms = start.elapsed().as_millis() as u64;
            Ok(build_result_json(&summary, parsed.iterations, compute_ms))
        }
        Ok(Err(e)) => Err(e),
        Err(panic_payload) => {
            let msg = panic_message(&panic_payload);
            Err(anyhow!(
                "solver not yet fully implemented (panicked at: {msg}) — \
                 run this after Day 2 main path lands (NlheSubgame, \
                 BetTree::default_v0_1, CFR tree walk)."
            ))
        }
    }
}

/// What a successful CFR run returns for JSON packaging.
#[derive(Debug, Default)]
struct SolveSummary {
    action_frequencies: Vec<(String, f32)>,
    ev_per_action: Vec<(String, f32)>,
    hero_equity: f32,
    exploitability: f32,
}

/// The actual solve. This is the single place that needs touching when
/// `solver-nlhe::NlheSubgame` and friends land — everything above is
/// plumbing that's already correct.
///
/// As of 2026-04-23 (Day 2 early), this returns a "blocked upstream"
/// error before ever invoking a `todo!()`. That's intentional: the
/// `panic::catch_unwind` wrapper above would also work, but emitting the
/// error from here gives a more precise message for the common case
/// (nothing has landed yet) and keeps the tree-walk call out of scope
/// until the real subgame constructor exists.
fn run_cfr(parsed: &ParsedInputs) -> Result<SolveSummary> {
    let subgame = build_subgame(parsed)?;

    // Upstream CFR walk. When NlheSubgame::initial_state/legal_actions/etc
    // are still `todo!()`, this will panic; `solve_to_json` catches it.
    let mut solver = CfrPlus::new(subgame);
    solver.run(parsed.iterations);

    // Extract a summary. At the current state of upstream, there's no
    // canonical root-action enumeration plumbed through the CLI yet — the
    // subgame's root info set identifies the root decision, but the CLI
    // doesn't yet know which actions correspond to human-readable labels
    // like "check" / "bet_33". When `NlheSubgame` exposes a
    // `root_action_labels()` helper (see ROADMAP Day 2), wire it up here.
    let summary = summarize_root_strategy(&solver)?;
    Ok(summary)
}

/// Build the NLHE subgame from parsed inputs.
///
/// Today this is a guard that returns a blocked-upstream error because
/// `NlheSubgame` has no public constructor. When the A-main agent lands
/// `NlheSubgame::new(board, hero, villain, pot, stack, bet_tree)` (or
/// whatever final signature they settle on), delete the guard and replace
/// with the real call.
#[allow(clippy::unnecessary_wraps, dead_code)]
fn build_subgame(_parsed: &ParsedInputs) -> Result<NlheSubgame> {
    // PLACEHOLDER — remove when upstream lands.
    //
    // Expected shape (from ROADMAP Day 2):
    //     Ok(NlheSubgame::new(
    //         _parsed.board,
    //         _parsed.hero.clone(),
    //         _parsed.villain.clone(),
    //         _parsed.pot,
    //         _parsed.stack,
    //         _parsed.bet_tree.clone(),
    //     ))
    anyhow::bail!(
        "solver-nlhe::NlheSubgame is not yet implemented — run this after \
         Day 2 main path lands (A-main agent owns NlheSubgame::new)"
    )
}

/// Summarize the solver's root-node strategy into a JSON-ready shape.
///
/// When `NlheSubgame` exposes its root info set and a label map for root
/// actions, this walks the average strategy at the root and maps each
/// action index to a human-readable name. Today it just reports
/// exploitability and iteration count; the per-action frequencies will
/// come online when the upstream plumbing lands and we can identify
/// the root.
fn summarize_root_strategy(solver: &CfrPlus<NlheSubgame>) -> Result<SolveSummary> {
    let exploitability = solver.exploitability();
    Ok(SolveSummary {
        action_frequencies: Vec::new(),
        ev_per_action: Vec::new(),
        hero_equity: 0.0,
        exploitability,
    })
}

/// Package a solve summary into the JSON `result` block defined in the
/// sprint spec (docs/ARCHITECTURE.md + the Day 2 A5 task brief).
fn build_result_json(summary: &SolveSummary, iterations: u32, compute_ms: u64) -> Value {
    let freq_obj: serde_json::Map<String, Value> = summary
        .action_frequencies
        .iter()
        .map(|(k, v)| (k.clone(), Value::from(*v)))
        .collect();
    let ev_obj: serde_json::Map<String, Value> = summary
        .ev_per_action
        .iter()
        .map(|(k, v)| (k.clone(), Value::from(*v)))
        .collect();

    json!({
        "action_frequencies": freq_obj,
        "ev_per_action": ev_obj,
        "hero_equity": summary.hero_equity,
        "exploitability": summary.exploitability,
        "iterations": iterations,
        "compute_ms": compute_ms,
    })
}

/// Extract a string from a caught panic payload. `catch_unwind` returns
/// `Box<dyn Any + Send>`; in practice the payload is `&'static str` (from
/// `panic!("literal")`) or `String` (from `panic!("{fmt}")`). `todo!()`
/// payloads fall under the `String` branch.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn args_for(board: &str, hero: &str, villain: &str) -> SolveArgs {
        SolveArgs {
            board_raw: board.to_string(),
            hero_range_raw: hero.to_string(),
            villain_range_raw: villain.to_string(),
            pot: 100,
            stack: 1000,
            iterations: 1000,
            bet_tree: "default".to_string(),
        }
    }

    #[test]
    fn parse_inputs_accepts_happy_path() {
        let a = args_for("AhKh2s", "AA,KK,AKs", "22+,AJs+,KQs");
        let p = parse_inputs(&a).unwrap();
        assert_eq!(p.board.len, 3);
        // Ranges non-empty.
        assert!(p.hero.total_weight() > 0.0);
        assert!(p.villain.total_weight() > 0.0);
        assert_eq!(p.pot, 100);
        assert_eq!(p.stack, 1000);
        assert_eq!(p.iterations, 1000);
    }

    /// Helper to unwrap to the `Err` arm without requiring `T: Debug`.
    /// `anyhow::Result::unwrap_err` needs `T: Debug` to print on `Ok`
    /// values; `ParsedInputs` can't derive Debug because `Range` and
    /// `BetTree` don't derive it either, and we don't own those.
    fn expect_err<T>(r: Result<T>, ctx: &str) -> anyhow::Error {
        match r {
            Ok(_) => panic!("expected Err ({ctx}), got Ok"),
            Err(e) => e,
        }
    }

    #[test]
    fn parse_inputs_rejects_bad_board() {
        let a = args_for("XxXxXx", "AA", "KK");
        let err = expect_err(parse_inputs(&a), "bad board");
        assert!(
            err.to_string().contains("invalid board"),
            "got: {err}",
        );
    }

    #[test]
    fn parse_inputs_rejects_bad_hero_range() {
        let a = args_for("AhKh2s", "ZZ", "KK");
        let err = expect_err(parse_inputs(&a), "bad hero range");
        assert!(
            err.to_string().to_lowercase().contains("hero range"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_inputs_rejects_bad_villain_range() {
        let a = args_for("AhKh2s", "AA", "not-a-range");
        let err = expect_err(parse_inputs(&a), "bad villain range");
        assert!(
            err.to_string().to_lowercase().contains("villain range"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_inputs_rejects_unknown_bet_tree() {
        let mut a = args_for("AhKh2s", "AA", "KK");
        a.bet_tree = "mystery-tree".to_string();
        let err = expect_err(parse_inputs(&a), "unknown bet-tree");
        assert!(err.to_string().contains("bet-tree"), "got: {err}");
    }

    #[test]
    fn parse_inputs_rejects_zero_iterations() {
        let mut a = args_for("AhKh2s", "AA", "KK");
        a.iterations = 0;
        let err = expect_err(parse_inputs(&a), "zero iterations");
        assert!(err.to_string().contains("iterations"), "got: {err}");
    }

    #[test]
    fn parse_inputs_rejects_zero_pot() {
        let mut a = args_for("AhKh2s", "AA", "KK");
        a.pot = 0;
        let err = expect_err(parse_inputs(&a), "zero pot");
        assert!(err.to_string().contains("pot"), "got: {err}");
    }

    #[test]
    fn parse_inputs_rejects_zero_stack() {
        let mut a = args_for("AhKh2s", "AA", "KK");
        a.stack = 0;
        let err = expect_err(parse_inputs(&a), "zero stack");
        assert!(err.to_string().contains("stack"), "got: {err}");
    }

    #[test]
    fn parse_inputs_accepts_preflop_empty_board() {
        let a = args_for("", "AA", "KK");
        let p = parse_inputs(&a).unwrap();
        assert_eq!(p.board.len, 0);
    }

    #[test]
    fn parse_inputs_accepts_turn_and_river_boards() {
        let a = args_for("AhKh2sQc", "AA", "KK");
        assert_eq!(parse_inputs(&a).unwrap().board.len, 4);

        let a = args_for("AhKh2sQc4d", "AA", "KK");
        assert_eq!(parse_inputs(&a).unwrap().board.len, 5);
    }

    #[test]
    fn run_solve_returns_useful_error_when_upstream_missing() {
        // Day 2-early state: NlheSubgame has no constructor. `run_solve`
        // must surface a readable error, not a raw todo! panic.
        let a = args_for("AhKh2s", "AA,KK,AKs", "22+,AJs+,KQs");
        let mut out = Vec::new();
        let err = run_solve(&a, &mut out).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("NlheSubgame") || msg.contains("not yet"),
            "expected blocked-upstream message, got: {msg}"
        );
        // No JSON should have been emitted — we bailed before writing.
        assert!(out.is_empty());
    }

    #[test]
    fn parse_inputs_accepts_multiword_ranges_with_whitespace() {
        let a = args_for("AhKh2s", "  AA ,\tKK ,AKs ", " 22+ , AJs+ , KQs ");
        let p = parse_inputs(&a).unwrap();
        assert!(p.hero.total_weight() > 0.0);
        assert!(p.villain.total_weight() > 0.0);
    }
}
