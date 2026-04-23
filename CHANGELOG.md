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
- `CfrPlusFlat` — flat-array `RegretTables` variant of CFR+, built for
  cache-friendliness over the HashMap-backed reference `CfrPlus` (A64).
  `CfrPlusFlat::walk` calls `regret_match_simd` internally, so future
  wide-action layouts vectorize automatically; for the current NLHE
  bet trees (≤5 actions) it short-circuits to scalar below
  `SIMD_THRESHOLD=8`. Convergence is guarded against the classic
  implementation by `tests/flat_equivalence.rs` (10k-iter Kuhn at 1e-6
  tolerance). On the real NLHE river benches: −9.1% on
  `river_canonical_spot`, −26.2% on `river_degenerate_spot`, −11.7%
  on `river_wet_board` — the flat layout, not SIMD, carried the gain.
  The remaining ~15× gap to the `<300 ms @ 1000 iters` v0.1 perf
  target is documented as v0.2 work (Vector CFR or rayon tree-walk).

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
- River CFR+ OOM fix (A58): `NlheSubgame::apply` now translates a bare
  `Action::AllIn` into a concrete `Bet(stack_start)` or
  `Raise(stack_start)` at subgame-build time. Previously the river
  action-log's `pot_contributions_on` returned `(0, 0)` for AllIn,
  causing `legal_river_actions` to re-enter the "no aggression yet"
  branch and emit another {Check, Bet, AllIn} on a subsequent pass —
  producing an unbounded tree and >30 GB RSS OOM on any non-trivial
  spot. River solves with `stack > 0` are now bounded; the blast
  radius is narrow (no changes to `ActionLog` or
  `pot_contributions_on`). Two of the three `river_canonical` walk
  tests that A56 tracked as `#[ignore]` can now run on non-zero
  stacks; remaining ignores are documented under `Known gaps`.

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
- `solver_solve` wired end-to-end (A59). Replaces the prior stub with
  a real dispatch into `solver_core::CfrPlus` (and post-A64,
  `CfrPlusFlat`) + `solver_nlhe::NlheSubgame`, mirroring the
  `solver-cli solve` pipeline so FFI and CLI agree numerically. Null
  pointers return `InvalidInput`; `HandState` is validated
  (`board_len==5`, `bet_tree_version==0`, `to_act∈{0,1}`, board cards
  in `0..52` distinct, ranges non-empty); the CFR walk runs on a
  128MB-stack worker thread (same pattern as `solve_cmd.rs`) to
  avoid blowing macOS's 8MB default stack. Hardcoded at 100
  iterations for v0.1; a TODO in `solver-ffi/src/lib.rs` flags the
  v0.2 ABI bump that adds an `iterations` field to `HandState`.
  Clippy clean, `#[doc] Safety` blocks on every `unsafe` symbol,
  happy-path smoke tests in `solver-ffi/tests/`.
- Regenerated `crates/solver-ffi/include/solver.h` post-A59: parameter
  rename `_input`→`input`, `_output`→`output` (cbindgen regen after
  the stub was replaced), expanded doc-comments with v0.1 caveats +
  Safety blocks. The stale "stack=0 until A58 lands" caveat that A59
  wrote was updated once A58 actually landed.
- v0.1.0-test2 dress rehearsal (A60): a second xcframework scratch
  consumer, this time invoking `solver_solve` on the royal-tie spot
  and asserting `hero_equity ~ 0.5`. Artifact sizes, SHA-256s, and
  live-consumer output appended to `docs/RELEASE_PROCESS.md`. Three
  FFI paths are now verified green: Rust smoke test, Swift harness,
  SwiftPM xcframework consumer.

### Added — CLI tools (`solver-cli`)

- `solve` subcommand with JSON output, plus integration tests for
  the CLI binary. This is the dev harness — Poker Panel does not
  ship it. Wired end-to-end (A47 `3328a99`): `--stack 0` is now
  accepted (it's the only river configuration that solves cleanly
  under the v0.1 bet tree), the end-to-end fixture was replaced
  with the "trivial all-in showdown" (hero=AhKh, villain=AsAd on
  2c7d9hTsJs, stack=0) that collapses to Check/Check → showdown, and
  the royal-vs-royal tie spec spot reports `hero_equity=0.5`,
  `exploitability=0.0`, `compute_ms=7`. JSON output now populates
  `action_frequencies`, `ev_per_action`, `hero_equity`,
  `exploitability`, `iterations`, `compute_ms`, and `solver_version`.
- `--solver flat|classic` flag on `solve` (A64). Defaults to `flat`
  (`CfrPlusFlat` with flat `RegretTables` + SIMD regret matching).
  `classic` keeps the HashMap-backed `CfrPlus` reference
  implementation available as an escape hatch for reproducibility
  checks.
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
- Real NLHE river benches (A62): `benches/river.rs` swapped from a
  Kuhn placeholder to genuine `NlheSubgame` + `chance_roots` + CFR+
  end-to-end walks. Three spots:
  - `river_canonical_spot` — AhKhQh2d4s, AA/KK/AKs vs 22+/AJs+/KQs,
    100 iters, 478.23 ms baseline → 434.65 ms post-A64 flat+SIMD.
  - `river_degenerate_spot` — already-all-in shape, 1000 iters,
    363.13 µs baseline → 268.15 µs post-A64 (−26.2%). 137× under the
    50 ms target.
  - `river_wet_board` — JhTh9c8h7s, AA/AKs/QTs vs 22+/AQs+, 100
    iters, 667.14 ms baseline → 589.27 ms post-A64 (−11.7%).
  Canonical + wet stay at 100 iters because the pre-SIMD scalar inner
  loop is ~5–7 ms/iter; 1000 × 100 samples would run 500–700 s per
  bench. Full table with captured-date snapshots lives in
  `docs/BENCHMARKS.md` and `bench-history/`.
- TexasSolver oracle coverage extended from 1 → 5 river fixtures
  (A63). `crates/solver-cli/tests/fixtures/oracle_outputs/` now holds
  `spot_015` (A50), `spot_016`, `spot_017`, `spot_018`, `spot_020`
  reference outputs: TexasSolver's `.our.json`,
  `.texassolver.json`, `.ts-log.txt`, and `.tsconfig` for each
  fixture, plus an extended Status table in `docs/DIFFERENTIAL_TESTING.md`
  covering TS iteration count / convergence time, our-side
  `compute_ms`, and an auto-diff-ready flag. All five share the same
  three A50-flagged blockers to full auto-diff (combo rollup,
  bet-size name map, log-line EV parse).
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

- Toolchain pinned to 1.85.0 (`rs_poker` requires edition 2024).
  Prior pins (1.75 MSRV, 1.82.0 channel) noted in history only.
- `solver_version()` bumped `"0.1.0-wip"` → `"0.1.0-dev"` when
  `solver_solve` was wired (A59). The tag-day flip to `"0.1.0"` is
  tracked under `Known gaps for v0.1 tag`.
- Default CLI solver is now `CfrPlusFlat` (A64); `--solver classic`
  remains available as the HashMap-backed reference fallback. Same
  switch applies inside `solver-ffi::solver_solve`, guarded by
  `tests/flat_equivalence.rs` on Kuhn and by `solver-cli/tests/cli.rs`
  on NLHE river for JSON shape.

### Known gaps (tracked for `v0.1.0`)

- Turn live solve is wired but slow — use cached paths where
  possible.
- Three `river_canonical` CFR+ walk tests (`no_brainer_fold`,
  `even_match_is_symmetric`, `convergence_decreases_exploitability`)
  were `#[ignore]`d on 64 GB Apple-Silicon hosts and macOS-14 CI
  runners. A58's AllIn → Bet/Raise translation fix addresses the
  root cause (runaway action-tree re-expansion); the ignores remain
  until A58 is corroborated against the full CI runner memory
  budget on tag day.
- Real Colab-generated flop-cache data (the shipped v0.1 cache is
  format-only placeholder).
- Full 20-spot TexasSolver validation battery run — the harness is
  in, 5 of 20 fixtures have reference outputs captured (A63), full
  auto-diff pending fixture translation polish + TS EV parsing
  (tracked in `docs/DIFFERENTIAL_TESTING.md`).
- Multi-way (3+ player) pots, ICM, PLO, exploit / node locking —
  all post-v0.1.

### Known gaps for v0.1 tag

Five concrete blockers A61's ship audit identified — these are the
items that must clear before `v0.1.0` is cut, distinct from the
out-of-scope `Known gaps (tracked for v0.1.0)` above.

1. **`solver_version()` flip** — bump the string from `"0.1.0-dev"`
   to `"0.1.0"` on the tag commit
   (`crates/solver-ffi/src/lib.rs:222`).
2. **Real Colab preflop-v0.1.bin generation** — loader is in,
   `data/preflop-ranges/` is `.gitkeep`-only. Blocked on Henry to
   run the precompute Colab notebook.
3. **Vector CFR perf work** — v0.2 path, flagged by A64. The
   post-A64 `river_canonical_spot` extrapolates to ~4.35 s @ 1000
   iters, ~15× over the `<300 ms @ 1000 iters` v0.1 target. Closing
   the gap needs either (a) vector-CFR layout so SIMD bites at
   N=1326, or (b) rayon tree-walk fan-out. Documented in
   `docs/BENCHMARKS.md#why-the-jump-is-smaller-than-expected`.
4. **Extend TexasSolver diff from 5 → 20 spots** — A63 captured 5
   fixtures (`spot_015`–`spot_020` minus `spot_019`). The remaining
   15 spots plus full auto-diff of all 20 lands before tag day.
5. **Exploitability = 101 on `spot_018`** — flagged by A63 / A60, a
   possible correctness bug on the quads-possible
   KhKdKc-2s-4h river. Needs triage before tag day; our solver
   takes 31.5 s on this fixture where TexasSolver converges to
   0.28% in 133 ms, so the discrepancy is worth isolating.

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
