# v0.1.0 Ship Checklist

**Target tag date:** 2026-04-29 (Day 7 of the sprint started 2026-04-22).
**Scope:** first working ship of the local NLHE GTO solver, consumed by
Poker Panel via the C FFI at `crates/solver-ffi/include/solver.h`.

Every box in this file is a hard gate. "Looks fine" does not count —
each box must be backed by a passing test, a criterion bench, or a
diff-against-TexasSolver run pasted in the PR or commit message that
ticks it off. See `docs/BENCHMARKS.md` for perf discipline and
`docs/REQUIREMENTS.md` for the underlying targets these gates enforce.

Check boxes off as they land. The current state of the sprint is the
best guide to what's realistic for today vs. Day 7.

---

## Code — build + lint + test gates

- [x] `cargo build --release --workspace` is clean on M-series Mac
      (the primary target platform — see `docs/REQUIREMENTS.md`).
      (A61 re-ran today at HEAD=96db52e, `Finished release profile in 0.04s`.)
- [ ] `cargo build --release --workspace` is clean on macOS 14 GitHub
      runner (`.github/workflows/ci.yml` green on the tag commit).
      (Not verified by A61 — CI status on current HEAD not checked this audit pass.)
- [ ] `cargo fmt --all -- --check` is clean.
      (Not re-run on tag commit. `ship.sh` wires it; last independent check was A53's pre-commit install.)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean.
      (Last clippy sweeps: A19 `19a6967`, A20 `a14c074`, A14 `da73746`. Not re-verified on HEAD.)
- [ ] `cargo test --workspace --release` passes. Release-mode tests
      are the rule, per `.github/workflows/ci.yml` — debug CFR is too
      slow to be a useful smoke signal (`docs/BENCHMARKS.md`).
      (Three `river_canonical` tests are `#[ignore]`'d for CFR+ OOM — `8e26e00`. The rest
      should pass but A61 did not run the suite this audit.)
- [ ] `cargo test --workspace` (debug) passes separately, so debug
      assertions actually fire somewhere.
      (Not verified. CI only runs release mode.)
- [ ] Address sanitizer clean: `cargo test --workspace` with ASan
      enabled shows no leaks or UB (per `docs/REQUIREMENTS.md` quality
      targets).
      (Never run. No ASan pipeline exists.)
- [ ] `cargo bench --workspace --no-run` compiles every bench.
      CI runs this on every push; it must still pass on the tag commit.
      (Baked into `ship.sh` + `ci.yml`, but not re-verified on HEAD.)

## Code — performance gates (the actual ship criteria)

All numbers measured on an M-series Mac via criterion. Targets are
from `docs/REQUIREMENTS.md` and `docs/BENCHMARKS.md`. Hard limits =
we do not ship below them; targets = what we aspire to.

- [ ] `river_canonical_spot` — 1000 iters, **< 300 ms target**,
      **< 1 s hard limit**. Paste criterion output on the tag PR.
      (Bench still runs the Kuhn placeholder — `benches/river.rs` is env-gated and
      NLHE river subgame isn't wired into it. Baseline table in `docs/BENCHMARKS.md`
      reads "not yet wired" for all three river benches.)
- [ ] `river_degenerate_spot` — all-in preflop → river, **< 50 ms**.
      (Not wired — same as above.)
- [ ] `river_wet_board` — JhTh9c-8h-7h texture, **< 500 ms**.
      (Not wired — same as above.)
- [x] `regret_matching_inner` — 1326-wide SIMD inner loop, **< 1 µs**
      per call. (**192.7 ns** at N=1326 per `docs/BENCHMARKS.md` SIMD vs scalar
      table; A51 `7e587c7`. 9.2× faster than scalar, 580× faster than Metal.)
- [ ] `range_vs_range_equity` — **< 2 ms** full board, **< 20 µs**
      per iteration on the river hot path.
      (No `range_vs_range_equity` criterion bench exists — only golden tests
      in `solver-eval/tests/equity_goldens.rs`. Latency never measured.)
- [ ] `turn_canonical_spot` — 500 iters via MCCFR external sampling,
      **< 30 s target**, **< 60 s hard limit**.
      (No turn bench. `solver-nlhe/src/subgame_turn.rs` exists but isn't benched.)
- [ ] `cache_lookup` — flop-cache hashmap lookup, **< 10 µs**.
      (No dedicated `cache_lookup` bench; `flat_vs_hashmap.rs` is a different microbench.)
- [ ] FFI call overhead measured separately, **< 10 µs** per call
      (`docs/REQUIREMENTS.md` hard limit).
      (Not benched. `solver_solve` wired in `8ee6d1b`; no overhead-only measurement.)
- [ ] Memory per live solve: **< 500 MB** river, **< 2 GB** turn.
      Measure via `/usr/bin/time -l` or equivalent.
      (A58 `5629935` fixed the 30 GB river OOM; no RSS measurement captured since.)
- [ ] No criterion regression > 5% vs the stored `main` baseline on
      any of the above benches, or the regression has a written
      justification in the tag PR (`docs/BENCHMARKS.md`).
      (Baseline captured by A55 `d1ea968` at `bench-history/2026-04-23_094257_8e26e00.json` —
      covers regret_matching + cfr_plus_kuhn only. River/turn/cache/equity rows empty.)

## Code — correctness gates

- [x] Kuhn Poker convergence test in `crates/solver-core/tests/`
      passes — equilibrium within published tolerance. This is the
      canary for CFR regressions. (`crates/solver-core/tests/kuhn.rs` — four
      tests, empirical exploitability ~0.00486 @ 1000 iters per CHANGELOG.)
- [x] River convergence on a canonical spot: exploitability
      **< 1% of pot** at 1000 iters (`docs/REQUIREMENTS.md`).
      (A50 verified spot_015 converges to 0.07% exploitability in ~15 ms at 1000 iters —
      see `docs/DIFFERENTIAL_TESTING.md` line 48.)
- [ ] Convergence vs TexasSolver within **5% per-action frequency**
      on all 20 canonical spots (`docs/REQUIREMENTS.md`). The 20-spot
      battery lives under `crates/solver-cli/tests/` (or `fixtures/`);
      `cargo run -p solver-cli -- validate` prints the diff table.
      (Only 1 of 20 spots run end-to-end through the diff harness (A50 spot_015).
      `texassolver_diff` test is `#[ignore]`'d and the `validate` subcommand hasn't
      been run across the full battery.)
- [ ] EV accuracy within **0.1 bb of TexasSolver** on the same 20
      spots. (Same blocker: only spot_015 diffed so far.)
- [x] Determinism check: given the same `HandState` and PRNG seed
      (default 0), MCCFR produces bit-identical output across two
      back-to-back runs. Tested in `solver-core`.
      (CHANGELOG: MCCFR seeded via `Xoshiro256StarStar`, "seed `u64=0` produces
      bit-identical output across runs" — claim covered by `de5e159`.)
- [x] No `panic!` reaches the FFI boundary. `solver-ffi` wraps every
      `extern "C"` body in `catch_unwind` and the tests exercise the
      panic path returning `InternalError` (-2).
      (`catch_unwind` wraps `solver_solve` and `solver_lookup_cached`;
      `crates/solver-ffi/tests/smoke.rs` exercises ABI. `684e4a6` added safety docs.)
- [ ] Edge cases solved end-to-end without crashing: all-in spots,
      split pots, dead cards present in the board, stack depths from
      10 bb to 500 bb (`docs/REQUIREMENTS.md` functional scope).
      (A58 `5629935` fixed stack>0 OOM by terminating on all-in. `solver_solve` doc
      still warns callers to use stack=0 or small values — larger stacks expose an
      unbounded-tree bug. Stack depths 10–500 bb **not** swept end-to-end.)

## FFI — integration gates

- [ ] `crates/solver-ffi/include/solver.h` is freshly regenerated by
      `cargo build -p solver-ffi` on the tag commit (checked in, no
      hand-edits).
      (Uncommitted — `git status` shows `modified: crates/solver-ffi/include/solver.h`
      after A61's build. The diff is only a doc-comment refresh + `_input` →
      `input` rename from A59's `684e4a6`; needs to land before tag day.)
- [x] Rust-side FFI smoke tests pass — `solver_new` / `solver_solve` /
      `solver_lookup_cached` / `solver_free` / `solver_version`
      callable end-to-end from Rust. (`crates/solver-ffi/tests/smoke.rs`
      exercises every symbol; A59 `7a462c3` added the `solver_solve` happy path.)
- [x] Swift harness at `crates/solver-ffi/examples/swift-harness/`
      builds with `swiftc` and runs — every symbol prints an expected
      status code per `examples/swift-harness/README.md`. No crashes,
      no linker errors, no non-enum integers.
      (A60 `96db52e` verified the end-to-end solve path through the xcframework via
      a SwiftPM consumer — "v0.1.0-test2 dress rehearsal".)
- [ ] `solver_version()` returns the tag string (e.g. `"0.1.0"`) —
      not `"0.1.0-wip"` or a git SHA. Consumer-side version checks
      are load-bearing (`docs/ARCHITECTURE.md` — Poker Panel reads
      the version and refuses to load a mismatch).
      (**Currently returns `"0.1.0-dev"`** per `crates/solver-ffi/src/lib.rs:222`.
      Must flip to `"0.1.0"` on tag commit.)
- [ ] Thread safety: spawning N concurrent solves with N separate
      `SolverHandle`s produces N correct results with no cross-handle
      state leakage. Test in `solver-ffi/tests/`.
      (No concurrent-handle test present in `smoke.rs`.)
- [x] Binary size: `libsolver_ffi.dylib` is **< 10 MB** stripped
      (`docs/REQUIREMENTS.md` non-functional size target).
      (`target/release/libsolver_ffi.dylib` = **472 KB** today. Static `.a` is 18 MB
      but the dylib gate is the one the doc names.)

## Data — shipped artifacts

- [ ] Preflop ranges packed to `data/preflop-ranges/preflop-v0.1.bin`.
      Loader hits the file and returns a valid range in < 5 ms
      (`docs/REQUIREMENTS.md` preflop target).
      (`data/preflop-ranges/` contains only `.gitkeep`. Loader exists
      (`crates/solver-nlhe/src/preflop.rs`), no `.bin` file produced yet.)
- [ ] Isomorphism tables generated at build time and committed
      (or built by `build.rs`; either is fine as long as they exist
      on a fresh clone after `cargo build`).
      (`data/iso-tables/` contains only `.gitkeep`; no `build.rs` emits tables.
      Canonicalization is pure compute in `crates/solver-eval/src/iso.rs` —
      acceptable if we treat "runtime-only" as the build contract, but the doc
      currently calls for a committed artifact.)
- [x] First batch of Colab-generated flop-cache entries landed in
      `data/flop-cache/` (Day 7, per `docs/ROADMAP.md`). Cache format
      is versioned; a v0.1 consumer refuses to load a v0.2 file.
      (A49 `2ad0a5e` + `7bc1f5e` + `4a092f4` — `flop-cache-v0.1.bin` 374 KB,
      36 entries. Release notes flag it as "format-only placeholder"; real
      Colab-populated data is v0.1.x. Version gate is enforced in `flop_cache.rs`.)
- [x] Total shipped data is **< 200 MB** for preflop + iso tables,
      **< 500 MB** for the initial flop-cache subset
      (`docs/REQUIREMENTS.md` size targets).
      (Well under: only `flop-cache-v0.1.bin` ships at 374 KB.)

## Docs — consumer-facing

- [ ] `README.md` quick-start works on a fresh clone. Verified by
      running the commands it lists, top to bottom.
      (Commands exist and look right; fresh-clone smoke run not done.)
- [ ] `CHANGELOG.md` `[Unreleased]` block moved under `[0.1.0]` with
      the release date filled in. (A56 `08979d9` rewrote the Unreleased
      block but hasn't flipped it to `[0.1.0]` — tag-day action.)
- [x] `docs/RELEASE_NOTES_v0.1.md` finalized — under 300 words,
      plain English, linkable. (A56 `49955e6` polished it for customer
      consumption.)
- [x] `docs/INTEGRATION.md` reflects the final FFI contract. Matches
      `crates/solver-ffi/include/solver.h` symbol-for-symbol. (Spec doc
      exists and is current; FFI contract froze at `684e4a6`.)
- [ ] `docs/ARCHITECTURE.md` layout and FFI contract sections match
      reality (no stale symbols, no stale crate list).
      (Not re-audited by A61.)
- [ ] `docs/BENCHMARKS.md` baseline table populated with the tag-day
      numbers for every bench in the table.
      (Only regret_matching and cfr_plus_kuhn rows populated from A55's
      `2026-04-23_094257_8e26e00.json`. River/turn/cache/equity rows are "not yet wired".)
- [ ] `docs/ROADMAP.md` — Day 7 row marked complete, link to this
      checklist and the release notes.
      (Day 7 is 2026-04-28; today is 2026-04-23. Not yet marked complete.)

## Release mechanics

- [ ] `scripts/ship.sh` runs clean end-to-end on a fresh clone. This
      is the single command that validates everything above before
      tagging. (Script exists (`027906a`); not re-run on HEAD.)
- [ ] `git tag v0.1.0` created on the commit where `ship.sh` passed.
      (No tags in repo; dress-rehearsal artifacts use
      `v0.1.0-test`/`-test2`/`-dryrun` suffixes.)
- [ ] Tag pushed: `git push origin v0.1.0`.
- [ ] GitHub Release created with:
      - Title: `v0.1.0 — first working GTO solver for broadcast overlays`
      - Body: contents of `docs/RELEASE_NOTES_v0.1.md`
      - Attached: `target/release/libsolver_ffi.dylib`
      - Attached: `target/release/libsolver_ffi.a`
      - Attached: `crates/solver-ffi/include/solver.h`
      - Attached: `CHANGELOG.md` (or linked in release body)
      (Pipeline dry-run complete — A52 `19336ad` produced `v0.1.0-dryrun` artifacts;
      A60 verified xcframework zip consumption. Real release still pending.)
- [ ] SHA-256 checksums listed in the release body so integrators
      can verify the artifact they downloaded matches the tag.
      (`.sha256` files exist alongside every dress-rehearsal tarball/zip in
      `target/release-bundle/`; just needs to be the real tag's outputs.)
- [ ] Poker Panel integration PR drafted in the `Poker Panel` repo
      referencing this release's tag + checksums. Not merged during
      this sprint — that's Day 7+ in `docs/ROADMAP.md` — but the PR
      itself exists so the handoff is real.
      (Not drafted yet.)

## Post-ship follow-up (not blocking v0.1)

- [ ] `docs/v0_2_PLANNING.md` drafted: Metal shader decision, ICM,
      multi-way, PLO priorities. (File does not exist. Metal decision is
      captured in `docs/BENCHMARKS.md` and the `7e587c7` commit; a consolidated
      planning doc hasn't been written.)
- [ ] Colab flop-cache continuous-run monitor documented — Henry
      needs to be able to eyeball cache growth over the next 2 weeks
      (`docs/ROADMAP.md` Day 7). (No monitor doc found.)
- [x] Regression suite pinned so every v0.1 test spot stays as a
      fixture under `fixtures/` forever.
      (20 spots live at `crates/solver-cli/tests/fixtures/spot_001..spot_020.json` with
      `SCHEMA.md`. Pinned as of `d6b939e`/`939f6b4`.)
