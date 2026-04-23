//! Differential test harness: our solver vs TexasSolver.
//!
//! For each `spot_NNN.json` fixture in `tests/fixtures/`, this test:
//!
//! 1. Runs our solver via `solver-cli solve ...` (built by cargo via
//!    `CARGO_BIN_EXE_solver-cli`).
//! 2. Runs TexasSolver via `./bin/texassolver -i <fixture.tsconfig>`, where
//!    the `.tsconfig` is produced by `scripts/translate_fixture.py`.
//! 3. Parses both outputs into a uniform `ActionSummary` (per-action
//!    frequency at the root + EV per action).
//! 4. Asserts that per-action frequency deltas are ≤ the fixture's
//!    `tolerances.action_freq_abs` (5%, as a default for v0.1) and EV
//!    deltas are ≤ `tolerances.ev_bb_abs` (0.1 bb).
//!
//! ## `#[ignore]` by default
//!
//! This test is heavy: it requires a built `./bin/texassolver` binary,
//! Python 3 for the translator, and many seconds-to-minutes of CPU per
//! fixture. CI does NOT run it. Invoke explicitly:
//!
//! ```text
//! cargo test -p solver-cli --test texassolver_diff -- --ignored
//! ```
//!
//! ## Legal: why calling TexasSolver from tests is okay
//!
//! TexasSolver is AGPL-3.0. We invoke its **binary** as a black-box test
//! oracle — we don't link against its source, we don't ship its binary
//! with Poker Panel. See `docs/DIFFERENTIAL_TESTING.md` for the full
//! legal reasoning. **Do NOT vendor TexasSolver source into this repo.**
//!
//! ## Scaffolding state (Day 6)
//!
//! The test harness code below is written so it **compiles today** even
//! though (a) our solver doesn't yet produce non-zero `action_frequencies`
//! (Day 2 upstream work), (b) A15's twenty fixtures are still being
//! written. The `#[ignore]` keeps CI green. Once both upstream pieces
//! land, just drop the ignore and the harness is live.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Repository root (the `poker-solver/` directory).
fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` = `<repo>/crates/solver-cli`. Climb two parents.
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| panic!("unexpected crate layout: {crate_dir:?}"))
}

/// `tests/fixtures/` — shared with `fixtures_parse.rs` (A15).
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// The TexasSolver binary. Build via `scripts/install-texassolver.sh`.
fn texassolver_bin() -> PathBuf {
    repo_root().join("bin").join("texassolver")
}

/// TexasSolver's runtime resource dir (hand tables, sample files).
fn texassolver_resources() -> PathBuf {
    repo_root().join("bin").join("resources")
}

/// The translator.
fn translator_script() -> PathBuf {
    repo_root().join("scripts").join("translate_fixture.py")
}

/// Our solver binary (built by cargo via the CARGO_BIN_EXE_* env var).
fn our_solver_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_solver-cli"))
}

// ---------------------------------------------------------------------------
// Preflight
// ---------------------------------------------------------------------------

enum PreflightStatus {
    Ready,
    Skip(String),
}

/// Check all external prerequisites. Returns a skip message if something's
/// missing rather than panicking — lets `--ignored` runs report a clean
/// "skipped because X" instead of a confusing crash.
fn preflight() -> PreflightStatus {
    let ts = texassolver_bin();
    if !ts.exists() {
        return PreflightStatus::Skip(format!(
            "TexasSolver binary not found at {ts:?}. \
             Run ./scripts/install-texassolver.sh first."
        ));
    }
    if !ts.is_file() {
        return PreflightStatus::Skip(format!("{ts:?} exists but is not a file"));
    }

    let resources = texassolver_resources();
    if !resources.is_dir() {
        return PreflightStatus::Skip(format!(
            "TexasSolver resources dir missing: {resources:?}. \
             Re-run ./scripts/install-texassolver.sh."
        ));
    }

    let tx = translator_script();
    if !tx.exists() {
        return PreflightStatus::Skip(format!("translator script missing: {tx:?}"));
    }

    // `python3` must be on PATH. macOS ships Python 3; Colab too.
    let python_ok = Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !python_ok {
        return PreflightStatus::Skip(
            "python3 not on PATH; install Python 3 to run the translator".into(),
        );
    }

    PreflightStatus::Ready
}

// ---------------------------------------------------------------------------
// Shared output shape
// ---------------------------------------------------------------------------

/// Per-action root summary, normalized across solvers.
///
/// Keys are action names. For a v0.1 heads-up flop spot these are typically
/// one of `check`, `call`, `fold`, `bet_33`, `bet_66`, `bet_pot`, `allin`
/// from our side; on the TexasSolver side they come through as labels like
/// `"CHECK"`, `"BET 20.0"`, `"CALL"`. We normalize to lowercase + strip the
/// numeric chip count (preserving ratio-to-pot as the label suffix).
#[derive(Debug, Default, Clone)]
struct ActionSummary {
    frequency: BTreeMap<String, f64>,
    ev: BTreeMap<String, f64>,
}

impl ActionSummary {
    fn action_names(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        out.extend(self.frequency.keys().cloned());
        out.extend(self.ev.keys().cloned());
        out
    }
}

// ---------------------------------------------------------------------------
// Fixture reading
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct FixtureEnvelope {
    path: PathBuf,
    raw: Value,
    tolerance_freq: f64,
    tolerance_ev_bb: f64,
    iterations: u32,
    pot_chips: u32,
    chips_per_bb: f64,
}

fn read_fixture(path: &Path) -> FixtureEnvelope {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read fixture {path:?}: {e}"));
    let raw: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse fixture {path:?}: {e}"));

    let tolerances = raw
        .get("tolerances")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{path:?}: missing tolerances object"));
    let tolerance_freq = tolerances
        .get("action_freq_abs")
        .and_then(Value::as_f64)
        .unwrap_or_else(|| panic!("{path:?}: missing tolerances.action_freq_abs"));
    let tolerance_ev_bb = tolerances
        .get("ev_bb_abs")
        .and_then(Value::as_f64)
        .unwrap_or_else(|| panic!("{path:?}: missing tolerances.ev_bb_abs"));

    let iterations = raw
        .get("iterations")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{path:?}: missing iterations")) as u32;

    let input = raw
        .get("input")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{path:?}: missing input object"));
    let pot_chips = input
        .get("pot")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{path:?}: missing input.pot")) as u32;

    // Schema convention: 1 bb = 10 chips. See SCHEMA.md.
    let chips_per_bb = 10.0;

    FixtureEnvelope {
        path: path.to_path_buf(),
        raw,
        tolerance_freq,
        tolerance_ev_bb,
        iterations,
        pot_chips,
        chips_per_bb,
    }
}

// ---------------------------------------------------------------------------
// Our solver runner
// ---------------------------------------------------------------------------

/// Runs `solver-cli solve ...` with the fields translated from the fixture,
/// parses the JSON response, returns an `ActionSummary`.
fn run_our_solver(env: &FixtureEnvelope) -> Result<ActionSummary, String> {
    let input = env
        .raw
        .get("input")
        .and_then(Value::as_object)
        .ok_or("fixture.input not an object")?;

    let board = input
        .get("board")
        .and_then(Value::as_str)
        .ok_or("missing input.board")?
        .to_string();
    let hero_range = input
        .get("hero_range")
        .and_then(Value::as_str)
        .ok_or("missing input.hero_range")?
        .to_string();
    let villain_range = input
        .get("villain_range")
        .and_then(Value::as_str)
        .ok_or("missing input.villain_range")?
        .to_string();
    let pot = input
        .get("pot")
        .and_then(Value::as_u64)
        .ok_or("missing input.pot")? as u32;
    let stack = input
        .get("effective_stack")
        .and_then(Value::as_u64)
        .ok_or("missing input.effective_stack")? as u32;
    let bet_tree = input
        .get("bet_tree")
        .and_then(Value::as_str)
        .ok_or("missing input.bet_tree")?
        .to_string();

    let mut cmd = Command::new(our_solver_bin());
    cmd.args([
        "solve",
        "--board",
        &board,
        "--hero-range",
        &hero_range,
        "--villain-range",
        &villain_range,
        "--pot",
        &pot.to_string(),
        "--stack",
        &stack.to_string(),
        "--iterations",
        &env.iterations.to_string(),
        "--bet-tree",
        &bet_tree,
    ]);
    let out = cmd.output().map_err(|e| format!("spawn solver-cli: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "solver-cli solve failed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let stdout = String::from_utf8(out.stdout).map_err(|e| format!("utf8: {e}"))?;
    let v: Value = serde_json::from_str(&stdout).map_err(|e| format!("parse our json: {e}"))?;

    let result = v
        .get("result")
        .and_then(Value::as_object)
        .ok_or("our solver JSON missing `result` object")?;

    let mut summary = ActionSummary::default();

    if let Some(freq) = result.get("action_frequencies").and_then(Value::as_object) {
        for (k, v) in freq {
            if let Some(f) = v.as_f64() {
                summary.frequency.insert(normalize_action(k), f);
            }
        }
    }
    if let Some(ev) = result.get("ev_per_action").and_then(Value::as_object) {
        for (k, v) in ev {
            if let Some(f) = v.as_f64() {
                summary.ev.insert(normalize_action(k), f);
            }
        }
    }

    Ok(summary)
}

// ---------------------------------------------------------------------------
// TexasSolver runner
// ---------------------------------------------------------------------------

/// Runs the translator + TexasSolver binary for a fixture, parses the
/// resulting `output_result.json`, returns an `ActionSummary`.
fn run_texassolver(env: &FixtureEnvelope, work_dir: &Path) -> Result<ActionSummary, String> {
    // 1. Translate our fixture to a TexasSolver config file.
    let cfg_path = work_dir.join("input.tsconfig");
    let dump_path = work_dir.join("ts_out.json");

    let tx_out = Command::new("python3")
        .arg(translator_script())
        .arg(&env.path)
        .arg("--dump")
        .arg(&dump_path)
        .arg("-o")
        .arg(&cfg_path)
        .output()
        .map_err(|e| format!("spawn translate_fixture.py: {e}"))?;
    if !tx_out.status.success() {
        return Err(format!(
            "translate_fixture.py failed: stderr={}",
            String::from_utf8_lossy(&tx_out.stderr)
        ));
    }

    // 2. Invoke TexasSolver.
    let ts_out = Command::new(texassolver_bin())
        .arg("-i")
        .arg(&cfg_path)
        .arg("--resource_dir")
        .arg(texassolver_resources())
        .current_dir(work_dir)
        .output()
        .map_err(|e| format!("spawn texassolver: {e}"))?;
    if !ts_out.status.success() {
        return Err(format!(
            "texassolver exit={:?} stderr={}",
            ts_out.status.code(),
            String::from_utf8_lossy(&ts_out.stderr)
        ));
    }

    // 3. Parse TexasSolver's `output_result.json`.
    //
    // Shape (observed on the sample fixture):
    //   {
    //     "node_type": "action_node",
    //     "childrens": {
    //       "CHECK":   {..., "strategy": {"strategy": {"AsAh": [fa, fb, ...], ...}}},
    //       "BET 20.0":{...},
    //       "BET 40.0":{...},
    //       ...
    //     }
    //   }
    //
    // "CHECK" / "BET X" / "CALL" / "RAISE X" / "FOLD" are the action keys.
    // Per-combo frequencies live under childrens.<action>.strategy.strategy.
    // To get the ROOT action frequency we average across combos (weighted
    // by the hero's range weights would be ideal; for v0.1 a uniform
    // average over enumerated combos is an acceptable approximation — the
    // same simplification our SolveSummary makes).
    let out_text = fs::read_to_string(&dump_path)
        .map_err(|e| format!("read texassolver output {dump_path:?}: {e}"))?;
    let out_json: Value =
        serde_json::from_str(&out_text).map_err(|e| format!("parse texassolver json: {e}"))?;

    let childrens = out_json
        .get("childrens")
        .and_then(Value::as_object)
        .ok_or("texassolver output missing `childrens`")?;

    let mut summary = ActionSummary::default();

    for (action_label, child) in childrens {
        let normalized = normalize_ts_action(action_label, env);
        // Mean combo frequency for this root action.
        let combos = child
            .get("strategy")
            .and_then(|s| s.get("strategy"))
            .and_then(Value::as_object);
        if let Some(combos) = combos {
            // Each combo value is an array [f_this_action, ...] where we
            // want the probability of picking THIS action from the combo's
            // strategy vector. TexasSolver packs per-combo probabilities as
            // an array whose index matches the child-action order. For the
            // mean-over-combos rollup, we take index 0 — the child node's
            // slot — which, for the action we're currently enumerating,
            // IS the probability of that action.
            //
            // If the combo array has only one element we treat it as
            // already representing "probability of this action".
            let mut sum = 0.0f64;
            let mut n = 0u64;
            for combo_freqs in combos.values() {
                if let Some(arr) = combo_freqs.as_array() {
                    if let Some(first) = arr.first().and_then(Value::as_f64) {
                        sum += first;
                        n += 1;
                    }
                }
            }
            let mean = if n > 0 { sum / (n as f64) } else { 0.0 };
            summary.frequency.insert(normalized, mean);
        }
    }

    // TexasSolver doesn't expose per-action EV directly in the strategy
    // dump — you get it from the log file (`ev` column in the per-iter
    // print). For v0.1 we skip EV comparison on TexasSolver's side; the
    // fixture will only assert frequency deltas when ev_per_action is
    // empty on one side. Future work: parse the log or use TS's
    // `exploitability` call. Tracking in docs/DIFFERENTIAL_TESTING.md.
    Ok(summary)
}

/// Canonicalize an action label from our solver.
///
/// Our `solve_cmd.rs` emits labels like `"check"`, `"bet_33"`, `"allin"`.
/// Those are already lowercase snake_case; pass through.
fn normalize_action(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// Canonicalize a TexasSolver action label.
///
/// TexasSolver emits:
///   - "CHECK" → "check"
///   - "CALL"  → "call"
///   - "FOLD"  → "fold"
///   - "BET 20.0"  → "bet_<pct>" where pct = bet / pot * 100, snapped to
///                   an integer. Pot is known from the fixture envelope.
///   - "RAISE 60.0" → "raise_<pct>"
///
/// For actions that don't fit a pattern we emit the original lowercased.
fn normalize_ts_action(raw: &str, env: &FixtureEnvelope) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    match parts.as_slice() {
        ["CHECK"] | ["check"] => "check".into(),
        ["CALL"] | ["call"] => "call".into(),
        ["FOLD"] | ["fold"] => "fold".into(),
        [verb, amount_str] => {
            let amount: f64 = amount_str.parse().unwrap_or(0.0);
            let pct = if env.pot_chips > 0 {
                (amount / (env.pot_chips as f64) * 100.0).round() as i64
            } else {
                0
            };
            format!("{}_{}", verb.to_ascii_lowercase(), pct)
        }
        _ => lower,
    }
}

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct DiffReport {
    failures: Vec<String>,
}

fn diff_spot(env: &FixtureEnvelope, ours: &ActionSummary, theirs: &ActionSummary) -> DiffReport {
    let mut failures = Vec::new();

    // Union of action names, so we flag an action that exists on one side
    // only (not the same thing as a frequency mismatch).
    let mut all = ours.action_names();
    all.extend(theirs.action_names());

    for action in &all {
        let ours_f = ours.frequency.get(action).copied().unwrap_or(0.0);
        let theirs_f = theirs.frequency.get(action).copied().unwrap_or(0.0);
        let delta = (ours_f - theirs_f).abs();
        if delta > env.tolerance_freq {
            failures.push(format!(
                "action {action:?}: frequency delta {delta:.4} > tolerance \
                 {:.4} (ours={ours_f:.4}, theirs={theirs_f:.4})",
                env.tolerance_freq
            ));
        }

        // EV comparison only when both sides provide a value. TexasSolver's
        // current JSON dump does not include EV — see run_texassolver's
        // TODO — so in practice only the `ours` side is populated today.
        let (Some(ours_ev), Some(theirs_ev)) = (ours.ev.get(action), theirs.ev.get(action)) else {
            continue;
        };

        // Our EV is in chips; fixture tolerance is in bb. Convert.
        let delta_chips = (ours_ev - theirs_ev).abs();
        let delta_bb = delta_chips / env.chips_per_bb;
        if delta_bb > env.tolerance_ev_bb {
            failures.push(format!(
                "action {action:?}: EV delta {delta_bb:.4} bb > tolerance \
                 {:.4} bb (ours={ours_ev:.2}, theirs={theirs_ev:.2}, \
                 chips_per_bb={})",
                env.tolerance_ev_bb, env.chips_per_bb
            ));
        }
    }

    let _ = env.path.file_name(); // keep env.path alive for future logging

    DiffReport { failures }
}

// ---------------------------------------------------------------------------
// Fixture enumeration
// ---------------------------------------------------------------------------

fn list_fixtures() -> Vec<PathBuf> {
    let dir = fixtures_dir();
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with("spot_") && name.ends_with(".json") {
            out.push(path);
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

/// The differential test. Guarded with `#[ignore]` so CI doesn't try to
/// build TexasSolver (which is AGPL-vendored externally, not present in
/// the test machine by default). Invoke explicitly:
///
/// ```text
/// cargo test -p solver-cli --test texassolver_diff -- --ignored
/// ```
#[test]
#[ignore = "heavyweight: requires ./bin/texassolver via install-texassolver.sh"]
fn texassolver_diff() {
    match preflight() {
        PreflightStatus::Ready => {}
        PreflightStatus::Skip(msg) => {
            eprintln!("texassolver_diff: skipping — {msg}");
            return;
        }
    }

    let fixtures = list_fixtures();
    if fixtures.is_empty() {
        eprintln!(
            "texassolver_diff: no fixtures in {:?} — skipping (A15's fixtures \
             haven't landed yet)",
            fixtures_dir()
        );
        return;
    }

    let tmp = tempdir_for("texassolver_diff");

    let mut failed_fixtures = Vec::new();

    for (i, fixture_path) in fixtures.iter().enumerate() {
        let env = read_fixture(fixture_path);
        let spot_tmp = tmp.join(format!("spot_{i:03}"));
        fs::create_dir_all(&spot_tmp).expect("create per-spot tmp dir");

        let ours = match run_our_solver(&env) {
            Ok(s) => s,
            Err(e) => {
                failed_fixtures.push(format!("{fixture_path:?}: our solver failed: {e}"));
                continue;
            }
        };
        let theirs = match run_texassolver(&env, &spot_tmp) {
            Ok(s) => s,
            Err(e) => {
                failed_fixtures.push(format!("{fixture_path:?}: texassolver failed: {e}"));
                continue;
            }
        };

        let report = diff_spot(&env, &ours, &theirs);
        if !report.failures.is_empty() {
            failed_fixtures.push(format!(
                "{fixture_path:?} ({} diffs):\n  - {}",
                report.failures.len(),
                report.failures.join("\n  - "),
            ));
        }
    }

    if !failed_fixtures.is_empty() {
        panic!(
            "differential test failed on {} fixture(s):\n\n{}",
            failed_fixtures.len(),
            failed_fixtures.join("\n\n"),
        );
    }
}

/// Create a fresh per-run temp directory under `$CARGO_TARGET_TMPDIR` (set
/// by cargo for integration tests) or `/tmp` as a fallback. Not deleted
/// on success — inspecting the generated `.tsconfig` and `ts_out.json`
/// is exactly what you want when debugging a divergence.
fn tempdir_for(label: &str) -> PathBuf {
    let base = std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = base.join(format!("{label}_{ts}"));
    fs::create_dir_all(&dir).expect("create tmp dir");
    dir
}
