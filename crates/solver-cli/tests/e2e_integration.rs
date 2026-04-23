//! End-to-end integration test for poker-solver v0.1.
//!
//! This is the "it all works together" gate: one test that exercises the
//! three consumption paths and asserts they agree.
//!
//!   1. `solver-cli solve ...` (shelled-out, JSON over stdout)
//!   2. `solver_ffi::solver_solve` (in-process FFI, the contract Poker
//!      Panel links against)
//!   3. The numeric agreement between the two paths is asserted.
//!
//! A third, Swift-side path is exercised by
//! `scripts/build_swift_harness.sh` + `target/swift-harness-e2e`, which
//! the outer `scripts/e2e.sh` orchestrator runs separately because
//! driving `swiftc` from `cargo test` is not worth the bootstrap pain.
//!
//! # Why `#[ignore]`
//!
//! The test is marked `#[ignore]` so it does not run under a plain
//! `cargo test` invocation. It needs the `solver-cli` release binary on
//! disk, which is a precondition `scripts/e2e.sh` sets up via
//! `cargo build --release --workspace` before invoking the test.
//!
//! Run it with:
//!
//! ```text
//! cargo test --release -p solver-cli --test e2e_integration \
//!     -- --ignored end_to_end
//! ```
//!
//! # Fail-loud discipline
//!
//! Per the A27 task brief: if the solver is not yet producing real
//! output, this test **must fail with a clear, actionable message** —
//! not silently skip. The assertions below are written so that a stub
//! (like today's `solver_solve` returning `InternalError`, or the CLI's
//! `build_subgame` returning "not yet implemented") produces a failure
//! that names the exact wiring gap.
//!
//! As of Day 2 (2026-04-23), both the CLI `solve` path and the FFI
//! `solver_solve` path are stubs. This test is expected to **fail**
//! until the main-path agent wires `NlheSubgame::new` into
//! `solver-cli::solve_cmd::build_subgame` and `solver-ffi::solver_solve`
//! dispatches into `solver_core::CfrPlus`. That is the whole point —
//! when v0.1 is ready to ship, this test turns green.

#![allow(clippy::approx_constant)]

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use solver_ffi::{HandState, SolveResult, SolverHandle, SolverStatus};

// ---------------------------------------------------------------------------
// Canonical spot
// ---------------------------------------------------------------------------
//
// Royal-flush-on-board with both players holding AK suited. Every
// dealt pair plays the five-card board, so the hero equity vs villain is
// exactly 0.5 (permanent tie). This is the most constrained spot we can
// express in the v0.1 range parser — "AsKs" (a single combo) is not yet
// supported; "AKs" (4 combos) is. The tie property survives either way:
// whichever suited-AK combinations are dealt, the playing hand is the
// board's royal flush.
//
// These constants are used verbatim by all three paths (CLI, FFI, Swift)
// so any numeric drift surfaces immediately.

const BOARD_STR: &str = "AhKhQhJhTh";
const HERO_RANGE_STR: &str = "AKs";
const VILLAIN_RANGE_STR: &str = "AKs";
const POT: u32 = 100;
const STACK: u32 = 500;
const ITERATIONS: u32 = 100;

/// The AKs combos that survive card-conflict pruning on a AhKhQhJhTh
/// board. AhKh and AsKs use board cards, so only AcKc and AdKd are
/// live — but even those "play the board" (royal flush on the board),
/// so hero equity must be exactly 0.5 no matter which combo is dealt.
const EXPECTED_HERO_EQUITY: f32 = 0.5;

/// Absolute tolerance for floating-point agreement between paths.
const EPS: f32 = 1e-6;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Locate the release binary produced by `cargo build --release -p solver-cli`.
///
/// We intentionally do *not* use `env!("CARGO_BIN_EXE_solver-cli")` — that
/// points at the *test* profile binary (usually under `target/debug/` for
/// `cargo test` and `target/release/` for `cargo test --release`). The
/// e2e script builds the whole workspace in release explicitly and we
/// want to exercise the same bits Poker Panel would ship, so we pin to
/// `target/release/solver-cli` relative to the workspace root.
fn solver_cli_release_path() -> PathBuf {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CARGO_MANIFEST_DIR has a parent (crates/)")
        .parent()
        .expect("crates/ has a parent (workspace root)")
        .to_path_buf();

    let bin = workspace_root
        .join("target")
        .join("release")
        .join("solver-cli");

    assert!(
        bin.exists(),
        "e2e test requires {} to exist — run `cargo build --release --workspace` first \
         (scripts/e2e.sh does this automatically)",
        bin.display()
    );
    bin
}

/// Wall-clock bound on the CLI subprocess during this test. If the CLI
/// takes longer than this — the typical sign is a runaway CFR walk
/// exhausting memory on the river spot — we kill the child and report a
/// blocked-upstream failure rather than letting the test hang for
/// hundreds of seconds. `cargo test` enforces no timeout on its own.
const CLI_TIMEOUT_SEC: u64 = 60;

/// Invoke the CLI with the canonical spot and return its raw Output, or a
/// synthetic "timed out" `Output` if the child didn't finish in
/// `CLI_TIMEOUT_SEC` seconds. The latter signals an upstream blocker and
/// is handled by `parse_cli_json` with a targeted error message.
fn run_cli_solve() -> Output {
    let bin = solver_cli_release_path();
    let mut child = Command::new(&bin)
        .args([
            "solve",
            "--board",
            BOARD_STR,
            "--hero-range",
            HERO_RANGE_STR,
            "--villain-range",
            VILLAIN_RANGE_STR,
            "--pot",
            &POT.to_string(),
            "--stack",
            &STACK.to_string(),
            "--iterations",
            &ITERATIONS.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", bin.display()));

    // Poll for completion up to CLI_TIMEOUT_SEC. If the child is still
    // running at the deadline, SIGKILL it and return a synthetic Output
    // with an exit code of 137 (SIGKILL) so the caller can distinguish
    // "solver took too long / OOM'd" from "solver errored cleanly".
    let deadline = Instant::now() + Duration::from_secs(CLI_TIMEOUT_SEC);
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                // Finished — drain its output.
                return child.wait_with_output().unwrap_or_else(|e| {
                    panic!("failed to read {}'s output after exit: {e}", bin.display())
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    // Return a stub Output indicating the killed state.
                    // We can't construct an Output directly from outside
                    // its crate, so run a trivially-failing process to
                    // produce one. `false` exits 1, then we override the
                    // stderr via a wrapper — but that's fragile. Cleaner:
                    // call `exit 137` via sh. macOS `sh -c 'exit 137'`
                    // returns an ExitStatus whose code is 137.
                    let synth = Command::new("sh")
                        .args(["-c", "exit 137"])
                        .output()
                        .expect("shell to produce Output");
                    return Output {
                        status: synth.status,
                        stdout: Vec::new(),
                        stderr: format!(
                            "solver-cli exceeded {CLI_TIMEOUT_SEC}s deadline \
                             on the canonical spot — test watchdog killed it"
                        )
                        .into_bytes(),
                    };
                }
                thread::sleep(Duration::from_millis(250));
            }
            Err(e) => panic!("error polling {}: {e}", bin.display()),
        }
    }
}

/// Parse the CLI's stdout into a `serde_json::Value`, failing loudly with
/// the raw output (on stdout AND stderr) if parsing fails.
///
/// Failure modes and what they usually mean, as of 2026-04-23:
///
/// - exit code 137 (SIGKILL) or exit code 124 (our watchdog): the CFR walk
///   ran out of memory or exceeded `CLI_TIMEOUT_SEC`. The CLI's
///   `build_subgame` is wired to `NlheSubgame::new` (A47 landed) but the
///   solver-core CFR implementation is blowing up on the canonical
///   river-with-5-combos spot. This is the current v0.1 blocker.
/// - non-zero exit with a readable stderr message: likely a parse error
///   in the inputs or an explicit `anyhow::bail!` somewhere upstream.
/// - exit 0 but invalid JSON: a JSON emitter regression.
fn parse_cli_json(out: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "solver-cli solve failed with status {:?}\n\
         \n\
         ---- stderr ----\n{}\n\
         ---- stdout ----\n{}\n\
         \n\
         If the exit is 137 (SIGKILL) or 124 (watchdog timeout), the CFR \
         walk is the blocker: `solver_core::CfrPlus::run_from` is \
         exhausting memory or running forever on the canonical river \
         spot. See crates/solver-core/ for the main-path fix. If the \
         exit is something else, the error message above should name \
         the actual failure.",
        out.status,
        stderr,
        stdout,
    );

    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "solver-cli stdout is not valid JSON: {e}\n\
             ---- stdout ----\n{stdout}\n\
             ---- stderr ----\n{stderr}"
        )
    })
}

/// Build a `HandState` corresponding to the canonical spot. Called by
/// the FFI-path test; the CLI builds its own HandState internally from
/// the string args.
///
/// On a AhKhQhJhTh royal-on-board, every surviving AKs combo plays the
/// board for a 50/50 tie. We encode both ranges as "AKs weight 1.0",
/// which maps to four combos (AhKh, AsKs, AcKc, AdKd). Card-conflict
/// pruning in `NlheSubgame::enumerate_combo_pairs` handles the AhKh /
/// AsKs overlaps with the board.
///
/// # FFI card encoding
///
/// `HandState::board[i]` is the `u8` inside `solver_eval::card::Card`:
/// `(rank << 2) | suit`, with ranks 0..13 (deuce=0, ace=12) and suits
/// 0..4 (clubs=0, diamonds=1, hearts=2, spades=3). See
/// `solver-eval/src/card.rs`. AhKhQhJhTh is therefore:
///   - Ah = (12<<2) | 2 = 50
///   - Kh = (11<<2) | 2 = 46
///   - Qh = (10<<2) | 2 = 42
///   - Jh = ( 9<<2) | 2 = 38
///   - Th = ( 8<<2) | 2 = 34
fn canonical_hand_state() -> HandState {
    use solver_eval::card::Card;
    use solver_nlhe::Range;

    let board_cards: [Card; 5] = [
        Card::parse("Ah").unwrap(),
        Card::parse("Kh").unwrap(),
        Card::parse("Qh").unwrap(),
        Card::parse("Jh").unwrap(),
        Card::parse("Th").unwrap(),
    ];

    let hero_range = Range::parse(HERO_RANGE_STR).expect("hero range parses");
    let villain_range = Range::parse(VILLAIN_RANGE_STR).expect("villain range parses");

    // Reconstruct the 1326-wide weight vectors. `Range::weights` is a
    // `Box<[f32; 1326]>`; we deref it into the FFI struct's inline
    // array. Copying ~5 KB per call is fine — this path only runs in
    // tests, and Poker Panel's real path memcpy's the buffer the same
    // way from CardEYE output.
    let hero_weights: [f32; 1326] = *hero_range.weights;
    let villain_weights: [f32; 1326] = *villain_range.weights;

    HandState {
        board: [
            board_cards[0].0,
            board_cards[1].0,
            board_cards[2].0,
            board_cards[3].0,
            board_cards[4].0,
        ],
        board_len: 5,
        hero_range: hero_weights,
        villain_range: villain_weights,
        pot: POT,
        effective_stack: STACK,
        to_act: 0, // hero first to act
        bet_tree_version: 0,
    }
}

/// Validate that the JSON structure matches the shape documented in the
/// A27 task brief AND the A5 contract in `solve_cmd.rs::build_result_json`.
/// A missing or extra top-level field is a failure.
fn assert_cli_json_shape(v: &Value) {
    // Top level: { input, result, solver_version }
    let top = v
        .as_object()
        .unwrap_or_else(|| panic!("CLI output is not a JSON object: {v}"));

    let allowed_top_keys: &[&str] = &["input", "result", "solver_version"];
    for k in top.keys() {
        assert!(
            allowed_top_keys.contains(&k.as_str()),
            "unexpected top-level key {k:?} in CLI JSON: {v}"
        );
    }
    for k in allowed_top_keys {
        assert!(
            top.contains_key(*k),
            "CLI JSON missing top-level key {k:?}: {v}"
        );
    }

    // `input` shape.
    let input = v.get("input").and_then(Value::as_object).unwrap();
    for k in [
        "board",
        "hero_range",
        "villain_range",
        "pot",
        "stack",
        "iterations",
        "bet_tree",
    ] {
        assert!(input.contains_key(k), "input missing key {k:?}: {v}");
    }
    assert_eq!(input["board"], BOARD_STR);
    assert_eq!(input["hero_range"], HERO_RANGE_STR);
    assert_eq!(input["villain_range"], VILLAIN_RANGE_STR);
    assert_eq!(input["pot"], POT);
    assert_eq!(input["stack"], STACK);
    assert_eq!(input["iterations"], ITERATIONS);

    // `result` shape.
    let result = v.get("result").and_then(Value::as_object).unwrap();
    for k in [
        "action_frequencies",
        "ev_per_action",
        "hero_equity",
        "exploitability",
        "iterations",
        "compute_ms",
    ] {
        assert!(result.contains_key(k), "result missing key {k:?}: {v}");
    }

    // `iterations` reported back in the result must match the request.
    assert_eq!(
        result["iterations"].as_u64().unwrap(),
        u64::from(ITERATIONS),
        "result.iterations does not match request"
    );

    // `compute_ms` must be a non-negative integer.
    let compute_ms = result["compute_ms"]
        .as_u64()
        .unwrap_or_else(|| panic!("result.compute_ms not a u64: {}", result["compute_ms"]));
    assert!(
        compute_ms < 10 * 60_000,
        "compute_ms = {compute_ms} is implausibly large (>10 minutes)"
    );

    // `hero_equity` must be finite in [0, 1].
    let hero_equity = result["hero_equity"]
        .as_f64()
        .unwrap_or_else(|| panic!("result.hero_equity not a number: {}", result["hero_equity"]));
    assert!(
        hero_equity.is_finite() && (0.0..=1.0).contains(&hero_equity),
        "hero_equity {hero_equity} out of [0,1]"
    );

    // `action_frequencies` must form a valid probability distribution if
    // non-empty (sum ≈ 1). When the CLI still emits an empty object —
    // the Day 2 default — we accept that as "no root action enumeration
    // wired up yet" and note it in the same assertion.
    let freqs = result["action_frequencies"]
        .as_object()
        .unwrap_or_else(|| panic!("action_frequencies not an object: {v}"));
    if !freqs.is_empty() {
        let sum: f64 = freqs
            .values()
            .map(|p| {
                p.as_f64()
                    .unwrap_or_else(|| panic!("action_frequencies value is not numeric: {p}"))
            })
            .sum();
        assert!(
            (sum - 1.0).abs() < 1e-3,
            "action_frequencies do not sum to 1: sum={sum}, freqs={freqs:?}"
        );
    }
}

/// Assert the FFI-produced `SolveResult` is internally consistent.
fn assert_ffi_result_shape(out: &SolveResult) {
    // action_count fits in the 8-slot array.
    assert!(
        out.action_count <= 8,
        "action_count {} > 8, overflows action_freq buffer",
        out.action_count
    );

    // If the solver populated actions, the frequency slots through
    // action_count must form a valid probability distribution.
    if out.action_count > 0 {
        let n = usize::from(out.action_count);
        let sum: f32 = out.action_freq[..n].iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-3,
            "FFI action_freq does not sum to 1: sum={sum}, freqs={:?}",
            &out.action_freq[..n]
        );
        for (i, &p) in out.action_freq[..n].iter().enumerate() {
            assert!(
                (0.0..=1.0).contains(&p),
                "action_freq[{i}] = {p} out of [0,1]"
            );
        }
    }

    // hero_equity in [0, 1] when finite; NaN is explicit "not populated"
    // but we treat it as failure because Poker Panel can't render NaN.
    assert!(
        out.hero_equity.is_finite(),
        "FFI hero_equity is not finite: {}",
        out.hero_equity
    );
    assert!(
        (0.0..=1.0).contains(&out.hero_equity),
        "FFI hero_equity {} out of [0,1]",
        out.hero_equity
    );

    assert!(
        out.iterations >= 1,
        "FFI iterations_used = {}, must be ≥ 1",
        out.iterations
    );
}

/// Assert that two path outputs agree numerically on the load-bearing
/// fields. This is the whole point of the e2e test.
fn assert_agreement(cli: &Value, ffi: &SolveResult) {
    let cli_result = cli.get("result").unwrap();

    let cli_equity = cli_result["hero_equity"].as_f64().unwrap() as f32;
    assert!(
        (cli_equity - ffi.hero_equity).abs() < EPS,
        "hero_equity disagreement: CLI={cli_equity}, FFI={}",
        ffi.hero_equity
    );

    // Both paths should have converged to something near the known tie
    // equity for royal-on-board.
    assert!(
        (cli_equity - EXPECTED_HERO_EQUITY).abs() < 1e-2,
        "CLI hero_equity = {cli_equity} but expected ~{EXPECTED_HERO_EQUITY} \
         (royal-flush-on-board → both players play the board → exact tie)"
    );
    assert!(
        (ffi.hero_equity - EXPECTED_HERO_EQUITY).abs() < 1e-2,
        "FFI hero_equity = {} but expected ~{EXPECTED_HERO_EQUITY}",
        ffi.hero_equity
    );

    // Iterations should match what we asked for on both sides.
    let cli_iters = cli_result["iterations"].as_u64().unwrap() as u32;
    assert_eq!(cli_iters, ITERATIONS, "CLI iterations mismatch");
    assert_eq!(ffi.iterations, ITERATIONS, "FFI iterations mismatch");
}

// ---------------------------------------------------------------------------
// FFI layout guards
// ---------------------------------------------------------------------------
//
// If someone reorders fields in `SolveResult` or `HandState`, Poker
// Panel's Swift bridging header silently desyncs — with consequences
// like reading a float as if it were a u32. These asserts catch layout
// drift at test time, long before any Swift binary sees the bits.
//
// We assert field **offsets** (via `memoffset`-equivalent unsafe
// arithmetic) rather than absolute sizes, because adding padding for
// alignment is OK but moving a field is not.

// Rust's `std::mem::offset_of!` stabilised in 1.77; our MSRV is 1.75,
// so we hand-roll a null-pointer offset helper using the
// `memoffset`-style idiom. The macro only computes the offset; it never
// dereferences, which is why it's sound without an actual instance.
macro_rules! offset_of {
    ($ty:ty, $field:ident) => {{
        // SAFETY: a null pointer is a valid base for address arithmetic as
        // long as we only compute the offset and never deref. This is the
        // idiom used by `memoffset::offset_of!` before Rust 1.77.
        let base = std::ptr::null::<$ty>();
        unsafe { &(*base).$field as *const _ as usize }
    }};
}

fn assert_ffi_layout() {
    // HandState field order (matches include/solver.h at 2026-04-22):
    //   board, board_len, hero_range, villain_range, pot,
    //   effective_stack, to_act, bet_tree_version.
    assert_eq!(offset_of!(HandState, board), 0, "board must be first");
    // board_len comes immediately after the 5-byte board array.
    assert_eq!(
        offset_of!(HandState, board_len),
        5,
        "board_len must follow board[5]"
    );
    // hero_range is a [f32; 1326] with 4-byte alignment; it starts at
    // the first multiple of 4 after board_len. With board[5]+board_len=6,
    // the next 4-aligned offset is 8.
    assert_eq!(
        offset_of!(HandState, hero_range),
        8,
        "hero_range must start at offset 8 (4-byte aligned after 6-byte prefix)"
    );
    // villain_range follows immediately: 8 + 1326*4 = 5312.
    assert_eq!(
        offset_of!(HandState, villain_range),
        8 + 1326 * 4,
        "villain_range must follow hero_range tightly"
    );
    // pot follows villain_range: 5312 + 1326*4 = 10616.
    assert_eq!(
        offset_of!(HandState, pot),
        8 + 2 * 1326 * 4,
        "pot must follow villain_range tightly"
    );
    // effective_stack is u32 aligned, follows pot: 10616 + 4 = 10620.
    assert_eq!(
        offset_of!(HandState, effective_stack),
        8 + 2 * 1326 * 4 + 4,
        "effective_stack must follow pot tightly"
    );

    // SolveResult field order:
    //   solver_version, action_count, action_freq, action_ev,
    //   hero_equity, exploitability, iterations, compute_ms.
    assert_eq!(
        offset_of!(SolveResult, solver_version),
        0,
        "solver_version must be first in SolveResult"
    );
    assert_eq!(
        offset_of!(SolveResult, action_count),
        4,
        "action_count must follow solver_version (u32)"
    );
    // action_freq is [f32; 8] with 4-byte alignment; with action_count=u8
    // at offset 4, the next 4-aligned offset is 8.
    assert_eq!(
        offset_of!(SolveResult, action_freq),
        8,
        "action_freq must start at the first 4-aligned offset after action_count"
    );
    assert_eq!(
        offset_of!(SolveResult, action_ev),
        8 + 8 * 4,
        "action_ev must follow action_freq[8] tightly"
    );
    assert_eq!(
        offset_of!(SolveResult, hero_equity),
        8 + 2 * 8 * 4,
        "hero_equity must follow action_ev[8] tightly"
    );

    // A sanity check on the status enum values — Poker Panel's consumer
    // code pattern-matches these, and silently shifting them would
    // cause the Swift side to mis-route "cache miss" as "invalid
    // input" etc.
    assert_eq!(SolverStatus::Ok as i32, 0);
    assert_eq!(SolverStatus::CacheMiss as i32, 1);
    assert_eq!(SolverStatus::InvalidInput as i32, -1);
    assert_eq!(SolverStatus::InternalError as i32, -2);
    assert_eq!(SolverStatus::OutputTooSmall as i32, -3);
}

// ---------------------------------------------------------------------------
// Actual test
// ---------------------------------------------------------------------------

/// The end-to-end integration test. Marked `#[ignore]` so plain
/// `cargo test` doesn't run it — it requires the release binary to
/// exist on disk. `scripts/e2e.sh` builds the binary and then invokes
/// `cargo test --release -p solver-cli --test e2e_integration -- --ignored end_to_end`.
#[test]
#[ignore = "requires target/release/solver-cli on disk — run via scripts/e2e.sh"]
fn end_to_end() {
    // Step 0: FFI struct layout is what the cbindgen-generated header
    // says it is. Do this before any other path so layout drift surfaces
    // immediately rather than looking like a value-mismatch further down.
    assert_ffi_layout();

    // Step 1: CLI path. Shell out to the release binary and validate
    // the JSON shape + numeric reasonableness.
    let cli_out = run_cli_solve();
    let cli_json = parse_cli_json(&cli_out);
    assert_cli_json_shape(&cli_json);

    // Step 2: FFI path. Call solver_solve in-process with the same
    // canonical HandState.
    let input = canonical_hand_state();
    let mut output = unsafe { std::mem::zeroed::<SolveResult>() };

    let handle: *mut SolverHandle = solver_ffi::solver_new();
    let rc = solver_ffi::solver_solve(handle, &input as *const _, &mut output as *mut _);
    // Always free, even on assertion failure.
    let free_guard = FreeOnDrop(handle);

    assert_eq!(
        rc,
        SolverStatus::Ok as i32,
        "solver_solve returned status {rc} (expected Ok=0).\n\
         \n\
         If rc == -2 (InternalError), `solver_ffi::solver_solve` is still \
         the stub in crates/solver-ffi/src/lib.rs that returns \
         InternalError unconditionally. Wire it to \
         `solver_core::CfrPlus` + `NlheSubgame::new` to complete the \
         FFI path.\n\
         \n\
         If rc == -1 (InvalidInput), the canonical HandState is being \
         rejected — check the card encoding helper \
         `canonical_hand_state()` above."
    );

    assert_ffi_result_shape(&output);

    // Step 3: Cross-path agreement.
    assert_agreement(&cli_json, &output);

    drop(free_guard);
}

/// Drop guard that frees a SolverHandle even if an assertion between
/// `solver_new` and `solver_free` panics. Without this, a failing
/// assertion would leak the handle's scratch memory and poison any
/// follow-up test run in the same process.
struct FreeOnDrop(*mut SolverHandle);
impl Drop for FreeOnDrop {
    fn drop(&mut self) {
        solver_ffi::solver_free(self.0);
    }
}
