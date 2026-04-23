# Changelog

All notable changes to `poker-solver` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

While we are pre-1.0, breaking changes to the FFI surface or on-disk data
formats may appear in any minor version. Once `v1.0.0` ships, we follow
semver strictly and tie it to `solver_version()` on the consumer side.

## [Unreleased]

This section tracks work landed on `main` that is NOT yet tagged as a
release. It will be rolled into the next version on tag day.

### Added

- Initial Cargo workspace scaffold with 6 member crates: `solver-core`,
  `solver-nlhe`, `solver-eval`, `solver-eval-reference`, `solver-ffi`,
  `solver-cli`. Workspace version pinned to `0.1.0`, MSRV `1.75`,
  toolchain pinned to `1.82.0` via `rust-toolchain.toml`.
- `solver-core`: CFR+ with regret matching, including the regret-sum
  + strategy-sum accumulators and linear-averaging strategy readout.
  Chance-layer hook wired up for games that deal cards.
- `solver-core`: Kuhn Poker convergence test — exercises the generic
  `trait Game` surface and validates equilibrium values within the
  expected tolerance. Load-bearing: this is the canary for any future
  CFR regression.
- `solver-nlhe`: `Range` type with 1326-weight vector backing and a
  text parser (`"AA, KK, AKs, T9s+"` style input).
- `solver-nlhe`: preflop range ingestion pipeline (reads exported
  range files so the Day 5 preflop lookup has real data to load).
- `solver-nlhe`: `ActionLog` type with a `SmallVec` backing so a typical
  hand's action history stays on the stack.
- `solver-nlhe`: `BetTree` with the v0.1 default discretization (33% /
  66% / pot / snap-jam) plus hooks for custom trees.
- `solver-nlhe`: shares a canonical `combo_index` with `solver-eval`
  so ranges and evaluators agree on how 1326 combos are numbered.
- `solver-eval`: `Card`, `Hand`, `Board`, `Combo` primitives; the
  representation is a packed `u8` per card (rank in high 4 bits, suit
  in low 2).
- `solver-eval`: wraps `rs-poker` as `eval_5` / `eval_7` instead of
  rolling our own 5-card lookup table. Per CLAUDE.md, we do not
  reimplement solved open-source primitives badly.
- `solver-eval`: isomorphism — `canonical_board` and `canonical_spot`
  for suit canonicalization (needed for the flop cache).
- `solver-eval`: hand-vs-hand equity and range-vs-range equity.
  Matrix-shaped range-vs-range is the floor the river CFR builds on.
- `solver-ffi`: `extern "C"` surface — `solver_new`, `solver_free`,
  `solver_solve`, `solver_lookup_cached`, `solver_version` — with
  `#[repr(C)]` `HandState` and `SolveResult` structs that match the
  header byte-for-byte.
- `solver-ffi`: `build.rs` + `cbindgen.toml` regenerate
  `crates/solver-ffi/include/solver.h` on every build. The generated
  header is checked in so integrators (and Xcode) can consume it
  without running `cargo build` first.
- `solver-ffi`: Rust-side FFI smoke tests that call every public
  symbol from Rust (independent of the Swift harness).
- `solver-ffi`: Swift harness example at
  `crates/solver-ffi/examples/swift-harness/` that links the staticlib
  and calls every FFI symbol. Proves the ABI is loadable from a real
  Swift consumer end-to-end (linker flags, header import, struct
  layout round-trip).
- `solver-cli`: `solve` subcommand with JSON output, plus integration
  tests for the CLI binary. This is the dev harness — Poker Panel
  does not ship it.
- Fixtures: spots 001–005 (dry flops 001–004, wet JT9 005). These
  seed the validation harness; the full 20-spot validation battery
  lands later in the sprint.
- `docs/`: `WHY.md`, `REQUIREMENTS.md`, `ROADMAP.md`,
  `ARCHITECTURE.md`, `POKER.md` (worked hands + game-tree diagram +
  position section), `ALGORITHMS.md`, `LIMITING_FACTOR.md`,
  `BENCHMARKS.md`, `COLAB.md`, `GETTING_STARTED.md`, `HARDWARE.md`,
  and a full `GLOSSARY.md` with cross-refs.
- `.github/workflows/ci.yml`: fmt + clippy + test + bench-compile on
  every push and PR (macOS 14 runner, concurrency-cancelled).
- `.github/workflows/bench.yml`: nightly criterion runs with cached
  `main` baseline and HTML artifact upload.
- `.github/pull_request_template.md`: PR body prompts for the required
  benchmark numbers and the roadmap task being fulfilled.
- `CLAUDE.md`: workflow rules for the 10 parallel agents, plus the
  Rust-first rule and the explicit policy on when shell/Python is
  acceptable.

### Known gaps (tracked for `v0.1.0`)

- River Vector-CFR SIMD path (Day 3 deliverable — open).
- Turn MCCFR path and Swift FFI happy-path end-to-end (Day 4 — open).
- Colab flop-cache precompute + preflop static data file (Day 5 — open).
- 20-spot TexasSolver validation battery (Day 6 — open).
- First batch of Colab-generated flop-cache entries in
  `data/flop-cache/` (Day 7 — open).

---

## [0.1.0] — TBD 2026-04-29

First tagged release. Ships the local NLHE GTO solver that Poker
Panel consumes via FFI. Planned, not yet cut. Full notes will be
copied in here from `[Unreleased]` on tag day plus anything added
between now and then.

See `docs/SHIP_V0_1.md` for the exhaustive gate list and
`docs/RELEASE_NOTES_v0.1.md` for the customer-facing summary.

[Unreleased]: https://github.com/henryschlesinger/poker-solver/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/henryschlesinger/poker-solver/releases/tag/v0.1.0
