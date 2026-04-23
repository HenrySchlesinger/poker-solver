//! Integration tests for the `solver-cli` binary.
//!
//! These exercise the CLI through the built binary: `cargo test` builds
//! `target/.../solver-cli` (via `env!("CARGO_BIN_EXE_solver-cli")`), and
//! we shell out to it. This catches regressions in the argument-parsing
//! layer and the stdout JSON shape that unit tests on `solve_cmd::*`
//! can't.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_solver-cli"))
}

#[test]
fn help_shows_all_three_subcommands() {
    let output = bin().arg("--help").output().expect("run solver-cli --help");
    assert!(output.status.success(), "--help must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // All three subcommands advertised in the help text.
    assert!(stdout.contains("solve"), "help missing `solve`: {stdout}");
    assert!(
        stdout.contains("validate"),
        "help missing `validate`: {stdout}"
    );
    assert!(
        stdout.contains("precompute"),
        "help missing `precompute`: {stdout}"
    );
}

#[test]
fn solve_help_shows_expected_args() {
    let output = bin()
        .args(["solve", "--help"])
        .output()
        .expect("run solver-cli solve --help");
    assert!(output.status.success(), "solve --help must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for flag in [
        "--board",
        "--hero-range",
        "--villain-range",
        "--pot",
        "--stack",
        "--iterations",
        "--bet-tree",
    ] {
        assert!(
            stdout.contains(flag),
            "solve --help missing {flag}: {stdout}"
        );
    }
}

#[test]
fn solve_with_bad_board_exits_nonzero_with_useful_error() {
    let output = bin()
        .args([
            "solve",
            "--board",
            "XxXxXx",
            "--hero-range",
            "AA",
            "--villain-range",
            "KK",
        ])
        .output()
        .expect("run solver-cli solve");
    assert!(
        !output.status.success(),
        "expected non-zero exit for bad board"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.to_lowercase().contains("invalid board"),
        "stderr missing 'invalid board': {stderr}"
    );
}

#[test]
fn validate_is_scaffolded_not_yet_implemented() {
    let output = bin()
        .args(["validate", "--spot", "/does/not/exist.json"])
        .output()
        .expect("run solver-cli validate");
    assert!(
        !output.status.success(),
        "expected non-zero exit for not-yet-implemented validate"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not-yet-implemented") || stderr.contains("not yet"),
        "stderr missing scaffolded-marker: {stderr}"
    );
}

#[test]
fn precompute_is_scaffolded_not_yet_implemented() {
    let output = bin()
        .args([
            "precompute",
            "--grid",
            "/does/not/exist.json",
            "--output",
            "/tmp/out",
        ])
        .output()
        .expect("run solver-cli precompute");
    assert!(
        !output.status.success(),
        "expected non-zero exit for not-yet-implemented precompute"
    );
}

/// When the upstream `NlheSubgame::new` lands, remove the `#[ignore]` and
/// this test becomes the "produces valid JSON" integration guarantee.
///
/// Today `build_subgame` intentionally returns a blocked-upstream error,
/// so the solve fails before printing JSON. Rather than assert a
/// failure-mode that will flip once upstream lands, we ignore the test
/// with a clear note — it'll start running automatically once the
/// blocking guard is removed and `cargo test -- --include-ignored`
/// catches it in CI.
#[test]
#[ignore = "unblocks when NlheSubgame::new lands (Day 2 main path)"]
fn solve_produces_valid_json_end_to_end() {
    let output = bin()
        .args([
            "solve",
            "--board",
            "AhKh2s",
            "--hero-range",
            "AA,KK,AKs",
            "--villain-range",
            "22+,AJs+,KQs",
            "--pot",
            "100",
            "--stack",
            "1000",
            "--iterations",
            "10", // small iteration count so the test is fast
        ])
        .output()
        .expect("run solver-cli solve");
    assert!(
        output.status.success(),
        "solve should succeed once upstream lands; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The stdout must be valid JSON with the expected top-level shape.
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("solve stdout must parse as JSON");
    assert!(v.get("input").is_some(), "JSON missing `input`: {v}");
    assert!(v.get("result").is_some(), "JSON missing `result`: {v}");
    assert!(
        v.get("solver_version").is_some(),
        "JSON missing `solver_version`: {v}"
    );
    let result = v.get("result").unwrap();
    for key in [
        "action_frequencies",
        "ev_per_action",
        "hero_equity",
        "exploitability",
        "iterations",
        "compute_ms",
    ] {
        assert!(
            result.get(key).is_some(),
            "result missing `{key}`: {result}"
        );
    }
}
