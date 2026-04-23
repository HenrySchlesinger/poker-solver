// main_e2e.swift
//
// End-to-end Swift harness for poker-solver. Unlike A13's main.swift
// (which is an *ABI smoke test* — "do the symbols link, does nothing
// crash"), this file is an *outcome test* — it calls solver_solve on a
// real HandState, asserts the returned SolveResult makes sense, and
// exits non-zero if anything is off.
//
// Build (via scripts/build_swift_harness.sh):
//
//   swiftc crates/solver-ffi/examples/swift-harness/main_e2e.swift \
//       -import-objc-header crates/solver-ffi/include/solver.h \
//       -L target/release -lsolver_ffi \
//       -o target/swift-harness-e2e
//
// The Rust-side test (crates/solver-cli/tests/e2e_integration.rs)
// asserts the CLI and in-process FFI paths agree numerically. This
// Swift binary adds the third corner: same canonical spot, called from
// a Swift-language consumer (as Poker Panel will), emitting the
// SolveResult fields as JSON so the driver script can diff them.
//
// Per the A27 brief:
//   - Call solver_solve with a real HandState (AhKhQhJhTh royal, AKs
//     ranges on both sides — the simplest spot where both players
//     play the board for a 50/50 tie).
//   - Print SolveResult fields as JSON to stdout.
//   - Exit non-zero if any assertion fails.
//
// A27 is explicit that this must fail loudly while the solver is
// stubbed. As of 2026-04-23 solver_solve returns InternalError (-2);
// this harness asserts the documented "Ok = 0" return and therefore
// exits non-zero, surfacing the wiring gap in CI rather than masking it.

import Foundation

// ----------------------------------------------------------------------
// Canonical spot. MUST match the constants in
// crates/solver-cli/tests/e2e_integration.rs — these three paths
// (CLI, FFI-in-Rust, FFI-in-Swift) are only meaningful if they're
// exercising the exact same solve.
// ----------------------------------------------------------------------

// Card encoding: (rank << 2) | suit, where rank 0..13 (2 = 0, A = 12)
// and suit 0..4 (c = 0, d = 1, h = 2, s = 3). See
// crates/solver-eval/src/card.rs::Card::new.
let BOARD: [UInt8] = [
    (12 << 2) | 2, // Ah
    (11 << 2) | 2, // Kh
    (10 << 2) | 2, // Qh
    (9  << 2) | 2, // Jh
    (8  << 2) | 2, // Th
]
let POT:   UInt32 = 100
let STACK: UInt32 = 500

// `AKs` encoded as 1326-combo weights, matching solver-nlhe's
// `Range::parse("AKs")`: weight 1.0 for each of the 4 suited AK combos,
// zero for everything else. We compute the four AK-suited indices from
// the same `combo_index` formula solver-eval uses, so we don't drift
// from the Rust canonicalization.
//
// combo_index formula (from solver-eval/src/combo.rs): the 1326 pair
// indices are produced by iterating over all (a, b) with a < b, 0 ≤ a,b
// < 52. We replicate that lookup locally to produce the same weight
// vector Rust's Range::parse("AKs") produces.
func combo_index(_ cardA: UInt8, _ cardB: UInt8) -> Int {
    let (lo, hi) = cardA < cardB ? (cardA, cardB) : (cardB, cardA)
    // Triangular index: sum_{k=0..lo} (51 - k) + (hi - lo - 1).
    // Equivalent closed form: lo*(51 - lo) + lo*(lo+1)/2 ... we just
    // iterate, this is called 4 times total.
    var idx = 0
    for a in 0..<Int(lo) {
        idx += 51 - a
    }
    idx += Int(hi) - Int(lo) - 1
    return idx
}

func aks_weights() -> [Float] {
    var w = [Float](repeating: 0.0, count: 1326)
    // AKs = { AcKc, AdKd, AhKh, AsKs }. Ranks A=12, K=11. Suits 0..3.
    for s in UInt8(0)...UInt8(3) {
        let aCard: UInt8 = (12 << 2) | s
        let kCard: UInt8 = (11 << 2) | s
        w[combo_index(aCard, kCard)] = 1.0
    }
    return w
}

// ----------------------------------------------------------------------
// Assertion helpers — print a clear failure message and exit 1.
// ----------------------------------------------------------------------

var testsPassed = 0

func fail(_ msg: String) -> Never {
    fputs("FAIL: \(msg)\n", stderr)
    exit(1)
}

func check(_ cond: @autoclosure () -> Bool, _ msg: String) {
    if !cond() { fail(msg) }
    testsPassed += 1
}

// ----------------------------------------------------------------------
// Build the canonical HandState
// ----------------------------------------------------------------------

let inputSize  = MemoryLayout<HandState>.size
let inputAlign = MemoryLayout<HandState>.alignment
let inputRaw   = UnsafeMutableRawPointer.allocate(byteCount: inputSize, alignment: inputAlign)
inputRaw.initializeMemory(as: UInt8.self, repeating: 0, count: inputSize)
defer { inputRaw.deallocate() }

let inputPtr = inputRaw.bindMemory(to: HandState.self, capacity: 1)

// Board: write bytes directly via the raw buffer — the imported tuple
// for [UInt8; 5] is clumsy to index in Swift.
let boardBase = UnsafeMutableRawPointer(mutating: inputRaw)
for (i, c) in BOARD.enumerated() {
    boardBase.storeBytes(of: c, toByteOffset: i, as: UInt8.self)
}
inputPtr.pointee.board_len = 5
inputPtr.pointee.pot = POT
inputPtr.pointee.effective_stack = STACK
inputPtr.pointee.to_act = 0 // hero first to act
inputPtr.pointee.bet_tree_version = 0

// Compute the hero_range / villain_range offsets from the same layout
// math the Rust-side test asserts:
//     board[5] board_len → 6 bytes
//     align(4) → offset 8
//     hero_range: [f32; 1326] → 5304 bytes → ends at 5312
//     villain_range: [f32; 1326] → ends at 10616
let HERO_RANGE_OFFSET:    Int = 8
let VILLAIN_RANGE_OFFSET: Int = 8 + 1326 * 4

let weights = aks_weights()
for (i, w) in weights.enumerated() {
    inputRaw.storeBytes(of: w, toByteOffset: HERO_RANGE_OFFSET    + i * 4, as: Float.self)
    inputRaw.storeBytes(of: w, toByteOffset: VILLAIN_RANGE_OFFSET + i * 4, as: Float.self)
}

// ----------------------------------------------------------------------
// Output buffer
// ----------------------------------------------------------------------

let outputSize  = MemoryLayout<SolveResult>.size
let outputAlign = MemoryLayout<SolveResult>.alignment
let outputRaw   = UnsafeMutableRawPointer.allocate(byteCount: outputSize, alignment: outputAlign)
outputRaw.initializeMemory(as: UInt8.self, repeating: 0, count: outputSize)
defer { outputRaw.deallocate() }

let outputPtr = outputRaw.bindMemory(to: SolveResult.self, capacity: 1)

// ----------------------------------------------------------------------
// Version + lifecycle
// ----------------------------------------------------------------------

guard let versionPtr = solver_version() else {
    fail("solver_version() returned null — contract violation")
}
let version = String(cString: versionPtr)
check(!version.isEmpty, "solver_version() returned empty string")

let handle = solver_new()
defer { solver_free(handle) }

// ----------------------------------------------------------------------
// The actual solve
// ----------------------------------------------------------------------

let rc = solver_solve(handle, inputPtr, outputPtr)

// The outcome test — this is where A27 differs from A13's smoke test.
// A13 accepts any status; we demand Ok.
check(
    rc == 0,
    """
    solver_solve returned status \(rc), expected 0 (Ok).

    If rc == -2 (InternalError), solver_ffi::solver_solve is still the
    Day 2 stub in crates/solver-ffi/src/lib.rs that returns
    InternalError unconditionally. Wire it to solver_core::CfrPlus +
    NlheSubgame::new to complete the Swift-facing FFI path.
    """
)

// ----------------------------------------------------------------------
// SolveResult sanity checks
// ----------------------------------------------------------------------

let hero_equity = outputPtr.pointee.hero_equity
check(
    hero_equity.isFinite && hero_equity >= 0.0 && hero_equity <= 1.0,
    "hero_equity=\(hero_equity) out of [0,1] or non-finite"
)
// Royal-on-board tie: both sides play the board for exactly 0.5.
check(
    abs(hero_equity - 0.5) < 0.01,
    "hero_equity=\(hero_equity) expected ~0.5 on AhKhQhJhTh (royal on board → both play the board → tie)"
)

let iterations = outputPtr.pointee.iterations
check(iterations >= 1, "iterations=\(iterations), expected >= 1")

let action_count = outputPtr.pointee.action_count
check(action_count <= 8, "action_count=\(action_count) > 8, would overflow action_freq[8]")

// If any actions populated, frequencies must form a probability distribution.
if action_count > 0 {
    var sum: Float = 0
    // action_freq is imported as a fixed-size tuple of Float8. We read
    // via raw-pointer arithmetic to avoid Swift's tuple-indexing clumsiness.
    let freqBase = outputRaw.advanced(by: 8) // offset from layout above
    for i in 0..<Int(action_count) {
        let p = freqBase.load(fromByteOffset: i * 4, as: Float.self)
        check(p >= 0.0 && p <= 1.0, "action_freq[\(i)]=\(p) out of [0,1]")
        sum += p
    }
    check(abs(sum - 1.0) < 1e-3, "action_freq sum=\(sum), expected ~1.0")
}

// ----------------------------------------------------------------------
// Emit the SolveResult as JSON — the e2e driver script consumes this.
// ----------------------------------------------------------------------

// Manual JSON emission: avoids linking JSONEncoder and keeps the binary
// small. Format matches exactly what the Rust test expects from this
// side of the diff.
print("{")
print("  \"solver_version\": \"\(version)\",")
print("  \"status\": \(rc),")
print("  \"hero_equity\": \(hero_equity),")
print("  \"exploitability\": \(outputPtr.pointee.exploitability),")
print("  \"iterations\": \(iterations),")
print("  \"compute_ms\": \(outputPtr.pointee.compute_ms),")
print("  \"action_count\": \(action_count),")
print("  \"tests_passed\": \(testsPassed)")
print("}")

fputs("Swift e2e harness: all \(testsPassed) assertions passed\n", stderr)
exit(0)
