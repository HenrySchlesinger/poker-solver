//
// PokerSolver.swift — thin Swift wrapper over the poker-solver FFI.
//
// Consumers `import PokerSolver` and get:
//   - All the raw C symbols/types (HandState, SolveResult, solver_new,
//     solver_solve, etc.) via `@_exported import PokerSolverBinary`.
//   - A Swift-flavored `PokerSolverStatus` enum with the documented
//     return codes, so you can `switch` on them without magic numbers.
//   - A `PokerSolver.version` convenience wrapping `solver_version()`.
//
// Anything more opinionated (a SolverSession class, range helpers,
// async shims) lives in the downstream app. See docs/INTEGRATION.md
// for the recommended call patterns.

@_exported import PokerSolverBinary

/// Strongly-typed wrapper around the C `SolverStatus` enum.
///
/// Mirrors the values defined in `solver.h`. Values match the C enum
/// exactly so `Int32(rawValue:)` round-trips.
public enum PokerSolverStatus: Int32, Sendable {
    /// Success. `SolveResult` is valid.
    case ok = 0
    /// `solver_lookup_cached` only — caller should fall through to `solver_solve`.
    case cacheMiss = 1
    /// Null pointer or malformed `HandState`.
    case invalidInput = -1
    /// A Rust panic was caught at the FFI boundary. File a bug.
    case internalError = -2
    /// Output buffer too small. Reserved, does not fire in v0.1.
    case outputTooSmall = -3
}

/// Namespace for Swifty helpers on top of the raw FFI.
public enum PokerSolver {
    /// The version string baked into the underlying native library.
    ///
    /// Matches the release tag for pinned builds (e.g. `"0.1.0"`).
    /// Poker Panel checks this at startup to catch ABI drift — see
    /// `docs/INTEGRATION.md` section 6c.
    public static var version: String {
        // solver_version() returns a static null-terminated string the
        // caller must not free. String(cString:) copies it, which is
        // what we want — we don't want to hand the caller a pointer
        // into library-owned memory.
        String(cString: solver_version())
    }
}
