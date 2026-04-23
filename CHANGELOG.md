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

### Added — core algorithm (`solver-core`)

- CFR+ with regret matching: regret-sum + strategy-sum accumulators,
  linear-averaging strategy readout, and a chance-layer hook
  (`iterate_from` / `run_from`) so callers with chance priors above
  the `Game` root (Kuhn card deal, NLHE board cards) can fold priors
  into own-reach and counterfactual-reach.
- Kuhn Poker convergence test — four tests validate CFR+ against the
  analytically-known Nash equilibrium. Empirical exploitability
  ~0.00486 at 1000 iterations (well under the 0.01 target), Hero EV
  within 1% of -1/18, Villain strategy matches unique Nash
  frequencies, Hero alpha in the [0, 1/3] Nash family. This is the
  regression canary for all future CFR work.
- External Sampling MCCFR (CFR+ variant): regret clamp + linear
  averaging, generic over `Game`, with an `Xoshiro256StarStar` PRNG
  seeded per instance. Deterministic: seed `u64=0` produces
  bit-identical output across runs. `iterate_with` / `run_with`
  accept a closure that samples a fresh root per iteration, pushing
  the chance layer (e.g., turn river card) above the `Game` root.
- SIMD regret matching via `wide::f32x8`, with a scalar fallback and
  a 10k-trial equivalence property test covering lengths 1..2000,
  edge cases (all-zero, all-negative, single positive, NaN), and the
  1326-combo NLHE scale, asserting agreement within 1e-6 per element.
- Metal compute shader for regret matching, feature-gated behind
  `metal`. The 10k-trial equivalence test passes, but GPU dispatch
  overhead (~100 µs per call) dominates the ~193 ns compute at
  N=1326 — scalar is 1.77 µs and SIMD is 193 ns on the same machine.
  **SIMD beats Metal ~580× at river scale.** SIMD is the shipping
  path; Metal is retained for future batched-kernel work.

### Added — NLHE primitives (`solver-nlhe`)

- `NlheSubgame` with a `Game` impl for the river: chance-layer combo
  pair enumeration, a 1326×1326 `i8` showdown sign matrix, legal
  action generation via the bet tree, fold/showdown terminal
  handling, and a deterministic FNV-1a info-set hash.
- `Range` type with a 1326-weight vector backing and a text parser
  (`"AA, KK, AKs, T9s+"` style input).
- Preflop range ingestion pipeline — reads exported range files so
  the preflop lookup has real data to load.
- `BetTree` with the v0.1 default discretization (33% / 66% / pot /
  snap-jam) plus hooks for custom trees.
- `ActionLog` type with `SmallVec` backing so a typical hand's
  action history stays on the stack.
- Flop cache runtime loader + binary packer: `FlopCache` keyed by
  `(board, spr, pot_type, bet_tree_version)`, loaded from a
  `PSFLOP\0\0` packed binary. `pack_binary` serializes `PackEntry`
  tuples for shipping. `CachedFlopStrategy` holds per-action
  strategies, EV, and exploitability. 16 unit + 5 integration tests
  cover round-trip, truncation, version mismatch, and duplicate-key
  rejection.
- Shares `combo_index` with `solver-eval` so ranges and evaluators
  agree on how 1326 combos are numbered.

### Added — poker evaluation (`solver-eval`)

- `Card`, `Hand`, `Board`, `Combo` primitives — packed `u8` per card
  (rank in high 4 bits, suit in low 2).
- `rs_poker` wrapped as `eval_5` / `eval_7` instead of rolling our
  own 5-card lookup table. Per CLAUDE.md, we do not reimplement
  solved open-source primitives badly.
- Isomorphism: `canonical_board` and `canonical_spot` for suit
  canonicalization (the basis of the flop cache key).
- Hand-vs-hand and range-vs-range equity. Matrix-shaped
  range-vs-range is the floor the river CFR builds on.
- Differential test harness (`solver-eval-reference`): a test-only
  crate that ports Poker Panel's Python hand-ranking / equity /
  showdown math into an independent Rust implementation
  (categorical 5-card + C(7,5)=21 best-of for 7-card). 10,000
  random river scenarios confirm `eval_7` agreement with the oracle
  on both category and ordering — a bug in `rs_poker` cannot be
  blessed by both evaluators.

### Added — FFI and distribution (`solver-ffi`)

- `extern "C"` surface: `solver_new`, `solver_free`, `solver_solve`,
  `solver_lookup_cached`, `solver_version`, with `#[repr(C)]`
  `HandState` and `SolveResult` structs that match the header
  byte-for-byte.
- `build.rs` + `cbindgen.toml` regenerate
  `crates/solver-ffi/include/solver.h` on every build. The
  generated header is checked in so integrators (and Xcode) can
  consume it without running `cargo build` first.
- Rust-side FFI smoke tests that call every public symbol.
- Swift harness example at
  `crates/solver-ffi/examples/swift-harness/` — links the staticlib
  and calls every FFI symbol, proving the ABI loads from a real
  Swift consumer end-to-end (linker flags, header import, struct
  layout round-trip).
- Universal macOS build pipeline (`scripts/build-release.sh`):
  `lipo`'d arm64 + x86_64 `.a` / `.dylib` bundle with VERSION
  metadata, sha256 manifest, and tarball.
- GitHub Release publisher (`scripts/gh-release.sh`): `gh release
  create` wrapper that attaches the tarball + SHA sidecar +
  `solver.h`.
- `PokerSolver.xcframework` build
  (`scripts/build-xcframework.sh`): wraps the universal bundle into
  an xcframework via `xcodebuild -create-xcframework`, then tarballs
  and SHA256s it (SwiftPM requires `.zip`, also supported via
  modulemap). `Package.swift` flipped to the xcframework binary
  target. Sources/PokerSolver/PokerSolver.swift adds a thin Swift
  wrapper with a `PokerSolverStatus` enum and `PokerSolver.version`
  accessor.
- v0.1.0-dryrun verification: `build-release.sh` and
  `build-xcframework.sh` ran first-try against a scratch SwiftPM
  consumer — `swift build` succeeded in 3.92 s, the executable
  resolved `solver_version()` correctly. Artifact sizes + SHA-256s
  recorded in `docs/RELEASE_PROCESS.md`.

### Added — CLI tools (`solver-cli`)

- `solve` subcommand with JSON output, plus integration tests for
  the CLI binary. This is the dev harness — Poker Panel does not
  ship it.
- `translate-fixture` subcommand wired to the `translate` module
  for TexasSolver differential-test fixture conversion.
- `demo` subcommand — four spots (`royal`, `coinflip`,
  `bluff_catch`, `all`) with a polished renderer: cyan-triangle
  decision labels, aligned grid rows (labels / bars / percentages
  on one 4-space indent), color via the `colored` crate with
  auto-`NO_COLOR` / TTY detection.
- `md-to-ipynb` — build Colab notebooks from Markdown plans. Used
  to regenerate the three `colab/*.ipynb` files on demand.
- `seed-cache` — ship a v0.1 format-only flop cache (36 entries =
  12 boards × 3 SPR buckets × Srp, 374 KB). Round-trip-verified
  via `FlopCache::load_from_file`. Real GTO data replaces this on
  Day 5 from Colab precompute.

### Added — data

- Validation fixtures spots 001–020: dry flops (001–004), wet JT9
  (005), wet flops (006–008), paired boards (009–010), trips KKK
  (011), 3BPs (012–014), rivers (015, 016–020), extreme SPR
  spots. These seed the 20-spot TexasSolver validation battery.
- `data/flop-cache/flop-cache-v0.1.bin` — 374 KB shipped
  placeholder cache.
- `data/preflop-ranges/`, `data/iso-tables/` scaffolding.

### Added — docs

- `WHY.md`, `REQUIREMENTS.md`, `ROADMAP.md`, `ARCHITECTURE.md`,
  `POKER.md` (worked hands + game-tree diagram + position section),
  `ALGORITHMS.md`, `LIMITING_FACTOR.md`, `BENCHMARKS.md`
  (including the SIMD-vs-scalar-vs-Metal table and rationale),
  `COLAB.md`, `GETTING_STARTED.md`, `HARDWARE.md`, full `GLOSSARY.md`
  with cross-refs, `E2E_TESTING.md`.
- `INTEGRATION.md` — Poker Panel consumer spec for the v0.1 FFI.
- `RELEASE_NOTES_v0.1.md` — customer-facing v0.1 summary.
- `SHIP_V0_1.md` — exhaustive v0.1 ship checklist.
- `RELEASE_PROCESS.md` — tag → build → publish → verify runbook
  for non-expert operators, plus manual-consumer-integration notes.
- `DIFFERENTIAL_TESTING.md` — TexasSolver differential-test
  workflow and license handling.
- `POKER_PANEL_ORACLE.md` — Poker Panel poker-math oracle survey:
  what was ported, what was intentionally skipped, and handoff
  notes for Days 2–7.
- GitHub bug-report issue template.
- README: CI + Rust + license badges row, ship section,
  integrator quick-install, xcframework + manual-integration
  pointers, Colab one-liner, release-doc cross-links.

### Added — CI and tooling

- `.github/workflows/ci.yml`: fmt + clippy + test + bench-compile
  on every push and PR (macOS 14 runner, concurrency-cancelled).
- `.github/workflows/bench.yml`: nightly criterion runs with
  cached `main` baseline and HTML artifact upload.
- `.github/pull_request_template.md`: PR body prompts for required
  benchmark numbers and the roadmap task being fulfilled.
- Pre-commit hook (`.githooks/`) — install via
  `./.githooks/install.sh`, runs fmt + clippy gates locally before
  commit.
- `scripts/ship.sh` — end-to-end validation script chaining fmt /
  clippy / test / bench-compile / FFI-artifact gates.
- TexasSolver build scripts for macOS + Colab
  (`scripts/build_texassolver_macos.sh`, Colab equivalent),
  licensed under AGPL — **build artifacts are gitignored**, only
  the build scripts ship in-tree.
- TexasSolver differential-test harness
  (`crates/solver-cli/tests/texassolver_diff.rs`): enumerates
  `spot_NNN.json` fixtures, runs each through our solver and
  TexasSolver, parses both into a uniform `ActionSummary`, asserts
  per-action frequency deltas within `tolerances.action_freq_abs`
  and EV deltas within `tolerances.ev_bb_abs`. `#[ignore]`-gated so
  CI doesn't build TexasSolver; preflight gracefully skips when
  the TS binary / fixtures / python3 are absent.
- Python fixture translator (`scripts/translate_fixture.py`) with
  doctests for TexasSolver input conversion.
- Colab notebooks built from the `.md` plans, with Open-in-Colab
  badges: `convergence_bench.ipynb`, `precompute_flops.ipynb`,
  `precompute_preflop.ipynb` (nbformat v4.5, 7–8 cells each).
- Benchmarks: `cfr_kuhn` (CFR+ full-solve at 10 / 100 / 1000
  iters), SIMD vs scalar regret matching, `flat_vs_hashmap` table
  lookup, river inner-loop criterion harness.
- Workspace deps: `proptest`, `zstd`, `wide` (SIMD), `metal` +
  `objc` (feature-gated), `Xoshiro256StarStar` (deterministic PRNG).

### Added — project rules (`CLAUDE.md`)

- Rust-first rule and explicit policy on when shell / Python is
  acceptable.
- "No paid services" as a permanent project rule (no Colab Pro /
  Pro+, no paid compute tiers).
- Memory-awareness note for RAM-constrained local dev.
- Pre-commit hook install + executable documentation.

### Changed

- Toolchain bumped to 1.85.0 (`rs_poker` requires edition 2024).
  Prior pins (1.75 MSRV, 1.82.0 channel) noted in history only.

### Known gaps (tracked for `v0.1.0`)

- Turn live solve is wired but slow — use cached paths where
  possible.
- Three `river_canonical` CFR+ walk tests (`no_brainer_fold`,
  `even_match_is_symmetric`, `convergence_decreases_exploitability`)
  are `#[ignore]`d: they hit SIGKILL on 64 GB Apple-Silicon hosts
  (30 GB peak RSS in 193 s) and would OOM the 7 GB macOS-14 CI
  runners. Allocation rate (~155 MB/s) points at a runaway inside
  `CfrPlus::walk` or the `NlheSubgame` `Game` impl rather than
  tree size.
- Real Colab-generated flop-cache data (the shipped v0.1 cache is
  format-only placeholder).
- Full 20-spot TexasSolver validation battery run — the harness is
  in, pending fixture translation polish + TS EV parsing (tracked
  in `docs/DIFFERENTIAL_TESTING.md`).
- Multi-way (3+ player) pots, ICM, PLO, exploit / node locking —
  all post-v0.1.

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
