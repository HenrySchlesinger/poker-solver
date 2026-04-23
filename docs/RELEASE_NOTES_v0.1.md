# poker-solver v0.1.0 — release notes

**Released:** TBD 2026-04-29
**Tag:** `v0.1.0`

## The headline

v0.1.0 is the **first working GTO solver built for live broadcast
overlays**. It ships as a local library — no cloud, no API keys, no
per-hand fees. Poker Panel consumes it via a C FFI to render
GTO-optimal strategies onto a live broadcast in real time.

Existing tools either cost thousands per year and don't do GTO
(PokerGFX, $999–$9,999/yr), or do GTO but won't partner with indie
broadcasters (GTO Wizard's GGPoker exclusive). v0.1.0 closes that gap.

## What works today

- **River solve** — CFR+ converges on any legal NLHE river spot
  inside the latency budget an overlay needs to feel instant.
- **Preflop** — static range lookup from shipped data.
- **Flop** — precomputed cache lookup; offline solves populate it.
- **C FFI for Swift** — five symbols, two structs, Swift-Package-
  Manager or bridging-header friendly.
- **Deterministic output** — same inputs and seed → bit-identical
  strategies.

## Known gaps

v0.1 is deliberately narrow. These are v0.2+:

- **Turn live solve** is present but slow. Use the cached path
  where possible.
- **Metal compute shader** — skipped; Rust SIMD hit target.
- **Multi-way pots** (3+ players) — not in v0.1. Heads-up only.
- **ICM tournament math** — cash-style EV only for v0.1.
- **PLO** — NLHE only; PLO is post-v1.0.
- **Exploit / node locking** — GTO only, no opponent modeling.

## How to integrate

See `docs/INTEGRATION.md`. Short version: grab `libsolver_ffi.a` and
`solver.h` from this release, link them into your Xcode target, call
`solver_new()` once, `solver_solve()` per hand, `solver_free()` on
shutdown.

## How to report bugs

Open an issue at
<https://github.com/henryschlesinger/poker-solver/issues> using the
`bug_report.md` template. Include `solver_version()` output, the
`HandState` that triggered it, and the return code. For critical
broadcast blockers, flag Henry directly.
