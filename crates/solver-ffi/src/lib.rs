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

use std::panic::{self, catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use solver_core::{CfrPlusVector, Player, VectorGame};
use solver_eval::card::Card;
use solver_eval::combo::NUM_COMBOS;
use solver_eval::Board;
use solver_nlhe::action::Action;
use solver_nlhe::{BetTree, NlheSubgameVector, Range};

/// Default CFR+ iteration count for the FFI `solver_solve` entry point.
///
/// v0.1 hardcodes this because `HandState` has no `iterations` field yet
/// and adding one would break ABI for Swift callers already compiled
/// against the current struct layout. See `TODO (v0.2)` below.
// TODO (v0.2): add an `iterations: u32` field to `HandState` so callers
// can dial the accuracy/latency trade-off per spot. That's an ABI break
// and must be coordinated with a Poker Panel release.
const DEFAULT_ITERATIONS: u32 = 100;

/// Worker-thread stack size for the CFR tree walk.
///
/// Matches `solver-cli::solve_cmd::SOLVE_THREAD_STACK_BYTES` (128 MB):
/// the CFR walk is a deep recursive descent and overflows the default
/// 8 MB macOS thread stack on non-trivial river trees. See the matching
/// comment in `solve_cmd.rs` for the full rationale.
const SOLVE_THREAD_STACK_BYTES: usize = 128 * 1024 * 1024;

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
    /// Exploitability at solve termination.
    ///
    /// **v0.1:** always `f32::NAN` on a successful solve — see
    /// `docs/EXPLOITABILITY_TRIAGE.md`. The current `CfrPlus::exploitability()`
    /// walker reports a phantom-root number that scales with pot size,
    /// not a real Nash distance, so we emit NaN as a "not meaningful"
    /// sentinel until the root-aware helper lands post-v0.1. The field
    /// is retained in the struct (rather than removed) because Swift
    /// callers compiled against the v0.1 header assume this layout —
    /// changing it would be an ABI break.
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
///
/// Passing a null pointer is a no-op (matches standard C `free` semantics).
#[no_mangle]
pub extern "C" fn solver_free(handle: *mut SolverHandle) {
    if !handle.is_null() {
        // TODO (Day 4): reclaim scratch arenas associated with `handle`.
        let _ = handle;
    }
}

/// Solve a spot live.
///
/// Returns `SolverStatus::Ok` on success, `InvalidInput` on malformed
/// arguments, `InternalError` if we caught a panic.
///
/// # v0.1 caveats
///
/// - River-only: `HandState.board_len` must be 5. Turn/flop subgames
///   come with the cache (v0.2) and the turn subgame (v0.3).
/// - `bet_tree_version` must be 0 (the default v0.1 tree). Other values
///   are reserved for future tree profiles.
/// - Iteration count is hardcoded to `DEFAULT_ITERATIONS` (100). See the
///   constant's doc-comment for the ABI-stability rationale.
/// - Any stack size is OK. A58's AllIn-terminal fix (commit `5629935`)
///   bounds the river tree under arbitrary stack depths; the previous
///   "stack=0 or small values" caveat is no longer accurate.
///
/// # Safety
///
/// `input` and `output` must either be null or point to a valid,
/// correctly-aligned `HandState` / `SolveResult` respectively. The
/// function null-checks them before dereferencing and returns
/// `InvalidInput` on null — any other invalid pointer (dangling,
/// misaligned, wrong size) is undefined behavior. This matches the C
/// ABI contract the cbindgen-generated `solver.h` documents.
// Clippy (not_unsafe_ptr_arg_deref) would like this marked `unsafe`. We
// deliberately keep it safe-callable because:
//   1. The C ABI does not distinguish safe vs unsafe — Swift/C callers
//      see the same symbol either way.
//   2. Every pointer deref is guarded by an explicit null check and
//      lives inside an `unsafe { … }` block with a SAFETY comment.
//   3. The documented C contract puts the burden of validity on the
//      caller, which is standard for extern "C" entry points.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn solver_solve(
    _handle: *mut SolverHandle,
    input: *const HandState,
    output: *mut SolveResult,
) -> i32 {
    let result = catch_unwind(|| {
        // Null-pointer check. `_handle` is allowed to be null (the v0.1
        // stub of `solver_new` returns null and the FFI contract says
        // that's legal). `input` and `output` are load-bearing.
        if input.is_null() || output.is_null() {
            return SolverStatus::InvalidInput as i32;
        }

        // SAFETY: we've null-checked `input`; the contract requires the
        // pointer be valid for reads of `size_of::<HandState>()` bytes
        // and properly aligned. Swift callers allocate via
        // `MemoryLayout<HandState>.alignment`, which satisfies that.
        let hs = unsafe { &*input };

        match run_solve(hs) {
            Ok(solved) => {
                // SAFETY: we've null-checked `output`; the contract
                // requires the pointer be valid for writes of
                // `size_of::<SolveResult>()` bytes and properly aligned.
                unsafe {
                    std::ptr::write(output, solved);
                }
                SolverStatus::Ok as i32
            }
            Err(status) => status as i32,
        }
    });
    result.unwrap_or(SolverStatus::InternalError as i32)
}

/// Look up a precomputed result from the cache.
///
/// Returns `Ok` on hit, `CacheMiss` on miss (caller should call
/// `solver_solve`), `InvalidInput` on malformed args.
#[no_mangle]
pub extern "C" fn solver_lookup_cached(_input: *const HandState, _output: *mut SolveResult) -> i32 {
    let result = catch_unwind(|| {
        // TODO (Day 5, agent A3): dispatch to cache lookup.
        SolverStatus::CacheMiss as i32
    });
    result.unwrap_or(SolverStatus::InternalError as i32)
}

/// Version string. Null-terminated. Do not free.
#[no_mangle]
pub extern "C" fn solver_version() -> *const std::os::raw::c_char {
    // Matches `solver-cli::solve_cmd::SOLVER_VERSION`. Keep in sync with
    // the workspace version in `Cargo.toml` until we expose a build-time
    // constant.
    b"0.1.0-dev\0".as_ptr() as *const _
}

// ---------------------------------------------------------------------------
// Internal implementation
// ---------------------------------------------------------------------------

/// Validated, owned inputs derived from a `HandState`. Holds everything
/// the CFR worker thread needs, with no pointers back into caller memory.
struct ParsedInputs {
    board: Board,
    hero: Range,
    villain: Range,
    pot: u32,
    stack: u32,
    first_to_act: Player,
    bet_tree: BetTree,
}

/// Successful solve summary — aggregated root frequencies + EVs, plus
/// the scalars we report alongside them. Serialized into `SolveResult`.
struct SolveOutcome {
    action_labels: Vec<String>,
    action_freq: Vec<f32>,
    action_ev: Vec<f32>,
    hero_equity: f32,
    exploitability: f32,
    iterations: u32,
    compute_ms: u32,
}

/// Top-level driver: validate the FFI input, dispatch into the CFR
/// worker, and build the `SolveResult` payload.
///
/// On error, returns a `SolverStatus` telling the FFI wrapper what code
/// to send back to the caller.
fn run_solve(hs: &HandState) -> Result<SolveResult, SolverStatus> {
    let parsed = validate_input(hs)?;
    let outcome = solve_on_worker(parsed)?;
    Ok(build_solve_result(&outcome))
}

/// Turn the FFI `HandState` into validated, owned Rust types. Any
/// malformed field → `InvalidInput`.
fn validate_input(hs: &HandState) -> Result<ParsedInputs, SolverStatus> {
    // v0.1: river-only. The subgame type panics if `board.len != 5`, so
    // we guard here and convert that to a clean `InvalidInput` code
    // rather than relying on `catch_unwind` to catch the panic.
    if hs.board_len != 5 {
        return Err(SolverStatus::InvalidInput);
    }

    // Bet-tree version: only v0 is defined.
    let bet_tree = match hs.bet_tree_version {
        0 => BetTree::default_v0_1(),
        _ => return Err(SolverStatus::InvalidInput),
    };

    // `to_act`: only 0 (hero) / 1 (villain) are legal.
    let first_to_act = match hs.to_act {
        0 => Player::Hero,
        1 => Player::Villain,
        _ => return Err(SolverStatus::InvalidInput),
    };

    // Board card bytes are validated by `Card`: each byte must encode a
    // card in 0..52, the five cards must be distinct.
    let mut cards = [Card(0); 5];
    for (i, slot) in cards.iter_mut().enumerate() {
        let b = hs.board[i];
        if b >= 52 {
            return Err(SolverStatus::InvalidInput);
        }
        *slot = Card(b);
    }
    // Distinctness check — `Board::river`'s `debug_assert` only fires in
    // debug builds; do it explicitly in release too so the FFI never
    // invokes the solver on an impossible board.
    for i in 0..5 {
        for j in (i + 1)..5 {
            if cards[i].0 == cards[j].0 {
                return Err(SolverStatus::InvalidInput);
            }
        }
    }
    let board = Board { cards, len: 5 };

    // Ranges: copy the 1326 weights out of the FFI struct into owned
    // `Range` instances. Reject ranges with no non-zero weights — CFR
    // can't solve a spot with an empty range on either side.
    let hero = hand_range_from_ffi(&hs.hero_range);
    let villain = hand_range_from_ffi(&hs.villain_range);
    if hero.total_weight() <= 0.0 || villain.total_weight() <= 0.0 {
        return Err(SolverStatus::InvalidInput);
    }

    Ok(ParsedInputs {
        board,
        hero,
        villain,
        pot: hs.pot,
        stack: hs.effective_stack,
        first_to_act,
        bet_tree,
    })
}

/// Copy an FFI `[f32; 1326]` into an owned `Range`, rejecting NaN and
/// negative weights.
///
/// We silently treat negative or NaN weights as zero rather than
/// returning an error — the subgame's pair-enumeration treats `<=0` as
/// "not in range" anyway, so this is a defensive normalization.
fn hand_range_from_ffi(weights: &[f32; 1326]) -> Range {
    debug_assert_eq!(NUM_COMBOS, 1326);
    let mut r = Range::empty();
    for (i, &w) in weights.iter().enumerate() {
        if w.is_finite() && w > 0.0 {
            r.weights[i] = w;
        }
    }
    r
}

/// Spawn a dedicated worker thread with a large stack and run CFR there.
///
/// We mirror `solver-cli::solve_cmd::solve_to_json`: the default 8 MB
/// macOS thread stack is not enough for the river CFR walk with the
/// default bet tree. See the corresponding doc-comment in
/// `solve_cmd.rs` for details.
fn solve_on_worker(parsed: ParsedInputs) -> Result<SolveOutcome, SolverStatus> {
    let start = Instant::now();

    let worker = std::thread::Builder::new()
        .name("solver-ffi-cfr".to_string())
        .stack_size(SOLVE_THREAD_STACK_BYTES)
        .spawn(move || panic::catch_unwind(AssertUnwindSafe(|| run_cfr(&parsed))))
        .map_err(|_| SolverStatus::InternalError)?;

    let joined = match worker.join() {
        Ok(x) => x,
        Err(_) => return Err(SolverStatus::InternalError),
    };

    match joined {
        Ok(Ok(mut outcome)) => {
            outcome.compute_ms = start.elapsed().as_millis().min(u32::MAX as u128) as u32;
            Ok(outcome)
        }
        Ok(Err(status)) => Err(status),
        // Panic inside the worker (past `catch_unwind`) — shouldn't
        // happen because `catch_unwind` wraps everything, but if it
        // somehow does, translate to InternalError rather than unwind.
        Err(_) => Err(SolverStatus::InternalError),
    }
}

/// The actual solve: build the subgame, enumerate chance roots, run
/// CFR+, aggregate.
///
/// Runs on the large-stack worker spawned by `solve_on_worker`.
fn run_cfr(parsed: &ParsedInputs) -> Result<SolveOutcome, SolverStatus> {
    // River-only guard is already in `validate_input`; we assert here
    // defensively because `NlheSubgameVector::new` panics on `len != 5`.
    if parsed.board.len != 5 {
        return Err(SolverStatus::InvalidInput);
    }

    // Range-vs-range equity.
    let hero_equity = solver_eval::equity::range_vs_range_equity(
        &parsed.hero.weights,
        &parsed.villain.weights,
        &parsed.board,
        1,
    );

    // Post-A70: default to `CfrPlusVector` (combo-axis-SIMD walk, ~10×
    // faster than A64's flat path on NLHE river spots). The vector
    // subgame handles its own chance-layer integration via the
    // showdown-sign matrix + per-combo reach vectors; no explicit
    // `chance_roots` enumeration needed by the FFI layer.
    let vector_subgame = NlheSubgameVector::new(
        parsed.board,
        parsed.hero.clone(),
        parsed.villain.clone(),
        parsed.pot,
        parsed.stack,
        parsed.first_to_act,
        parsed.bet_tree.clone(),
    );

    // Check that at least one valid (hero, villain) combo pair exists.
    // If ranges fully conflict with the board or each other, there's
    // nothing to solve.
    if vector_subgame.hero_active().is_empty() || vector_subgame.villain_active().is_empty() {
        return Err(SolverStatus::InvalidInput);
    }

    let mut solver = CfrPlusVector::new(vector_subgame);
    solver.run(DEFAULT_ITERATIONS);

    // Exploitability: vector solver doesn't have a root-aware
    // convergence helper; emit NaN per the v0.1 `docs/EXPLOITABILITY_TRIAGE.md`
    // sentinel convention. The FFI struct's `exploitability` field
    // documents this.
    let exploitability = f32::NAN;
    let (labels, freq, ev) = aggregate_root_strategy_and_ev_vector(solver.game(), &solver);

    Ok(SolveOutcome {
        action_labels: labels,
        action_freq: freq,
        action_ev: ev,
        hero_equity,
        exploitability,
        iterations: DEFAULT_ITERATIONS,
        compute_ms: 0, // set by `solve_on_worker` with the wall-clock.
    })
}

/// Aggregate per-combo root strategy from the vector solver into a
/// single action-frequency vector. EVs are currently stubbed to zero
/// (v0.2 TODO; the frequencies are the primary user-visible output).
fn aggregate_root_strategy_and_ev_vector(
    game: &NlheSubgameVector,
    solver: &CfrPlusVector<NlheSubgameVector>,
) -> (Vec<String>, Vec<f32>, Vec<f32>) {
    let root = game.root();
    let root_actions: Vec<Action> = game.legal_actions(&root).to_vec();
    let num_actions = root_actions.len();
    let labels: Vec<String> = root_actions.iter().map(action_label).collect();

    if num_actions == 0 {
        return (labels, Vec::new(), Vec::new());
    }

    let first_to_act = game.current_player(&root);
    let info_id = game.info_set_id(&root, first_to_act);
    let Some(per_combo) = solver.per_combo_average_strategy(info_id) else {
        let uniform = 1.0 / num_actions as f32;
        let freq = vec![uniform; num_actions];
        let ev = vec![0.0f32; num_actions];
        return (labels, freq, ev);
    };

    let range = game.hero_range();
    let mut freq_acc = vec![0.0_f64; num_actions];
    let mut total_weight = 0.0_f64;
    for &h in game.hero_active() {
        let w = range.weights[h as usize] as f64;
        if w == 0.0 {
            continue;
        }
        for i in 0..num_actions {
            freq_acc[i] += w * per_combo[i][h as usize] as f64;
        }
        total_weight += w;
    }
    if total_weight > 0.0 {
        for f in &mut freq_acc {
            *f /= total_weight;
        }
    }
    let freq: Vec<f32> = freq_acc.iter().map(|f| *f as f32).collect();
    let ev = vec![0.0f32; num_actions];
    (labels, freq, ev)
}

/// Human-readable label for an action. Mirrors the CLI's
/// `action_label` so the two paths emit identical keys.
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

/// Package an outcome into the `SolveResult` wire format.
///
/// - The first 8 action-frequency slots hold per-action probabilities.
///   `action_count` gates how many are valid.
/// - Actions 9+ are silently dropped — the v0.1 bet tree never produces
///   more than a handful of root actions, so this is a non-issue today.
fn build_solve_result(outcome: &SolveOutcome) -> SolveResult {
    // v0.1: set to NaN — the current exploitability() walker reports a
    // phantom-root number that scales with pot, not a real Nash distance.
    // See docs/EXPLOITABILITY_TRIAGE.md; root-aware helper lands post-v0.1.
    // We keep `outcome.exploitability` populated internally (cheap, same
    // compute path) so switching back to a real value later is a one-line
    // change, but the FFI wire value is the NaN sentinel.
    let _ = outcome.exploitability;
    let mut result = SolveResult {
        solver_version: 1,
        action_count: 0,
        action_freq: [0.0; 8],
        action_ev: [0.0; 8],
        hero_equity: outcome.hero_equity,
        exploitability: f32::NAN,
        iterations: outcome.iterations,
        compute_ms: outcome.compute_ms,
    };

    let n = outcome.action_freq.len().min(8);
    result.action_count = n as u8;
    for i in 0..n {
        result.action_freq[i] = outcome.action_freq[i];
        result.action_ev[i] = outcome.action_ev[i];
    }
    // `action_labels` is not part of the FFI struct — labels are
    // communicated by position in `action_freq` / `action_ev`. The
    // labels are still useful for tests + logging; we just don't
    // serialize them into the wire format.
    let _ = &outcome.action_labels;

    result
}
