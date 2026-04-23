# poker-solver

[![CI](https://github.com/henryschlesinger/poker-solver/actions/workflows/ci.yml/badge.svg)](https://github.com/henryschlesinger/poker-solver/actions/workflows/ci.yml)
[![Rust: 1.82](https://img.shields.io/badge/rust-1.82-orange.svg)](./rust-toolchain.toml)
[![License: Proprietary](https://img.shields.io/badge/license-proprietary-red.svg)](#license)

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
./.githooks/install.sh               # install pre-commit hook (once per clone)
cargo build --release --workspace
cargo test --workspace
cargo bench -p solver-core           # river inner-loop benchmarks
cargo run -p solver-cli -- solve \
    --board AhKh2s \
    --hero-range "AA,KK,QQ,AKs" \
    --villain-range "22+,A2s+,K9s+" \
    --pot 100 --stack 1000
```

Before tagging a release, run `scripts/ship.sh` — it chains the fmt /
clippy / test / bench-compile / ffi-artifact gates in one go. See
[docs/SHIP_V0_1.md](docs/SHIP_V0_1.md) for the full ship checklist
and [docs/MORNING_BRIEF.md](docs/MORNING_BRIEF.md) for the latest
day's audit of what's ship-ready vs open.

Run precompute on Colab (free T4 GPU) — see [colab/README.md](colab/README.md) for one-click notebooks.

## Quick install (Poker Panel integrators)

Full guide: [docs/INTEGRATION.md](docs/INTEGRATION.md). TL;DR:

```bash
# Grab the release artifact
gh release download v0.1.0 \
    --repo henryschlesinger/poker-solver \
    --pattern 'libsolver_ffi.a' \
    --pattern 'solver.h'
```

Then in your Xcode target: add `libsolver_ffi.a` to **Frameworks,
Libraries, and Embedded Content**, point your bridging header at
`solver.h`, and call:

```swift
let handle = solver_new()
defer { solver_free(handle) }
var result = SolveResult()
if solver_solve(handle, &state, &result) == Ok {
    overlay.render(result)
}
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

## Release + integration docs

- [CHANGELOG.md](CHANGELOG.md) — what landed in each tag.
- [docs/SHIP_V0_1.md](docs/SHIP_V0_1.md) — ship checklist for v0.1.
- [docs/RELEASE_PROCESS.md](docs/RELEASE_PROCESS.md) — tag → build → publish runbook.
- [docs/INTEGRATION.md](docs/INTEGRATION.md) — Poker Panel consumer guide.
- [docs/RELEASE_NOTES_v0.1.md](docs/RELEASE_NOTES_v0.1.md) — customer-
  facing summary for the v0.1 tag.

## Installing for Poker Panel integration

Once v0.1.0 is tagged and released, Swift Package Manager consumers can
pin to the published release (`crates/solver-ffi/Package.swift` is the
scaffold — see [docs/RELEASE_PROCESS.md](docs/RELEASE_PROCESS.md) for
the full xcframework wrapping status):

```swift
.package(url: "https://github.com/HenrySchlesinger/poker-solver", from: "0.1.0")
```

For the v0.1 manual-integration path (static lib + header, no SwiftPM
binary target yet), the commands are:

```bash
VERSION=v0.1.0
gh release download "$VERSION" \
    --repo HenrySchlesinger/poker-solver \
    --pattern "solver-$VERSION-macos-universal.tar.gz*"
shasum -a 256 -c "solver-$VERSION-macos-universal.tar.gz.sha256"
tar xzf "solver-$VERSION-macos-universal.tar.gz"
# -> solver-v0.1.0/lib/libsolver_ffi.{a,dylib}
# -> solver-v0.1.0/include/solver.h
```

Then follow the Xcode drop-in steps in
[docs/RELEASE_PROCESS.md](docs/RELEASE_PROCESS.md#manual-a-drop-in-for-xcode-without-spm-setups).

## License

Proprietary. Not for redistribution. This is a component of Poker Panel.
