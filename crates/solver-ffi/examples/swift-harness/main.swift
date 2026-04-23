// swift-harness/main.swift
//
// Minimal Swift program that exercises every symbol declared in
// solver.h. This is the dry run of Poker Panel's consumption path: if
// this compiles, links, and runs, the Swift side of the contract is
// real.
//
// Build: see ./README.md
//
// Runtime behaviour with the current (stubbed) solver-ffi:
//   - solver_version()         → prints "0.1.0-wip"
//   - solver_new()             → returns a null handle (stub)
//   - solver_solve(...)        → returns a non-OK status (stub)
//   - solver_lookup_cached(..) → returns CacheMiss (stub)
//
// That's fine. This harness validates linkage + ABI shape, not solver
// correctness. Once solver-core lands the real implementation, the same
// .swift file will start returning meaningful results without any code
// change here.
//
// Why not use the nicer memberwise initialisers? `HandState` contains
// two `[Float; 1326]` fields. Swift imports a C fixed-size array as a
// homogeneous tuple of that arity — a 1326-tuple cannot realistically
// be written as a literal. The approach below instead allocates the
// structs as zeroed raw memory and hands pointers to the C API, which
// is exactly how Poker Panel will drive it in production anyway (the
// HandState originates from CardEYE, not from a Swift literal).

import Foundation

print("poker-solver Swift harness — ABI smoke test")
print("============================================")

// ----------------------------------------------------------------------
// 1. Version string — no allocation, no pointer juggling. Easy case.
// ----------------------------------------------------------------------
if let versionPtr = solver_version() {
    let version = String(cString: versionPtr)
    print("solver_version():          \(version)")
} else {
    // solver_version()'s contract says it must never return null.
    print("solver_version():          <null>  (FAIL — contract violation)")
    exit(1)
}

// ----------------------------------------------------------------------
// 2. Handle lifecycle
// ----------------------------------------------------------------------
let handle = solver_new()
print("solver_new():              \(handle == nil ? "<null>  (expected for v0.1 stub)" : "non-null")")

// Defer so solver_free runs even if we bail out. Mirrors how a real
// consumer (one handle per thread, dropped on thread teardown) is
// structured.
defer {
    solver_free(handle)
    print("solver_free():             returned")
}

// ----------------------------------------------------------------------
// 3. solver_solve — pass a zeroed HandState via raw-pointer allocation
// ----------------------------------------------------------------------
// Allocate a zeroed HandState on the heap. UnsafeMutablePointer is the
// right primitive here because HandState is ~10KB of range data and we
// want to avoid tuple-literal gymnastics.
let inputSize = MemoryLayout<HandState>.size
let inputAlign = MemoryLayout<HandState>.alignment
let inputRaw = UnsafeMutableRawPointer.allocate(byteCount: inputSize, alignment: inputAlign)
inputRaw.initializeMemory(as: UInt8.self, repeating: 0, count: inputSize)
defer { inputRaw.deallocate() }

let inputPtr = inputRaw.bindMemory(to: HandState.self, capacity: 1)
// Set the few scalar fields that matter for a smoke test. The two range
// arrays stay zero-filled. A real consumer would memcpy into them.
inputPtr.pointee.pot = 100
inputPtr.pointee.effective_stack = 10_000
inputPtr.pointee.to_act = 0
inputPtr.pointee.bet_tree_version = 0
inputPtr.pointee.board_len = 0

let outputSize = MemoryLayout<SolveResult>.size
let outputAlign = MemoryLayout<SolveResult>.alignment
let outputRaw = UnsafeMutableRawPointer.allocate(byteCount: outputSize, alignment: outputAlign)
outputRaw.initializeMemory(as: UInt8.self, repeating: 0, count: outputSize)
defer { outputRaw.deallocate() }

let outputPtr = outputRaw.bindMemory(to: SolveResult.self, capacity: 1)

let solveStatus = solver_solve(handle, inputPtr, outputPtr)
print("solver_solve():            status=\(solveStatus)")
print("  action_count            = \(outputPtr.pointee.action_count)")
print("  hero_equity             = \(outputPtr.pointee.hero_equity)")
print("  iterations              = \(outputPtr.pointee.iterations)")

// ----------------------------------------------------------------------
// 4. solver_lookup_cached — same input, fresh output buffer
// ----------------------------------------------------------------------
// Reuse the output buffer by zeroing it. Real consumers will do the same
// — scratch buffers are recycled across calls.
outputRaw.initializeMemory(as: UInt8.self, repeating: 0, count: outputSize)
let lookupStatus = solver_lookup_cached(inputPtr, outputPtr)
print("solver_lookup_cached():    status=\(lookupStatus)")

// ----------------------------------------------------------------------
// Summary
// ----------------------------------------------------------------------
// We don't assert particular status values — the stubs return non-OK on
// purpose — but we do assert the calls returned at all. A broken ABI
// would manifest as a linker error or a crash, not a wrong status code.
print("============================================")
print("ABI smoke test complete. All symbols linked and callable.")
