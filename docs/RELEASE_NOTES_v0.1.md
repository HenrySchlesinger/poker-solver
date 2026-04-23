# poker-solver v0.1.0 — release notes

**Ship target:** 2026-04-29
**Tag:** `v0.1.0`

## Pitch

v0.1.0 is a local NLHE GTO solver packaged as a C FFI + Swift
wrapper, built to drive real-time strategy overlays on live
broadcasts without a cloud round-trip.

## What works today

- **River solve** — CFR+ on any legal NLHE river spot inside an
  overlay-usable latency budget. Validated against Kuhn Poker at
  ~0.00486 exploitability after 1000 iterations.
- **Preflop** — static range lookup from shipped data.
- **Flop** — cache lookup via `FlopCache`. The shipped
  `flop-cache-v0.1.bin` (374 KB, 36 entries) is **format-only
  placeholder**; real Colab-precomputed data lands in a v0.1.x
  point release. Loader, binary format, round-trip, and version
  checks are production.
- **C FFI for Swift** — five symbols, two `#[repr(C)]` structs,
  distributed as a static lib + `solver.h` or a
  `PokerSolver.xcframework` (SwiftPM binary target).
- **Deterministic output** — same inputs + seed produce
  bit-identical strategies (MCCFR seeded via
  `Xoshiro256StarStar`).
- **`demo` CLI** — four canned spots render frequencies and EV in
  a colored terminal grid for quick sanity checks.

## Not in v0.1

- **Turn live solve** is wired but slow. Prefer cached paths.
- **Flop live solve** — cache-only. The MCCFR kernel exists;
  Colab precompute populates the cache.
- **Multi-way pots (3+ players)** — heads-up only.
- **PLO** — NLHE only. PLO is post-v1.0.
- **ICM tournament math** — cash-game EV only.
- **Exploit / node-locking** — pure GTO.

## How to integrate

Full guide: [INTEGRATION.md](INTEGRATION.md). Two paths:

1. **xcframework via SwiftPM** — pin
   `https://github.com/HenrySchlesinger/poker-solver` to `0.1.0`.
2. **Static lib + header** — `gh release download v0.1.0`, grab
   `libsolver_ffi.a` + `solver.h`, add them to your Xcode target.

Then: `solver_new()` once, `solver_solve()` per hand,
`solver_free()` on shutdown.

## Known performance

At river scale (N=1326 combos), one regret-matching call on
Apple-Silicon:

- Scalar: 1.77 µs
- **SIMD (`wide::f32x8`): 193 ns** — ships
- Metal GPU: 112 µs (~100 µs dispatch overhead dominates)

SIMD beats Metal ~580× on this single-spot workload. The Metal
module is gated behind the `metal` feature with a 10k-trial
equivalence test, held for future batched-kernel work.

## Reporting bugs

Open an issue using the `bug_report.md` template at
<https://github.com/HenrySchlesinger/poker-solver/issues>. Include
`solver_version()` output, the `HandState` that triggered it, and
the return code. For live-broadcast blockers, flag Henry directly.
