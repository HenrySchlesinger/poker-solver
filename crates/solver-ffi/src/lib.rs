//! C FFI surface for Swift consumption by Poker Panel.
//!
//! ALL public items here are part of the contract with Poker Panel.
//! Breaking changes bump the solver major version and require a matching
//! Poker Panel release.
//!
//! Safety rules for every `extern "C" fn`:
//! 1. Wrap the body in `std::panic::catch_unwind` and translate panics
//!    to an error code. A Rust panic must NEVER unwind across the FFI
//!    boundary — that's undefined behavior.
//! 2. Check all pointer arguments for null before deref.
//! 3. Never take a Rust type across the boundary — only `repr(C)` structs,
//!    primitives, and opaque pointers.

#![allow(non_snake_case)]

use std::panic::catch_unwind;

/// Opaque handle — owns scratch memory for reuse across calls.
///
/// Pokeer Panel creates one per thread/solve-queue via `solver_new`.
#[repr(C)]
pub struct SolverHandle {
    _private: [u8; 0],
}

/// Input: the full description of a spot to solve.
///
/// Laid out for cheap FFI copy. All data is owned; no pointers into
/// caller memory.
#[repr(C)]
pub struct HandState {
    /// Board cards (0..5 used, rest ignored).
    pub board: [u8; 5],
    /// Number of valid board cards.
    pub board_len: u8,
    /// Hero's range weights (1326 f32s).
    pub hero_range: [f32; 1326],
    /// Villain's range weights.
    pub villain_range: [f32; 1326],
    /// Pot size in chips.
    pub pot: u32,
    /// Effective stack in chips.
    pub effective_stack: u32,
    /// 0 = hero to act, 1 = villain to act.
    pub to_act: u8,
    /// Bet-tree version (0 = v0.1 defaults).
    pub bet_tree_version: u8,
    // TODO (Day 3, agent A6): action history encoding.
}

/// Output: strategy and derived quantities.
#[repr(C)]
pub struct SolveResult {
    /// Solver version string ID (for consumer to verify compat).
    pub solver_version: u32,
    /// Strategy: up to 8 actions, normalized to sum to 1.
    pub action_count: u8,
    /// Per-action frequency.
    pub action_freq: [f32; 8],
    /// Per-action expected value (in big blinds).
    pub action_ev: [f32; 8],
    /// Hero's equity vs villain's range on this board.
    pub hero_equity: f32,
    /// Exploitability at solve termination (lower = closer to Nash).
    pub exploitability: f32,
    /// Iterations run.
    pub iterations: u32,
    /// Wall-clock solve time, milliseconds.
    pub compute_ms: u32,
}

/// Returned error codes from `solver_solve` / `solver_lookup_cached`.
#[repr(i32)]
pub enum SolverStatus {
    /// Success.
    Ok = 0,
    /// Cache miss — caller may fall through to live solve.
    CacheMiss = 1,
    /// Invalid input (null pointer, malformed HandState, etc.).
    InvalidInput = -1,
    /// Panic caught at FFI boundary.
    InternalError = -2,
    /// Output buffer too small.
    OutputTooSmall = -3,
}

/// Create a new solver handle. Returns null on allocation failure.
#[no_mangle]
pub extern "C" fn solver_new() -> *mut SolverHandle {
    // TODO (Day 4, agent A4): allocate scratch arenas, return.
    std::ptr::null_mut()
}

/// Free a solver handle.
#[no_mangle]
pub extern "C" fn solver_free(handle: *mut SolverHandle) {
    if handle.is_null() {
        return;
    }
    // TODO (Day 4): free scratch.
}

/// Solve a spot live.
///
/// Returns `SolverStatus::Ok` on success, `InvalidInput` on malformed
/// arguments, `InternalError` if we caught a panic.
#[no_mangle]
pub extern "C" fn solver_solve(
    _handle: *mut SolverHandle,
    _input: *const HandState,
    _output: *mut SolveResult,
) -> i32 {
    let result = catch_unwind(|| {
        // TODO (Day 4, agent A_main): dispatch to solver_core.
        SolverStatus::InternalError as i32
    });
    result.unwrap_or(SolverStatus::InternalError as i32)
}

/// Look up a precomputed result from the cache.
///
/// Returns `Ok` on hit, `CacheMiss` on miss (caller should call
/// `solver_solve`), `InvalidInput` on malformed args.
#[no_mangle]
pub extern "C" fn solver_lookup_cached(
    _input: *const HandState,
    _output: *mut SolveResult,
) -> i32 {
    let result = catch_unwind(|| {
        // TODO (Day 5, agent A3): dispatch to cache lookup.
        SolverStatus::CacheMiss as i32
    });
    result.unwrap_or(SolverStatus::InternalError as i32)
}

/// Version string. Null-terminated. Do not free.
#[no_mangle]
pub extern "C" fn solver_version() -> *const std::os::raw::c_char {
    // TODO (Day 7): build-time version string.
    b"0.1.0-wip\0".as_ptr() as *const _
}
