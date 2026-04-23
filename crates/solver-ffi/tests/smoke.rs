//! FFI smoke tests.
//!
//! These call every `extern "C"` symbol exposed by `solver-ffi` directly
//! from Rust, using the same signatures Swift will use. The goal is not
//! to test solver correctness (that lives in `solver-core`) — it's to
//! verify that the C ABI surface is intact:
//!
//! - Symbols are exported and callable.
//! - Null-pointer handling doesn't crash.
//! - Returned status codes match the documented values.
//! - Structs have the layout cbindgen says they do.
//!
//! If this file stops compiling, the FFI contract with Poker Panel has
//! changed — which is fine, but it's a deliberate breaking change and
//! the consumer needs a matching update.
//!
//! Why test this in Rust at all (not just Swift)? Because these run in
//! CI where swiftc isn't always available, and because a broken C ABI
//! is worth catching *before* the Swift side tries to link.
#![allow(clippy::useless_conversion)]

use solver_ffi::{HandState, SolveResult, SolverHandle, SolverStatus};
use std::ffi::CStr;
use std::mem::MaybeUninit;

/// Build a minimally-valid `HandState`. All fields zeroed is fine for
/// smoke-testing the ABI; the solver is stubbed out and doesn't inspect
/// the values yet.
fn dummy_hand_state() -> HandState {
    HandState {
        board: [0; 5],
        board_len: 0,
        hero_range: [0.0; 1326],
        villain_range: [0.0; 1326],
        pot: 100,
        effective_stack: 10_000,
        to_act: 0,
        bet_tree_version: 0,
    }
}

/// Zeroed `SolveResult` for use as an out-param. Using `MaybeUninit` here
/// would be more correct ABI-wise — the C contract doesn't require the
/// out buffer be initialized — but zeroing is cheap and makes assertions
/// on unchanged fields sensible.
fn zeroed_result() -> SolveResult {
    // SAFETY: `SolveResult` is `repr(C)` with all primitive fields, so
    // the zero pattern is a valid bit representation of the struct.
    unsafe { MaybeUninit::<SolveResult>::zeroed().assume_init() }
}

#[test]
fn version_is_non_null_and_parseable() {
    let ptr = solver_ffi::solver_version();
    assert!(!ptr.is_null(), "solver_version() returned null");

    // SAFETY: the FFI contract states the pointer is a static
    // null-terminated UTF-8 string. `CStr::from_ptr` requires exactly that.
    let cstr = unsafe { CStr::from_ptr(ptr) };
    let s = cstr
        .to_str()
        .expect("solver_version() returned invalid UTF-8");
    assert!(!s.is_empty(), "solver_version() returned empty string");
    // The current stub is "0.1.0-wip"; accept anything semver-ish so this
    // test doesn't flip every time we bump the version.
    assert!(
        s.chars().any(|c| c.is_ascii_digit()),
        "solver_version() = {s:?} looks nothing like a version"
    );
}

#[test]
fn new_and_free_are_symmetric() {
    // The current stub returns null from `solver_new` — that's a
    // documented possible outcome ("null on allocation failure"), so we
    // accept either a real pointer or null here. What we're testing is
    // that `solver_free` tolerates whatever `solver_new` returns without
    // crashing.
    let handle: *mut SolverHandle = solver_ffi::solver_new();
    solver_ffi::solver_free(handle);

    // Freeing null twice must be a no-op per the contract.
    solver_ffi::solver_free(std::ptr::null_mut());
    solver_ffi::solver_free(std::ptr::null_mut());
}

#[test]
fn solve_with_null_inputs_does_not_crash() {
    // The stub currently returns `InternalError` for every call because
    // the real solver isn't wired up yet. We don't care which specific
    // non-success code comes back — only that the call returns at all,
    // i.e. we haven't accidentally introduced UB.
    let rc = solver_ffi::solver_solve(std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut());
    assert_ne!(
        rc,
        SolverStatus::Ok as i32,
        "solve() with all-null args must not report success"
    );
}

#[test]
fn solve_with_valid_inputs_returns_a_status() {
    let handle = solver_ffi::solver_new();
    let input = dummy_hand_state();
    let mut out = zeroed_result();

    let rc = solver_ffi::solver_solve(handle, &input as *const _, &mut out as *mut _);

    // Every code from the documented status enum is acceptable. What we
    // reject is any *other* integer — that would mean the FFI is
    // returning something the C header has no name for, which Poker
    // Panel would not know how to react to.
    let allowed = [
        SolverStatus::Ok as i32,
        SolverStatus::CacheMiss as i32,
        SolverStatus::InvalidInput as i32,
        SolverStatus::InternalError as i32,
        SolverStatus::OutputTooSmall as i32,
    ];
    assert!(
        allowed.contains(&rc),
        "solver_solve returned undocumented status code {rc}"
    );

    solver_ffi::solver_free(handle);
}

/// Build the canonical-spot `HandState`: royal-flush-on-board,
/// both players holding AKs, pot=100, stack=500.
///
/// Matches `crates/solver-cli/tests/e2e_integration.rs::canonical_hand_state`
/// so the FFI path here exercises the same bits the outer e2e script
/// uses to assert CLI / FFI / Swift agreement.
fn royal_on_board_hand_state() -> HandState {
    // Card encoding: (rank << 2) | suit. ranks: 2=0..A=12, suits: c=0..s=3.
    // See crates/solver-eval/src/card.rs. AhKhQhJhTh:
    //   Ah = (12<<2)|2 = 50; Kh = 46; Qh = 42; Jh = 38; Th = 34.
    let board: [u8; 5] = [50, 46, 42, 38, 34];

    // AKs = { AcKc, AdKd, AhKh, AsKs }. Four combos, weight 1.0 each.
    // We compute each combo's 1326-wide index with the same closed form
    // `solver_eval::combo::combo_index` uses (triangular number index:
    // for cards `lo < hi`, idx = sum_{k=0..lo}(51-k) + (hi - lo - 1)).
    fn combo_index(a: u8, b: u8) -> usize {
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        let mut idx = 0usize;
        for k in 0..lo as usize {
            idx += 51 - k;
        }
        idx += (hi as usize) - (lo as usize) - 1;
        idx
    }

    let mut weights = [0.0f32; 1326];
    for suit in 0u8..4 {
        let a_card = (12u8 << 2) | suit; // A of `suit`
        let k_card = (11u8 << 2) | suit; // K of `suit`
        weights[combo_index(a_card, k_card)] = 1.0;
    }

    HandState {
        board,
        board_len: 5,
        hero_range: weights,
        villain_range: weights,
        pot: 100,
        effective_stack: 500,
        to_act: 0,
        bet_tree_version: 0,
    }
}

#[test]
fn solve_with_canonical_hand_state_returns_ok() {
    // Happy path: real inputs, real dispatch → real SolveResult.
    //
    // On AhKhQhJhTh the board is already a royal flush, so every dealt
    // AKs combo plays the board for a 50/50 tie. We assert:
    //
    // - `solver_solve` returns Ok (0).
    // - `hero_equity` is ≈ 0.5.
    // - `iterations` matches the FFI's hardcoded default (100 in v0.1).
    // - `action_count` ≤ 8 and `action_freq[..action_count]` forms a
    //   valid probability distribution.
    //
    // If this test fails with rc = InternalError, the dispatch
    // regressed — the CFR worker probably panicked and the
    // `catch_unwind` translated it. Inspect the worker path in
    // `solver_ffi::run_cfr`.
    let handle = solver_ffi::solver_new();
    let input = royal_on_board_hand_state();
    let mut out = zeroed_result();

    let rc = solver_ffi::solver_solve(handle, &input as *const _, &mut out as *mut _);
    solver_ffi::solver_free(handle);

    assert_eq!(
        rc,
        SolverStatus::Ok as i32,
        "expected Ok (0), got {rc}. \
         If -2 (InternalError), the CFR worker panicked; check solver_core. \
         If -1 (InvalidInput), the HandState failed validation in solver-ffi."
    );

    // hero_equity should be ~0.5 (royal on board = mandatory tie).
    assert!(
        out.hero_equity.is_finite(),
        "hero_equity is non-finite: {}",
        out.hero_equity
    );
    assert!(
        (out.hero_equity - 0.5).abs() < 0.01,
        "hero_equity = {} but expected ≈ 0.5 (royal-on-board tie)",
        out.hero_equity
    );

    // Iterations should match the FFI's hardcoded default. If we bump
    // DEFAULT_ITERATIONS, update this assertion to match.
    assert_eq!(
        out.iterations, 100,
        "FFI v0.1 hardcodes DEFAULT_ITERATIONS=100; got {}",
        out.iterations
    );

    // Action distribution sanity.
    assert!(
        out.action_count <= 8,
        "action_count {} > 8, overflows action_freq buffer",
        out.action_count
    );
    if out.action_count > 0 {
        let n = out.action_count as usize;
        let sum: f32 = out.action_freq[..n].iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-3,
            "action_freq does not sum to 1: sum={sum}, freqs={:?}",
            &out.action_freq[..n]
        );
        for (i, &p) in out.action_freq[..n].iter().enumerate() {
            assert!(
                (0.0..=1.0).contains(&p),
                "action_freq[{i}] = {p} out of [0,1]"
            );
        }
    }
}

#[test]
fn solve_with_non_river_board_returns_invalid_input() {
    // v0.1 is river-only. A flop board (board_len=3) must reject cleanly
    // with InvalidInput, not crash into NlheSubgame::new's `board.len == 5`
    // assertion.
    let mut input = royal_on_board_hand_state();
    input.board_len = 3;

    let mut out = zeroed_result();
    let rc = solver_ffi::solver_solve(std::ptr::null_mut(), &input as *const _, &mut out as *mut _);
    assert_eq!(
        rc,
        SolverStatus::InvalidInput as i32,
        "board_len=3 should be rejected with InvalidInput, got {rc}"
    );
}

#[test]
fn solve_with_unknown_bet_tree_version_returns_invalid_input() {
    let mut input = royal_on_board_hand_state();
    input.bet_tree_version = 7; // undefined in v0.1

    let mut out = zeroed_result();
    let rc = solver_ffi::solver_solve(std::ptr::null_mut(), &input as *const _, &mut out as *mut _);
    assert_eq!(
        rc,
        SolverStatus::InvalidInput as i32,
        "bet_tree_version=7 should be rejected with InvalidInput, got {rc}"
    );
}

#[test]
fn lookup_cached_returns_cache_miss_for_unknown_spot() {
    let input = dummy_hand_state();
    let mut out = zeroed_result();

    let rc = solver_ffi::solver_lookup_cached(&input as *const _, &mut out as *mut _);
    // The stub always reports `CacheMiss` today. When the cache is wired
    // up this may start returning `Ok` for some inputs — that's fine;
    // the assertion below just rules out UB-class return codes.
    let allowed = [
        SolverStatus::Ok as i32,
        SolverStatus::CacheMiss as i32,
        SolverStatus::InvalidInput as i32,
        SolverStatus::InternalError as i32,
        SolverStatus::OutputTooSmall as i32,
    ];
    assert!(
        allowed.contains(&rc),
        "solver_lookup_cached returned undocumented status code {rc}"
    );
}

#[test]
fn status_enum_numeric_values_are_stable() {
    // These values are documented in solver.h and consumed by Poker
    // Panel. If any of them changes you have silently broken the ABI.
    assert_eq!(SolverStatus::Ok as i32, 0);
    assert_eq!(SolverStatus::CacheMiss as i32, 1);
    assert_eq!(SolverStatus::InvalidInput as i32, -1);
    assert_eq!(SolverStatus::InternalError as i32, -2);
    assert_eq!(SolverStatus::OutputTooSmall as i32, -3);
}

#[test]
fn struct_sizes_match_documented_layout() {
    // Poker Panel's Swift bridging header assumes these layouts. If they
    // change without coordination, every in-flight SolveResult on the
    // wire is corrupt in subtle ways. Lock them here.
    //
    // Exact byte sizes depend on alignment; we assert relationships
    // instead of hardcoded numbers, so a field tweak that's ABI-neutral
    // doesn't fail this test.
    assert_eq!(
        std::mem::size_of::<HandState>() % std::mem::align_of::<HandState>(),
        0
    );
    assert_eq!(
        std::mem::size_of::<SolveResult>() % std::mem::align_of::<SolveResult>(),
        0
    );

    // HandState is dominated by the two 1326*f32 range arrays.
    // = 2 * 1326 * 4 = 10608 bytes of range data, plus the small fields.
    // Give a generous range so we catch a wildly different layout but
    // not a reasonable field addition.
    let hs = std::mem::size_of::<HandState>();
    assert!(
        (10_608..=10_800).contains(&hs),
        "HandState size {hs} outside expected band — did the ranges change?"
    );
}
