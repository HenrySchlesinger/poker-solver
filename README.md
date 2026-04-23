# poker-solver

A local NLHE GTO solver, built to power live-broadcast overlays in
[Poker Panel](~/Desktop/Poker%20Panel) — the $60/mo macOS streaming app.

**Owner:** Henry (millex53@gmail.com)
**Started:** 2026-04-22
**Status:** Week-1 sprint in progress. Not yet consumed by Poker Panel.

## What this is

A Rust workspace that compiles to a static library + C header that Swift
consumes via FFI. Given a hand state (board, ranges, pot, stacks), it
returns GTO-optimal action frequencies for the hero. Runs entirely on the
user's Mac — no cloud, no API calls at runtime.

## Why this exists

PokerGFX charges $999–$9,999/yr and ships no GTO. Vizrt charges $1,000+/mo
and has zero poker-specific products. GTO Wizard has an exclusive GGPoker
broadcast deal and won't partner with a $60/mo indie tool. The only way for
Poker Panel to have real-time GTO overlays is to own the compute.

Longer version: [docs/WHY.md](docs/WHY.md).

## Quick start

```bash
cargo build --release --workspace
cargo test --workspace
cargo bench -p solver-core           # river inner-loop benchmarks
cargo run -p solver-cli -- solve \
    --board AhKh2s \
    --hero-range "AA,KK,QQ,AKs" \
    --villain-range "22+,A2s+,K9s+" \
    --pot 100 --stack 1000
```

## Layout

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). TL;DR:

```
crates/
  solver-core/    # CFR+/MCCFR, regret matching, convergence metrics
  solver-nlhe/    # NLHE game tree, bet-tree abstraction, ranges
  solver-eval/    # hand evaluator, equity, card isomorphism
  solver-ffi/     # C FFI surface — the contract with Poker Panel
  solver-cli/     # dev harness + Colab driver
```

## Roadmap

7-day sprint. [docs/ROADMAP.md](docs/ROADMAP.md) has the day-by-day.

| Day | Ship |
|---|---|
| 1 | Repo scaffold, CFR+ on Kuhn Poker, eval/range/iso crates |
| 2 | NLHE river subgame, vanilla CFR+ correctness |
| 3 | Vector CFR on river, SIMD inner loop |
| 4 | Turn solver (MCCFR), Swift FFI |
| 5 | Colab flop precompute kicks off (runs overnight), preflop import |
| 6 | End-to-end integration test with mock Poker Panel consumer |
| 7 | v0.1, convergence validated, benchmarks locked |

## The limiting factor

Convergence speed on the river inner loop. Everything else is tractable.
See [docs/LIMITING_FACTOR.md](docs/LIMITING_FACTOR.md).

## License

Proprietary. Not for redistribution. This is a component of Poker Panel.
