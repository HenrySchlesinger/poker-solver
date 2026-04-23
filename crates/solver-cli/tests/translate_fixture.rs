//! Integration tests for `solver-cli translate-fixture`.
//!
//! These shell out to the compiled CLI binary (via
//! `env!("CARGO_BIN_EXE_solver-cli")`) with fixtures written to tempfiles,
//! then parse the emitted TexasSolver config and assert field-level
//! correctness. This catches:
//!
//! 1. Argument-parsing regressions (the `--help` output, flag names).
//! 2. The fixture → `.tsconfig` translation for the three street types.
//! 3. Error paths: malformed JSON, unknown bet-tree preset, bad street.
//!
//! We construct fixtures inline rather than reading from
//! `crates/solver-cli/tests/fixtures/` because:
//!   (a) those files are owned by Agent A15 and may not have landed yet
//!       when this test runs.
//!   (b) we want the assertions to be self-contained and tightly coupled
//!       to the fixture shape they're checking, without drifting if A15
//!       changes the canonical fixture contents.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The compiled `solver-cli` binary path.
fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_solver-cli"))
}

/// Write `fixture` to a temp file under the OS temp dir and return the path.
/// Uses the test name + a counter to avoid collisions across parallel tests.
fn write_fixture(fixture: &Value, stem: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("poker-solver-a20-tests");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(format!("{stem}_{}.json", std::process::id()));
    let mut f = std::fs::File::create(&path).expect("create fixture tempfile");
    f.write_all(serde_json::to_string_pretty(fixture).unwrap().as_bytes())
        .expect("write fixture");
    path
}

/// Translate a fixture and return the emitted config as a string.
/// Panics if the CLI exits non-zero (for the success-path tests).
fn translate(fixture: &Value, stem: &str) -> String {
    let in_path = write_fixture(fixture, stem);
    let out_path = in_path.with_extension("tsconfig");
    let output = bin()
        .args(["translate-fixture", "--input"])
        .arg(&in_path)
        .args(["--output"])
        .arg(&out_path)
        .args(["--dump-path", "/tmp/ts_dump.json"])
        .output()
        .expect("spawn solver-cli translate-fixture");
    assert!(
        output.status.success(),
        "translate-fixture failed: stderr = {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::read_to_string(&out_path).expect("read emitted config")
}

// ---------------------------------------------------------------------------
// Fixture builders
// ---------------------------------------------------------------------------

/// A flop fixture matching SCHEMA.md's example spot.
fn flop_fixture() -> Value {
    json!({
        "id": "spot_flop_test",
        "name": "Dry AK-high flop, BB vs BTN SRP",
        "description": "Narrow broadway-heavy SRP c-bet study spot.",
        "street": "flop",
        "input": {
            "board": "AhKd2c",
            "hero_range": "AA, KK, AKs, AKo, QQ",
            "villain_range": "77, 88, 99, TT, AQs, AQo",
            "pot": 60,
            "effective_stack": 970,
            "to_act": "hero",
            "bet_tree": "default_v0_1"
        },
        "iterations": 1000,
        "tolerances": { "action_freq_abs": 0.05, "ev_bb_abs": 0.10 },
        "expected_reference": "texassolver",
        "expected_notes": "placeholder"
    })
}

/// A river fixture: 5-card board, river-only bet tree.
fn river_fixture() -> Value {
    json!({
        "id": "spot_river_test",
        "name": "Paired river, value-heavy IP range",
        "description": "Wet run-out on a monotone-broken river. TexasSolver \
                        handles 5-card boards as a one-street subgame.",
        "street": "river",
        "input": {
            "board": "AhKd2cQc4d",
            "hero_range": "AA, KK",
            "villain_range": "QQ, JJ, TT",
            "pot": 200,
            "effective_stack": 800,
            "to_act": "villain",
            "bet_tree": "default_v0_1"
        },
        "iterations": 500,
        "tolerances": { "action_freq_abs": 0.05, "ev_bb_abs": 0.10 },
        "expected_reference": "texassolver",
        "expected_notes": "placeholder"
    })
}

/// A 3-bet-pot flop fixture: narrow ranges, deeper SPR implied by a
/// smaller effective stack relative to pot.
fn three_bet_flop_fixture() -> Value {
    json!({
        "id": "spot_3bp_test",
        "name": "3-bet pot flop, SRP-style c-bet",
        "description": "3BP: narrow hero (PFR), narrow villain (3-better). \
                        Pot is bigger than the SRP example, stack is \
                        correspondingly smaller.",
        "street": "flop",
        "input": {
            "board": "QhJs9d",
            "hero_range": "AA, KK, QQ, JJ, AKs, AKo",
            "villain_range": "TT, 99, AKs, KQs",
            "pot": 180,
            "effective_stack": 820,
            "to_act": "hero",
            "bet_tree": "default_v0_1"
        },
        "iterations": 1000,
        "tolerances": { "action_freq_abs": 0.05, "ev_bb_abs": 0.10 },
        "expected_reference": "texassolver",
        "expected_notes": "placeholder"
    })
}

/// Edge case: fixture with an "empty" super-tight villain range.
/// TexasSolver accepts any range string the parser handles; we just
/// exercise the translator's passthrough here.
fn empty_villain_fixture() -> Value {
    json!({
        "id": "spot_empty_villain",
        "name": "Tournament super-tight villain",
        "description": "Edge case: villain range has only AA. Translator \
                        should pass this through verbatim without trying to \
                        second-guess the range parser.",
        "street": "flop",
        "input": {
            "board": "2c3d4h",
            "hero_range": "AA, KK, QQ, JJ, TT, 99, 88, 77",
            "villain_range": "AA",
            "pot": 40,
            "effective_stack": 1000,
            "to_act": "hero",
            "bet_tree": "default_v0_1"
        },
        "iterations": 500,
        "tolerances": { "action_freq_abs": 0.05, "ev_bb_abs": 0.10 },
        "expected_reference": "texassolver",
        "expected_notes": "placeholder"
    })
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

/// Parse a TexasSolver config into a map from command → argument string.
/// Duplicate commands (like multiple `set_bet_sizes`) are collected as
/// a Vec under the same key.
fn parse_ts_config(cfg: &str) -> std::collections::HashMap<String, Vec<String>> {
    let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for line in cfg.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // command is the first whitespace-delimited token; args is the
        // rest of the line.
        let (cmd, rest) = match line.split_once(char::is_whitespace) {
            Some((c, r)) => (c.to_string(), r.trim().to_string()),
            None => (line.to_string(), String::new()),
        };
        out.entry(cmd).or_default().push(rest);
    }
    out
}

fn assert_has_line(cfg: &str, needle: &str) {
    assert!(
        cfg.lines().any(|l| l.trim() == needle),
        "config does not contain expected line {:?}\nconfig:\n{}",
        needle,
        cfg
    );
}

fn assert_no_line_starts_with(cfg: &str, prefix: &str) {
    for line in cfg.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            continue;
        }
        assert!(
            !t.starts_with(prefix),
            "config contained unexpected line starting with {:?}: {:?}\nconfig:\n{}",
            prefix,
            t,
            cfg
        );
    }
}

// ---------------------------------------------------------------------------
// Success-path tests (one per street + one 3-bet pot)
// ---------------------------------------------------------------------------

#[test]
fn translate_flop_fixture_emits_all_streets() {
    let cfg = translate(&flop_fixture(), "flop");
    let parsed = parse_ts_config(&cfg);

    // Basics.
    assert_eq!(parsed.get("set_pot").unwrap()[0], "60");
    assert_eq!(parsed.get("set_effective_stack").unwrap()[0], "970");
    assert_eq!(parsed.get("set_board").unwrap()[0], "Ah,Kd,2c");
    // hero=OOP convention. Ranges are whitespace-stripped before being
    // emitted because TexasSolver's config parser rejects lines with
    // more than two space-separated tokens — so `"AA, KK"` round-trips
    // to `"AA,KK"`.
    assert_eq!(parsed.get("set_range_oop").unwrap()[0], "AA,KK,AKs,AKo,QQ");
    assert_eq!(
        parsed.get("set_range_ip").unwrap()[0],
        "77,88,99,TT,AQs,AQo"
    );

    // Full bet-size coverage: 6 lines per street × 3 streets = 18 total.
    let bet_size_lines = parsed.get("set_bet_sizes").unwrap();
    assert_eq!(
        bet_size_lines.len(),
        18,
        "expected 18 set_bet_sizes lines (3 streets × 6 lines each), got {}: {:?}",
        bet_size_lines.len(),
        bet_size_lines
    );

    // Specific sizings match the default_v0_1 preset.
    assert_has_line(&cfg, "set_bet_sizes oop,flop,bet,33,66,100");
    assert_has_line(&cfg, "set_bet_sizes ip,flop,bet,33,66,100");
    assert_has_line(&cfg, "set_bet_sizes oop,turn,bet,50,100,200");
    assert_has_line(&cfg, "set_bet_sizes ip,turn,bet,50,100,200");
    assert_has_line(&cfg, "set_bet_sizes oop,river,bet,33,66,100,200");
    assert_has_line(&cfg, "set_bet_sizes ip,river,bet,33,66,100,200");

    // All-in lines for each street.
    assert_has_line(&cfg, "set_bet_sizes oop,flop,allin");
    assert_has_line(&cfg, "set_bet_sizes ip,river,allin");

    // Global solver knobs.
    assert_eq!(parsed.get("set_allin_threshold").unwrap()[0], "0.67");
    assert_eq!(parsed.get("set_max_iteration").unwrap()[0], "1000");
    assert_eq!(parsed.get("set_use_isomorphism").unwrap()[0], "1");
    assert!(parsed.contains_key("build_tree"));
    assert!(parsed.contains_key("start_solve"));
    assert_eq!(parsed.get("dump_result").unwrap()[0], "/tmp/ts_dump.json");
}

#[test]
fn translate_river_fixture_emits_river_only() {
    let cfg = translate(&river_fixture(), "river");
    let parsed = parse_ts_config(&cfg);

    assert_eq!(parsed.get("set_pot").unwrap()[0], "200");
    assert_eq!(parsed.get("set_effective_stack").unwrap()[0], "800");
    assert_eq!(parsed.get("set_board").unwrap()[0], "Ah,Kd,2c,Qc,4d");
    // Whitespace stripped; see note in flop-fixture test above.
    assert_eq!(parsed.get("set_range_oop").unwrap()[0], "AA,KK");
    assert_eq!(parsed.get("set_range_ip").unwrap()[0], "QQ,JJ,TT");

    // Only 6 set_bet_sizes lines — just the river.
    let bet_size_lines = parsed.get("set_bet_sizes").unwrap();
    assert_eq!(
        bet_size_lines.len(),
        6,
        "river fixture should emit only 6 bet-size lines, got {}: {:?}",
        bet_size_lines.len(),
        bet_size_lines
    );
    assert_no_line_starts_with(&cfg, "set_bet_sizes oop,flop");
    assert_no_line_starts_with(&cfg, "set_bet_sizes ip,flop");
    assert_no_line_starts_with(&cfg, "set_bet_sizes oop,turn");
    assert_no_line_starts_with(&cfg, "set_bet_sizes ip,turn");
    assert_has_line(&cfg, "set_bet_sizes oop,river,bet,33,66,100,200");
    assert_has_line(&cfg, "set_bet_sizes ip,river,bet,33,66,100,200");

    assert_eq!(parsed.get("set_max_iteration").unwrap()[0], "500");
}

#[test]
fn translate_3bet_pot_flop_fixture_preserves_fields() {
    let cfg = translate(&three_bet_flop_fixture(), "3bp");
    let parsed = parse_ts_config(&cfg);
    assert_eq!(parsed.get("set_pot").unwrap()[0], "180");
    assert_eq!(parsed.get("set_effective_stack").unwrap()[0], "820");
    assert_eq!(parsed.get("set_board").unwrap()[0], "Qh,Js,9d");
    // All three street's bet-size lines should be present.
    let bet_size_lines = parsed.get("set_bet_sizes").unwrap();
    assert_eq!(bet_size_lines.len(), 18);
}

#[test]
fn translate_with_empty_villain_range_passes_through() {
    let cfg = translate(&empty_villain_fixture(), "emptyv");
    let parsed = parse_ts_config(&cfg);
    // The super-tight villain range should survive verbatim — the
    // translator does not second-guess range syntax.
    assert_eq!(parsed.get("set_range_ip").unwrap()[0], "AA");
    // Whitespace stripped; see note in flop-fixture test above.
    assert_eq!(
        parsed.get("set_range_oop").unwrap()[0],
        "AA,KK,QQ,JJ,TT,99,88,77"
    );
    assert_eq!(parsed.get("set_pot").unwrap()[0], "40");
    assert_eq!(parsed.get("set_effective_stack").unwrap()[0], "1000");
}

// ---------------------------------------------------------------------------
// Error-path tests
// ---------------------------------------------------------------------------

#[test]
fn help_shows_translate_fixture_subcommand() {
    let output = bin().arg("--help").output().expect("run solver-cli --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("translate-fixture"),
        "--help missing `translate-fixture`:\n{stdout}"
    );
}

#[test]
fn translate_fixture_help_shows_flags() {
    let output = bin()
        .args(["translate-fixture", "--help"])
        .output()
        .expect("run solver-cli translate-fixture --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for flag in ["--input", "--output", "--format", "--dump-path"] {
        assert!(
            stdout.contains(flag),
            "translate-fixture --help missing {flag}:\n{stdout}"
        );
    }
}

#[test]
fn malformed_json_fails_with_useful_message() {
    let dir = std::env::temp_dir().join("poker-solver-a20-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let in_path = dir.join(format!("malformed_{}.json", std::process::id()));
    std::fs::write(&in_path, "{ this is not valid json").expect("write malformed fixture");
    let out_path = dir.join(format!("malformed_{}.tsconfig", std::process::id()));

    let output = bin()
        .args(["translate-fixture", "--input"])
        .arg(&in_path)
        .arg("--output")
        .arg(&out_path)
        .output()
        .expect("spawn solver-cli");
    assert!(
        !output.status.success(),
        "expected non-zero exit for malformed JSON"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    assert!(
        stderr.contains("parse") || stderr.contains("json"),
        "stderr should mention parse/JSON failure: {stderr}"
    );
}

#[test]
fn unknown_bet_tree_preset_fails_cleanly() {
    let mut fix = flop_fixture();
    fix["input"]["bet_tree"] = json!("experimental_v2");
    let in_path = write_fixture(&fix, "badtree");
    let out_path = in_path.with_extension("tsconfig");

    let output = bin()
        .args(["translate-fixture", "--input"])
        .arg(&in_path)
        .arg("--output")
        .arg(&out_path)
        .output()
        .expect("spawn solver-cli");
    assert!(
        !output.status.success(),
        "expected non-zero exit for unknown bet_tree"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bet_tree") && stderr.contains("experimental_v2"),
        "stderr should name the offending preset: {stderr}"
    );
}

#[test]
fn unknown_format_fails_cleanly() {
    let fix = flop_fixture();
    let in_path = write_fixture(&fix, "badfmt");
    let out_path = in_path.with_extension("tsconfig");

    let output = bin()
        .args(["translate-fixture", "--input"])
        .arg(&in_path)
        .arg("--output")
        .arg(&out_path)
        .args(["--format", "piosolver"])
        .output()
        .expect("spawn solver-cli");
    assert!(
        !output.status.success(),
        "expected non-zero exit for unknown format"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("format") || stderr.contains("piosolver"),
        "stderr should mention the offending format: {stderr}"
    );
}

#[test]
fn output_dash_writes_to_stdout() {
    let fix = flop_fixture();
    let in_path = write_fixture(&fix, "stdout");

    let child = bin()
        .args(["translate-fixture", "--input"])
        .arg(&in_path)
        .args(["--output", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn solver-cli");
    let output = child.wait_with_output().expect("wait_with_output");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("set_pot 60"),
        "stdout should contain the emitted config: {stdout}"
    );
}
