# poker-solver — workflow rules for Claude sessions

This is a **brand new repo, started 2026-04-22**, targeting a 7-day sprint to
ship a local GTO solver that gets consumed by the sibling app
`~/Desktop/Poker Panel/` via FFI. The owner (Henry) is not a coder — follow
these rules so work stays coherent across the ~10 parallel agents running at
once.

## Read these first (in order)

Before doing anything substantive in this repo, read:

1. [docs/WHY.md](docs/WHY.md) — why we're building this at all
2. [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md) — what "done" means
3. [docs/ROADMAP.md](docs/ROADMAP.md) — the 7-day plan and where your task
   lives in it
4. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — crate layout + the FFI contract
5. [docs/POKER.md](docs/POKER.md) — if you're a Rust engineer who doesn't
   play poker, read this before touching NLHE code
6. [docs/ALGORITHMS.md](docs/ALGORITHMS.md) — CFR+/MCCFR/Vector CFR reference
7. [docs/LIMITING_FACTOR.md](docs/LIMITING_FACTOR.md) — the critical path
8. [docs/GLOSSARY.md](docs/GLOSSARY.md) — terms, keep this open in a tab

## Single source of truth

- **Main checkout: `~/Desktop/poker-solver/`**, always on `main`.
- **Do NOT touch `~/Desktop/Poker Panel/`** during this sprint. Poker Panel
  is shipping. The integration work (consuming our FFI) happens *after* v0.1
  of this repo is stable. Leave the app alone.
- All work lands on `main`. No long-lived feature branches. Small PRs, fast
  merges. With 10 agents running you cannot afford divergent branches.

## Worktrees: opt-in, not default

- Do NOT `EnterWorktree` by default. Parallel agents run in the same tree —
  Rust's cargo workspace handles concurrent compilation fine, and agents
  working on different crates don't collide.
- Only use a worktree when an agent is doing something destructive (rewriting
  the FFI surface, rebuilding the tree builder) and needs isolation.
- If you start a worktree, merge it before ending your session.

## Cargo workspace discipline

- The workspace lives at `Cargo.toml` in the repo root. All crates are
  members. Add new dependencies to `[workspace.dependencies]` once, then
  reference as `workspace = true` in each crate.
- Run `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings`
  before committing. CI will reject dirty formatting.
- `cargo test --workspace` must pass. Convergence tests in
  `crates/solver-core/tests/` are load-bearing — they catch CFR regressions
  that benchmarks won't.
- `cargo bench` via criterion is the truth for performance claims. See
  [docs/BENCHMARKS.md](docs/BENCHMARKS.md). Don't regress the river inner
  loop without a really good reason.

## Pre-commit hook

- Install once per clone: `./.githooks/install.sh`. This symlinks
  `.git/hooks/pre-commit` to `.githooks/pre-commit`, so future edits to
  the hook take effect immediately without reinstalling.
- The hook runs (in order) `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace --lib --no-run`. It does NOT run the tests
  themselves — that's CI's job. Warm-cache target: ~1s, well under 30s.
- If a commit is rejected, the hook prints a clear fix hint (e.g. `Fix
  with: cargo fmt --all`). Bypass with `git commit --no-verify` only for
  snapshotting known-broken WIP; CI will still reject on push.

## Rust wherever possible

- **This is a Rust-first project.** Prefer Rust binaries over shell scripts,
  Rust test harnesses over Python harnesses, Rust CLI tools over Python
  CLI tools. `solver-cli` is the home for dev tools that would be Python
  elsewhere.
- **Exception: Colab notebooks.** Colab runs Jupyter — Python is unavoidable
  there. Keep notebook Python minimal and offload real work to invocations
  of our compiled Rust binaries (`./target/release/solver-cli ...`).
- **Shell scripts are OK only for** short glue that invokes external tools
  (git clone, cmake) where Rust would add zero value.
- If you find yourself writing more than ~30 lines of bash or ~50 lines of
  Python, it should probably be a Rust binary under `solver-cli`.

## No paid GTO / solver services (Henry's rule)

- **We are building our own solver.** Do NOT add paid GTO / solver
  dependencies: no Deepsolver API, no GTO Wizard API, no PioSolver
  licenses, no third-party commercial solvers. Those are the services
  our product is designed to REPLACE, so depending on them defeats the
  whole point.
- **Free FLOSS is fine.** `rs_poker`, `zstd`, `rand`, `wide`, `criterion`,
  `proptest`, `cbindgen`, TexasSolver (used only as a locally-built test
  oracle — not redistributed) all remain in scope.
- **Compute is pragmatic.** Colab free tier is the default for precompute;
  Colab Pro ($10/mo) is OK if Henry genuinely needs the throughput for
  overnight jobs. The rule is "don't pay for stuff we can build or do
  ourselves," not "never spend a dollar on compute." Default to free, and
  if free is slow, ask before paying.

## Be kind to Henry's machine

- Henry's Mac has been memory-pressured by many concurrent Claude agents.
  Prefer `cargo check -p <crate>` over `cargo build --workspace` when
  you only need to verify a single crate compiles. Don't run more than
  one heavy bench or release build at a time.
- If you're dispatched alongside other agents (common in this repo's
  speedrun mode), assume they may also be running cargo concurrently.
  Your build doesn't need to be the first one to finish.

## Don't over-engineer

- We have 7 days. Ship the working thing, not the elegant thing.
- Prefer `u8` and fixed arrays to smart types. Cache-friendly beats generic.
- No async unless it's genuinely needed (it isn't — CFR is CPU-bound).
- No serde on hot paths. Parse once, work on packed structs.
- TODO comments for optimizations we'll do after week 1 are fine. Leave them.

## Don't do

- Don't depend on cloud services at runtime. The solver runs **local, on the
  user's Mac**. Colab is for offline precompute only.
- Don't reach for Metal compute shaders on Day 1. SIMD in Rust first; Metal
  is the Day 4+ optimization if pure Rust can't hit the latency target.
- Don't add web frameworks, HTTP servers, or databases. The FFI surface is
  a function call, period.
- Don't reimplement open-source primitives badly. Hand evaluators are a
  solved problem — wrap an existing one (`rs-poker`, or port the bit tricks
  from `poker-eval`) rather than rolling your own poorly.
- Don't publish to crates.io. Private build artifacts only for now.

## Definition of done for week 1

See [docs/ROADMAP.md](docs/ROADMAP.md) for the day-by-day. The v0.1 ship
criteria are:

1. River solver: full solve converges in <1s on M1 Pro at 1000 iterations
2. Turn solver: converges in <30s at 500 iterations
3. Preflop: static range lookup from shipped data
4. Flop: lookup from precomputed cache (populated via Colab)
5. FFI: Swift can call `solve_hand_state(HandState) -> SolveResult` and get
   valid output
6. Convergence validated against TexasSolver on 20 canonical spots within
   5% strategy delta

If you're doing something not on that list, stop and check with Henry.
