---
name: poker-solver
description: Use whenever working on the poker-solver Rust workspace at ~/Desktop/poker-solver/. Covers CFR+/MCCFR algorithms, NLHE game tree + bet-tree abstraction, card/range/equity primitives, the Swift FFI contract with Poker Panel, Colab precompute pipeline, and the 7-day sprint roadmap. Auto-loads for any mention of GTO, CFR, solver, or working in the poker-solver directory. Trigger if the user mentions: solver work, GTO computation, CFR, MCCFR, regret matching, bet-tree abstraction, NLHE game tree, card isomorphism, postflop solving, preflop ranges, the Rust → Swift FFI, Colab precompute, or any file under ~/Desktop/poker-solver/.
---

# poker-solver skill

You are working on the `poker-solver` Rust workspace — a local NLHE GTO
solver that feeds live broadcast overlays in Poker Panel via FFI.

## Critical context

- **Repo:** `~/Desktop/poker-solver/` (SEPARATE from `~/Desktop/Poker Panel/`)
- **Do NOT touch Poker Panel during this sprint.** It is shipping. Leave it
  alone. Integration work happens after v0.1 of the solver is stable.
- **Deadline:** 7-day sprint starting 2026-04-22. v0.1 ships end of
  2026-04-29.
- **Owner:** Henry (millex53@gmail.com) — not a coder; explain
  tradeoffs plainly.
- **Runs locally** on the user's Mac. No cloud at runtime. Colab is for
  offline precompute only.
- **Target hardware:** M-series MacBook / Mac Studio. Apple Silicon's
  unified memory is a real advantage for the vector CFR inner loop.

## Read these before doing work

Load whichever of these apply to the task in front of you. They live in
`~/Desktop/poker-solver/docs/`:

- **`WHY.md`** — product context, build-vs-buy decision, market
- **`REQUIREMENTS.md`** — performance targets, functional requirements
- **`ROADMAP.md`** — 7-day plan, parallel streams, your day's work
- **`ARCHITECTURE.md`** — crate layout, FFI contract, data flow
- **`POKER.md`** — NLHE primer for non-poker engineers
- **`ALGORITHMS.md`** — CFR+/MCCFR/Vector CFR reference
- **`LIMITING_FACTOR.md`** — the critical path (river inner loop latency)
- **`BENCHMARKS.md`** — what "fast enough" means and how to measure it
- **`GLOSSARY.md`** — keep this open
- **`COLAB.md`** — when to use Colab vs local compute
- **`HARDWARE.md`** — Mac specifics, Metal, Apple Silicon tricks
- **`GETTING_STARTED.md`** — how to pick a task and validate your work

## Default behaviors

1. **Every change must pass `cargo test --workspace`** before commit. The
   convergence tests are the guardrail — they catch algorithm regressions.
2. **Every performance claim must have a criterion benchmark.** Don't say
   "this is faster" — measure it with `cargo bench` and paste the output.
3. **Don't regress the river inner loop without a clear reason.** That
   benchmark is the product's reason for existing.
4. **Prefer packed `#[repr(C)]` structs and fixed arrays over `Vec` and
   smart types** on hot paths. Cache-friendliness matters more than
   elegance this sprint.
5. **Validate against TexasSolver on canonical spots.** If our output
   drifts from TexasSolver by more than 5%, something is wrong with the
   regret accumulation or the abstraction.

## Dispatch patterns for the 10-agent sprint

When Henry is running many agents in parallel, each agent should:
- Claim ONE crate or ONE doc section at a time (check ROADMAP.md for
  what's in flight)
- Work on `main`, not a feature branch (small commits, fast merges)
- Write a criterion bench before optimizing
- Check `cargo fmt --all && cargo clippy --all-targets` before committing

If a task spans multiple crates, break it into per-crate PRs so parallel
agents don't collide on the same file.

## What to NOT do

- Don't reach for Metal compute shaders before SIMD in Rust is profiled
  and shown insufficient. Day 1–3 is pure Rust.
- Don't build async/tokio anything — CFR is CPU-bound, async is a footgun.
- Don't add a CLI flag for every option. CLI is a dev tool, not a product.
- Don't write a new hand evaluator. Wrap `rs-poker` or port the bit tricks
  from `poker-eval`. Hand evaluation is a solved problem; don't redo it
  badly.
- Don't edit anything in `~/Desktop/Poker Panel/`. That repo is locked
  until v0.1 of the solver ships.

## Key algorithms at a glance

- **CFR+** — the default. Regret-matching variant that ignores negative
  regrets. Fast convergence on most spots.
- **MCCFR (External Sampling)** — for trees too big for vanilla CFR.
  Sample villain's actions, enumerate hero's. We'll use this on the turn.
- **Vector CFR** — at the river, every hand is a showdown. The whole
  strategy update becomes a 1326×1326 matrix operation. This is the hot
  path; SIMD/Metal accelerate it.
- **Discounted CFR+** — post-v0.1 optimization. Faster convergence with a
  small extra term in the regret update.

Full details in `docs/ALGORITHMS.md`.
