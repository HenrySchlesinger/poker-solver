//! `solve` subcommand: solve a single NLHE spot and print JSON to stdout.
//!
//! This is Henry's dev harness. He types a spot, gets JSON back.
//!
//! ```text
//! solver-cli solve \
//!     --board AhKhQhJhTh \
//!     --hero-range "AsKs" \
//!     --villain-range "AsKs" \
//!     --pot 100 --stack 500 --iterations 100
//! ```
//!
//! ## Architecture
//!
//! The flow is:
//!
//! 1. **Parse** raw strings into domain types (`Board`, `Range`,
//!    `BetTree`). Typos surface here with a clean error, independently
//!    of the solver.
//! 2. **Build** the NLHE river subgame via `NlheSubgame::new`. v0.1 is
//!    river-only; non-river boards bail with a clear error.
//! 3. **Enumerate** the chance-layer root mixture via
//!    `NlheSubgame::chance_roots` — one entry per non-conflicting
//!    `(hero_combo, villain_combo)` pair, weighted by the normalized
//!    product of range weights.
//! 4. **Solve** via `CfrPlus::run_from(roots, iterations)`.
//! 5. **Aggregate** the root strategy + per-action EV across combo
//!    pairs. Compute range-vs-range equity separately via
//!    `solver_eval::equity::range_vs_range_equity`.
//! 6. **Emit** JSON to `out`.
//!
//! A `panic::catch_unwind` still wraps the solve call so a stray
//! `todo!()` or assertion failure in an upstream crate becomes a clean
//! non-zero exit rather than an ugly process abort.

use std::io::Write;
use std::panic::{self, AssertUnwindSafe};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use solver_core::{CfrPlus, CfrPlusFlat, Game, Player, Strategy};
use solver_eval::card::Card;
use solver_eval::combo::combo_index;
use solver_eval::Board;
use solver_nlhe::action::Action;
use solver_nlhe::subgame::SubgameState;
use solver_nlhe::{BetTree, NlheSubgame, Range};

/// The solver version string emitted in JSON output. Keep in sync with
/// `Cargo.toml`'s workspace version until we expose it via a build-time
/// constant.
pub const SOLVER_VERSION: &str = "0.1.0-wip";

/// Worker-thread stack size for the CFR tree walk.
///
/// `solver_core::CfrPlus::walk` is a recursive descent over the game
/// tree. The v0.1 NLHE bet tree (five river sizings plus raise
/// continuations) can produce depths that overflow the default 8 MB
/// thread stack on macOS — we've observed this on even a 1-combo-vs-1
/// river spot with full default sizings. Running the solve on a
/// dedicated thread with a fat stack keeps the process independent of
/// how aggressive the bet tree gets.
///
/// 128 MB is overkill for v0.1 but cheap — the thread is torn down the
/// moment the solve finishes, and committed pages are lazy.
const SOLVE_THREAD_STACK_BYTES: usize = 128 * 1024 * 1024;

/// Which CFR+ solver implementation to drive. Callers pick via
/// `--solver flat|classic`; `flat` is the default post-A64 (flat
/// `RegretTables` + SIMD regret matching, ~3-9× faster on river spots).
/// `classic` is the original `HashMap<InfoSetId, _>` implementation,
/// kept as an escape hatch for reproducibility and convergence-check
/// comparisons against the flat path (see `tests/flat_equivalence.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolverKind {
    /// `CfrPlusFlat` — flat-array `RegretTables` + SIMD regret matching.
    /// Default as of A64. Requires upfront info-set enumeration (a few
    /// ms on an NLHE river subgame, amortized over the CFR iterations).
    Flat,
    /// `CfrPlus` — `HashMap<InfoSetId, _>` reference implementation.
    /// Slower but has no enumeration cost and is the convergence oracle
    /// for `tests/flat_equivalence.rs`.
    Classic,
}

impl SolverKind {
    /// Parse the CLI `--solver` value. Unknown strings produce a readable
    /// error; "flat" and "classic" are the only accepted values today.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "flat" => Ok(Self::Flat),
            "classic" => Ok(Self::Classic),
            other => anyhow::bail!(
                "unknown --solver value {:?} (known: \"flat\", \"classic\")",
                other
            ),
        }
    }
}

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
    /// Which solver implementation to use. Defaults to `Flat` (the
    /// post-A64 flat+SIMD path). `Classic` keeps the HashMap reference
    /// implementation available for convergence comparisons.
    pub solver: SolverKind,
}

/// Parsed/validated inputs after the string-parsing stage. Owned so the
/// solve loop can consume them.
pub struct ParsedInputs {
    pub board: Board,
    pub hero: Range,
    pub villain: Range,
    pub bet_tree: BetTree,
    pub pot: u32,
    pub stack: u32,
    pub iterations: u32,
    pub solver: SolverKind,
}

/// Parse the raw string inputs into domain types. Does not touch the solver.
///
/// Errors surface here rather than from the solver, so a typo in a range
/// or board string is caught before we even try to build a subgame.
pub fn parse_inputs(args: &SolveArgs) -> Result<ParsedInputs> {
    let board = Board::parse(&args.board_raw)
        .ok_or_else(|| anyhow!("invalid board: {:?}", args.board_raw))?;

    let hero = parse_range_allowing_specific_combos(&args.hero_range_raw)
        .with_context(|| format!("invalid hero range: {:?}", args.hero_range_raw))?;

    let villain = parse_range_allowing_specific_combos(&args.villain_range_raw)
        .with_context(|| format!("invalid villain range: {:?}", args.villain_range_raw))?;

    let bet_tree = match args.bet_tree.as_str() {
        "default" => BetTree::default_v0_1(),
        other => anyhow::bail!("unknown bet-tree profile: {:?} (known: \"default\")", other),
    };

    if args.iterations == 0 {
        anyhow::bail!("--iterations must be > 0");
    }
    if args.pot == 0 {
        anyhow::bail!("--pot must be > 0");
    }
    // `--stack 0` is a legitimate river-subgame configuration: both
    // players are already all-in before the river, so the only legal
    // action is Check and the tree collapses to Check/Check → showdown.
    // See `solver-nlhe/tests/river_canonical.rs::trivial_allin_showdown`.

    Ok(ParsedInputs {
        board,
        hero,
        villain,
        bet_tree,
        pot: args.pot,
        stack: args.stack,
        iterations: args.iterations,
        solver: args.solver,
    })
}

/// Parse a range string, allowing specific-combo tokens (e.g. `"AsKs"`,
/// `"7c7d"`) in addition to everything `Range::parse` already handles.
///
/// A specific-combo token is a 4-character string of the form
/// `<rank><suit><rank><suit>`, where both suits are explicit letters.
/// We detect these tokens and apply them directly via `combo_index`;
/// everything else (pocket pairs, `"AKs"`-style suitedness, `"22+"`,
/// weight suffixes, whitespace) is delegated to `Range::parse`, one
/// token at a time.
///
/// This exists so the dev-harness CLI can take spot-precise inputs
/// (like a single royal-flush-vs-royal-flush combo) without growing the
/// grammar of the core `Range` parser.
///
/// Semantics match `Range::parse`'s last-write-wins: tokens are
/// processed left-to-right, and a later token's non-zero weight
/// overwrites any combo already written by an earlier token. A
/// `:weight` suffix on a specific-combo token applies to that combo
/// only; `Range::parse` handles suffixes on its own tokens.
fn parse_range_allowing_specific_combos(s: &str) -> Result<Range> {
    let mut range = Range::empty();
    for raw in s.split(',') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }

        // Optional ":weight" suffix, only peeled off for specific-combo
        // tokens. Broader tokens keep the suffix intact so `Range::parse`
        // handles it natively.
        let (body, weight) = match token.rsplit_once(':') {
            Some((b, w)) => {
                let w_trim = w.trim();
                // Only try to parse the suffix if the body looks like a
                // specific-combo token. Otherwise we pass the original
                // token through to `Range::parse` unchanged.
                if try_parse_specific_combo(b.trim()).is_some() {
                    let parsed: f32 = w_trim.parse().with_context(|| {
                        format!("bad weight in token {token:?} (suffix {w_trim:?})")
                    })?;
                    (b.trim(), parsed)
                } else {
                    (token, 1.0)
                }
            }
            None => (token, 1.0),
        };

        // Specific-combo token: 4 chars, rank+suit+rank+suit. Applied
        // directly; no need to go through `Range::parse`.
        if let Some((a, b)) = try_parse_specific_combo(body) {
            if a == b {
                anyhow::bail!("combo token {token:?} uses the same card twice");
            }
            let idx = combo_index(a, b);
            range.weights[idx] = weight;
            continue;
        }

        // Not a specific-combo token: delegate a single token at a time
        // to the core parser. Parsing token-by-token (rather than
        // joining and calling `Range::parse` once) keeps the
        // left-to-right last-write-wins semantics sensible when the
        // user mixes specific and broad tokens.
        let sub = Range::parse(token)
            .map_err(|e| anyhow!("unknown token {token:?} in range {s:?}: {e}"))?;
        // Overlay: any non-zero weight from `sub` replaces whatever is
        // currently in `range` for that combo. Zero weights do not
        // overwrite — otherwise a later broad token could erase an
        // earlier specific-combo setting just by touching other combos,
        // which is surprising.
        for (i, &w) in sub.weights.iter().enumerate() {
            if w != 0.0 {
                range.weights[i] = w;
            }
        }
    }
    Ok(range)
}

/// Attempt to read `body` as a specific-combo token
/// (`<rank><suit><rank><suit>`, 4 characters, e.g. `"AsKs"`).
///
/// Returns `Some((card_a, card_b))` on match, `None` otherwise.
fn try_parse_specific_combo(body: &str) -> Option<(Card, Card)> {
    if body.len() != 4 {
        return None;
    }
    let a = Card::parse(&body[0..2])?;
    let b = Card::parse(&body[2..4])?;
    Some((a, b))
}

/// Entry point for the `solve` subcommand. Writes JSON to `out` (typically
/// stdout) and returns `Ok(())` iff the solve completed and the JSON was
/// flushed.
///
/// If something fails upstream, returns an error with a message pointing
/// at the specific failure. The caller (`main`) converts that into a
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
/// The CFR walk runs on a dedicated worker thread with a large stack
/// (`SOLVE_THREAD_STACK_BYTES`) so deep bet trees don't overflow the
/// default 8 MB macOS thread stack. A `panic::catch_unwind` inside the
/// worker converts any upstream panic (`todo!()`, assertion failure)
/// into a structured error rather than letting the process abort.
fn solve_to_json(parsed: &ParsedInputs) -> Result<Value> {
    let start = Instant::now();

    // Clone into owned data so we can move it into the worker thread.
    // `ParsedInputs` isn't `Send`-assumed by the caller, but all its
    // fields (`Board`, `Range`, `BetTree`, u32s) are. Construct a fresh
    // owned copy for the thread.
    let parsed_owned = ParsedInputs {
        board: parsed.board,
        hero: parsed.hero.clone(),
        villain: parsed.villain.clone(),
        bet_tree: parsed.bet_tree.clone(),
        pot: parsed.pot,
        stack: parsed.stack,
        iterations: parsed.iterations,
        solver: parsed.solver,
    };

    let worker = std::thread::Builder::new()
        .name("solver-cli-cfr".to_string())
        .stack_size(SOLVE_THREAD_STACK_BYTES)
        .spawn(move || panic::catch_unwind(AssertUnwindSafe(|| run_cfr(&parsed_owned))))
        .context("failed to spawn CFR worker thread")?;

    // `JoinHandle::join` returns `Err` only if the worker panicked
    // outside the `catch_unwind` (e.g. a panic during stack setup).
    // That's fatal; report it cleanly.
    let joined = worker
        .join()
        .map_err(|p| anyhow!("CFR worker thread panicked: {}", panic_message(&p)))?;

    match joined {
        Ok(Ok(summary)) => {
            let compute_ms = start.elapsed().as_millis() as u64;
            Ok(build_result_json(&summary, parsed.iterations, compute_ms))
        }
        Ok(Err(e)) => Err(e),
        Err(panic_payload) => {
            let msg = panic_message(&panic_payload);
            Err(anyhow!("solver panicked: {msg}"))
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

/// The actual solve. Builds the NLHE river subgame, enumerates the
/// chance-layer root mixture (one entry per non-conflicting combo pair,
/// weighted by the product of range weights), runs CFR+ over it, and
/// summarizes the root strategy + per-action EV + range-vs-range equity.
///
/// v0.1 hard-codes `first_to_act = Hero`. When `--to-act` surfaces as a
/// CLI flag, replace the hard-coded value here.
fn run_cfr(parsed: &ParsedInputs) -> Result<SolveSummary> {
    // v0.1 restriction: river only. Guard here (not in `parse_inputs`)
    // so parse-stage tests that accept 3/4/5-card boards keep passing.
    if parsed.board.len != 5 {
        anyhow::bail!(
            "v0.1 supports river-only subgames (need 5 board cards, got {})",
            parsed.board.len
        );
    }

    let subgame = build_subgame(parsed)?;

    // Enumerate the chance-layer root mixture.
    let roots = subgame.chance_roots();
    if roots.is_empty() {
        anyhow::bail!(
            "no valid (hero, villain) combo pairs — ranges conflict with the board \
             or with each other, so CFR has nothing to solve"
        );
    }

    // Range-vs-range equity. Computed against the parsed ranges (the
    // subgame holds identical copies internally). `samples` is ignored
    // on a 5-card board (exact enumeration).
    let hero_equity = solver_eval::equity::range_vs_range_equity(
        &parsed.hero.weights,
        &parsed.villain.weights,
        &parsed.board,
        1,
    );

    // Run CFR+ from the weighted root set. Dispatch on `--solver`:
    // `Flat` drives the post-A64 flat-array + SIMD path (default),
    // `Classic` keeps the reference HashMap implementation alive as an
    // escape hatch.
    let (exploitability, avg_strategy, action_frequencies, ev_per_action) = match parsed.solver {
        SolverKind::Flat => {
            let mut solver = CfrPlusFlat::from_roots(subgame, &roots);
            solver.run_from(&roots, parsed.iterations);
            let expl = solver.exploitability();
            let avg = solver.average_strategy();
            let (freq, ev) = aggregate_root_strategy_and_ev(solver.game(), &avg, &roots);
            (expl, avg, freq, ev)
        }
        SolverKind::Classic => {
            let mut solver = CfrPlus::new(subgame);
            solver.run_from(&roots, parsed.iterations);
            let expl = solver.exploitability();
            let avg = solver.average_strategy();
            let (freq, ev) = aggregate_root_strategy_and_ev(solver.game(), &avg, &roots);
            (expl, avg, freq, ev)
        }
    };
    // Keep `avg_strategy` around so the `let` above doesn't warn about
    // an unused binding — downstream aggregation consumed the summary.
    let _ = &avg_strategy;

    Ok(SolveSummary {
        action_frequencies,
        ev_per_action,
        hero_equity,
        exploitability,
    })
}

/// Build the NLHE river subgame from parsed inputs.
///
/// v0.1 hard-codes `first_to_act = Hero`. When the CLI exposes a
/// `--to-act` flag, thread it through `SolveArgs` and into here.
fn build_subgame(parsed: &ParsedInputs) -> Result<NlheSubgame> {
    Ok(NlheSubgame::new(
        parsed.board,
        parsed.hero.clone(),
        parsed.villain.clone(),
        parsed.pot,
        parsed.stack,
        Player::Hero,
        parsed.bet_tree.clone(),
    ))
}

/// A list of (action-label, weight) pairs, used for both the aggregated
/// root frequency vector and the matching EV-per-action vector. The two
/// share a shape because they're two views on the same ordered action
/// set at the root — labels line up 1:1.
type ActionWeights = Vec<(String, f32)>;

/// Aggregate the per-combo-pair root strategy into a single
/// action-frequency vector, and compute EV per action.
///
/// Under the pair-enumeration (chance-layer) formulation there is one
/// "root info set" per first-to-act combo. We walk each root in
/// `roots`, fetch first-to-act's info-set strategy, weight it by the
/// chance-layer prior for that root, and sum.
///
/// EV per action is computed by one-ply lookahead: for each action at
/// the root, walk the subtree under `avg_strategy` (both players
/// following the average) and compute the expected utility for
/// first-to-act, then weight-sum across combo pairs.
fn aggregate_root_strategy_and_ev(
    game: &NlheSubgame,
    avg_strategy: &Strategy,
    roots: &[(SubgameState, f32)],
) -> (ActionWeights, ActionWeights) {
    // Establish the action set + labels from a representative root. All
    // roots share the same legal-actions list (root state: empty action
    // log, stacks + pot untouched — deterministic from subgame config).
    let Some((first_root, _)) = roots.first() else {
        return (Vec::new(), Vec::new());
    };
    let root_actions = game.legal_actions(first_root);
    let num_actions = root_actions.len();
    let labels: Vec<String> = root_actions.iter().map(action_label).collect();

    // Who acts at the root? All roots share this — `current_player` is
    // a function of the action history, which is empty at every root.
    let first_to_act = game.current_player(first_root);

    // Weighted accumulators across combo pairs.
    let mut freq_acc = vec![0.0_f64; num_actions];
    let mut ev_acc = vec![0.0_f64; num_actions];
    let mut total_weight = 0.0_f64;

    for (root_state, weight) in roots {
        let w = *weight as f64;
        if w == 0.0 {
            continue;
        }

        // Fetch first_to_act's strategy at this root (keyed on their
        // combo + empty action history). Fall back to uniform if the
        // info set was never reached (shouldn't happen for a live root,
        // but matches Strategy::get's None handling).
        let info = game.info_set(root_state, first_to_act);
        let uniform_fallback: Vec<f32>;
        let strat: &[f32] = match avg_strategy.get(info) {
            Some(s) => s,
            None => {
                uniform_fallback = vec![1.0 / num_actions as f32; num_actions];
                &uniform_fallback
            }
        };
        debug_assert_eq!(strat.len(), num_actions);

        // Walk each action once to get subtree EV for first_to_act.
        for (i, action) in root_actions.iter().enumerate() {
            let next = game.apply(root_state, action);
            let child_ev = subtree_ev_under_avg_strategy(game, &next, avg_strategy, first_to_act);
            ev_acc[i] += w * child_ev as f64;
            freq_acc[i] += w * strat[i] as f64;
        }
        total_weight += w;
    }

    // Normalize frequencies + EVs (defensive: roots are already normalized
    // to sum to 1 by `chance_roots`, so `total_weight` should be ~1.0).
    if total_weight > 0.0 {
        for f in &mut freq_acc {
            *f /= total_weight;
        }
        for ev in &mut ev_acc {
            *ev /= total_weight;
        }
    }

    let frequencies: Vec<(String, f32)> = labels
        .iter()
        .zip(freq_acc.iter())
        .map(|(lbl, f)| (lbl.clone(), *f as f32))
        .collect();
    let evs: Vec<(String, f32)> = labels
        .iter()
        .zip(ev_acc.iter())
        .map(|(lbl, ev)| (lbl.clone(), *ev as f32))
        .collect();

    (frequencies, evs)
}

/// Expected utility for `player` at `state`, walking the subtree under
/// `avg_strategy` (both players follow the averaged strategy, with a
/// uniform fallback for info sets the averaging never touched).
fn subtree_ev_under_avg_strategy(
    game: &NlheSubgame,
    state: &SubgameState,
    avg_strategy: &Strategy,
    player: Player,
) -> f32 {
    if game.is_terminal(state) {
        return game.utility(state, player);
    }
    let current = game.current_player(state);
    let actions = game.legal_actions(state);
    let n = actions.len();
    debug_assert!(n > 0, "non-terminal with no legal actions");

    let info = game.info_set(state, current);
    let uniform_fallback: Vec<f32>;
    let strat: &[f32] = match avg_strategy.get(info) {
        Some(s) => s,
        None => {
            uniform_fallback = vec![1.0 / n as f32; n];
            &uniform_fallback
        }
    };

    let mut val = 0.0_f32;
    for (i, action) in actions.iter().enumerate() {
        let p = strat[i];
        if p == 0.0 {
            continue;
        }
        let next = game.apply(state, action);
        val += p * subtree_ev_under_avg_strategy(game, &next, avg_strategy, player);
    }
    val
}

/// Human-readable label for a root action. Names are stable and suitable
/// as JSON object keys: `check`, `call`, `fold`, `bet_<chips>`,
/// `raise_<chips>`, `allin`.
fn action_label(a: &Action) -> String {
    match a {
        Action::Fold => "fold".to_string(),
        Action::Check => "check".to_string(),
        Action::Call => "call".to_string(),
        Action::Bet(amt) => format!("bet_{amt}"),
        Action::Raise(amt) => format!("raise_{amt}"),
        Action::AllIn => "allin".to_string(),
    }
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
            solver: SolverKind::Flat,
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
        assert!(err.to_string().contains("invalid board"), "got: {err}",);
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
    fn parse_inputs_accepts_zero_stack() {
        // stack=0 is the "already all-in into the river" configuration;
        // legit in a river-only v0.1 subgame. See `run_cfr` and
        // `trivial_allin_showdown` in `solver-nlhe/tests/river_canonical.rs`.
        let mut a = args_for("AhKh2s", "AA", "KK");
        a.stack = 0;
        let p = parse_inputs(&a).unwrap();
        assert_eq!(p.stack, 0);
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
    fn run_solve_rejects_non_river_board_with_readable_error() {
        // v0.1 handles river-only subgames. A flop board must bail
        // with a readable error, not a raw panic from `NlheSubgame::new`.
        let a = args_for("AhKh2s", "AA,KK,AKs", "22+,AJs+,KQs");
        let mut out = Vec::new();
        let err = run_solve(&a, &mut out).unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("river") || msg.contains("5 board"),
            "expected river-only restriction message, got: {msg}"
        );
        // No JSON should have been emitted — we bailed before writing.
        assert!(out.is_empty());
    }

    #[test]
    fn run_solve_produces_json_on_river_spot() {
        // Full end-to-end on a river spot. The "trivial all-in showdown"
        // shape: both players are already all-in entering the river
        // (stack_start = 0), so the only legal action at every state
        // is Check and the tree collapses to Check/Check → showdown.
        // Mirrors `solver-nlhe/tests/river_canonical.rs::trivial_allin_showdown`.
        //
        // Hero holds Ah-Kh, Villain holds As-Ad. Board 2c7d9hTsJs.
        // Villain's pocket aces make a pair; Hero has A-high only.
        let a = SolveArgs {
            board_raw: "2c7d9hTsJs".to_string(),
            hero_range_raw: "AhKh".to_string(),
            villain_range_raw: "AsAd".to_string(),
            pot: 100,
            stack: 0,
            iterations: 10,
            bet_tree: "default".to_string(),
            solver: SolverKind::Flat,
        };
        let mut out = Vec::new();
        run_solve(&a, &mut out).expect("solve must succeed on a river spot");
        let s = String::from_utf8(out).expect("solve output must be UTF-8");
        let v: Value = serde_json::from_str(&s).expect("solve output must parse as JSON");
        assert!(v.get("input").is_some(), "missing input: {v}");
        assert!(v.get("result").is_some(), "missing result: {v}");
        assert!(
            v.get("solver_version").is_some(),
            "missing solver_version: {v}"
        );
        let result = v.get("result").unwrap();
        for k in [
            "action_frequencies",
            "ev_per_action",
            "hero_equity",
            "exploitability",
            "iterations",
            "compute_ms",
        ] {
            assert!(result.get(k).is_some(), "result missing {k}: {result}");
        }
    }

    #[test]
    fn run_solve_produces_json_with_positive_stack() {
        // Regression test for the A47+ bug (fixed in A58): a bare
        // `Action::AllIn` in the river action log caused
        // `ActionLog::pot_contributions_on` to return (0, 0), which made
        // `legal_river_actions` re-enter the "no aggression yet" branch
        // and emit another `{Check, Bet, AllIn}` — producing an
        // unbounded tree and > 30 GB RSS OOM.
        //
        // The fix in `NlheSubgame::apply` translates `AllIn` into a
        // concrete `Bet(stack_start)` or `Raise(stack_start)` before
        // pushing it to the log, which bounds the river tree.
        //
        // This test runs a non-trivial river spot with `stack > 0` to
        // guard against regressions: the old behaviour failed to
        // terminate; the fixed behaviour returns valid JSON in well
        // under a second.
        let a = SolveArgs {
            board_raw: "AhKhQhJhTh".to_string(),
            hero_range_raw: "AsKs".to_string(),
            villain_range_raw: "AdKd".to_string(),
            pot: 100,
            stack: 500,
            iterations: 50,
            bet_tree: "default".to_string(),
            solver: SolverKind::Flat,
        };
        let mut out = Vec::new();
        run_solve(&a, &mut out).expect("solve must succeed on a stack>0 river spot");
        let s = String::from_utf8(out).expect("solve output must be UTF-8");
        let v: Value = serde_json::from_str(&s).expect("solve output must parse as JSON");
        let result = v.get("result").expect("result block missing");
        let freqs = result
            .get("action_frequencies")
            .and_then(|f| f.as_object())
            .expect("action_frequencies should be a JSON object");
        // Stack > 0 means the opener has real choices — Check, at least
        // one Bet sizing, and AllIn. The tree must have actually expanded
        // (rather than collapsing to Check/Check) for this test to be a
        // meaningful regression guard.
        assert!(
            freqs.len() >= 3,
            "expected multiple root actions with stack > 0, got {}: {:?}",
            freqs.len(),
            freqs
        );
        assert!(
            freqs.contains_key("allin"),
            "root should include allin when stack > 0: {:?}",
            freqs
        );
    }

    #[test]
    fn parse_inputs_accepts_multiword_ranges_with_whitespace() {
        let a = args_for("AhKh2s", "  AA ,\tKK ,AKs ", " 22+ , AJs+ , KQs ");
        let p = parse_inputs(&a).unwrap();
        assert!(p.hero.total_weight() > 0.0);
        assert!(p.villain.total_weight() > 0.0);
    }

    #[test]
    fn solver_kind_parses_known_values() {
        assert_eq!(SolverKind::parse("flat").unwrap(), SolverKind::Flat);
        assert_eq!(SolverKind::parse("classic").unwrap(), SolverKind::Classic);
    }

    #[test]
    fn solver_kind_rejects_unknown_values() {
        let err = SolverKind::parse("mystery").unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("solver"), "err: {err}");
        assert!(msg.contains("mystery"), "err: {err}");
    }

    /// Both `--solver flat` and `--solver classic` must produce a valid
    /// JSON document on the degenerate river spot. This is the smoke
    /// test that the classic escape hatch still works after A64's
    /// default-swap.
    #[test]
    fn run_solve_classic_solver_still_works() {
        let a = SolveArgs {
            board_raw: "2c7d9hTsJs".to_string(),
            hero_range_raw: "AhKh".to_string(),
            villain_range_raw: "AsAd".to_string(),
            pot: 100,
            stack: 0,
            iterations: 10,
            bet_tree: "default".to_string(),
            solver: SolverKind::Classic,
        };
        let mut out = Vec::new();
        run_solve(&a, &mut out).expect("solve must succeed under --solver classic");
        let s = String::from_utf8(out).expect("solve output must be UTF-8");
        let v: Value = serde_json::from_str(&s).expect("solve output must parse as JSON");
        assert!(v.get("result").is_some(), "missing result: {v}");
    }
}
