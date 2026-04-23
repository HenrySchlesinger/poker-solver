# Getting started (for a new agent)

You're joining a 7-day sprint to build a local NLHE GTO solver.
This is your onboarding doc. Read it once, then pick a task.

## The TL;DR

- We're building a **Rust workspace** that Swift consumes via FFI
- It lives at `~/Desktop/poker-solver/` (NOT `~/Desktop/Poker Panel/`)
- It computes GTO strategies for live poker broadcasts
- 7-day sprint, started 2026-04-22
- Runs local on Mac, uses Colab for offline precompute
- Owner: Henry, not a coder — explain things plainly in commits/PRs

## Reading order

Before writing any code:

1. [CLAUDE.md](../CLAUDE.md) — workflow rules (~5 min)
2. [WHY.md](WHY.md) — product context (~5 min)
3. [REQUIREMENTS.md](REQUIREMENTS.md) — performance and functional
   targets (~5 min)
4. [ROADMAP.md](ROADMAP.md) — day-by-day plan and your day's tasks
   (~10 min)
5. [ARCHITECTURE.md](ARCHITECTURE.md) — crate layout, FFI contract
   (~10 min)

If you don't play poker:
6. [POKER.md](POKER.md) — NLHE primer (~15 min)

If you're working on the algorithm:
7. [ALGORITHMS.md](ALGORITHMS.md) — CFR+/MCCFR/Vector CFR (~15 min)

Keep open in a tab:
- [GLOSSARY.md](GLOSSARY.md) — terms reference

Total onboarding: ~45 minutes, maybe 60 if you're new to poker.

## Pick a task

Look at today's row in [ROADMAP.md](ROADMAP.md). Pick an unclaimed task
(one without initials next to it). Claim it by editing the roadmap with
your agent name and pushing a commit:

```
-| A3 | Range parser ("AA, KK, AKs, T9s+") → 1326 weight vector | `solver-nlhe` |
+| A3 | Range parser ("AA, KK, AKs, T9s+") → 1326 weight vector | `solver-nlhe` | [claude-abc123] |
```

If you can't tell what's in flight, ask Henry.

## Validate your work

Three gates before committing:

### 1. It compiles and tests pass

```bash
cargo build --workspace
cargo test --workspace
```

### 2. Lints are clean

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```

### 3. If you claim a performance improvement, benchmark it

```bash
# Save baseline before your change:
cargo bench -p solver-core -- --save-baseline pre-change

# Make change, then:
cargo bench -p solver-core -- --baseline pre-change
```

Paste the before/after numbers in your commit message.

## Commit and push

Small commits on `main`. Don't branch. With 10 parallel agents, branches
cause merge pain.

```bash
git add <files>
git commit -m "solver-core: implement CFR+ regret matching

- Add RegretMatching trait in src/matching.rs
- Unit tests cover uniform fallback (all regrets zero)
- Benchmark: 612 ns / call on M1 Pro (baseline for Day 3 SIMD work)
"
git push  # safe; auto-commit hook doesn't conflict with real commits
```

## Common pitfalls

### "It works on my test but breaks convergence"
You probably broke regret accumulation. Check:
- Counterfactual reach probabilities multiplied correctly?
- Regrets clamped to ≥ 0 at end of each iter (CFR+ rule)?
- Strategy sum weighted by iteration `t` (linear averaging)?

Run the Kuhn Poker convergence test in `crates/solver-core/tests/`. If
that passes but NLHE fails, the bug is in `solver-nlhe`, not the core.

### "My NLHE solve disagrees with TexasSolver"
Common causes:
- Bet tree mismatch (different discretization). Check the tree JSON.
- Iteration count too low (TexasSolver runs 10k+ by default).
- Range representation differs (check you're using 1326 weights, not
  169 "hand types").
- Card isomorphism canonicalization bug.

Run `cargo run -p solver-cli -- validate` for structured diff output.

### "FFI crashes in Swift"
- Did you mark structs `#[repr(C)]`?
- Did you use `extern "C"` on the function?
- Did you wrap panics with `catch_unwind`?
- Did you update the cbindgen-generated header after changing layouts?

### "Compilation is slow"
The workspace uses `opt-level = 1` for dev builds. If something's still
slow, check `[profile.dev.package."*"]` — deps should be at opt-level 3.

## Don't do these

- Don't edit files in `~/Desktop/Poker Panel/`. That repo is shipping;
  leave it alone.
- Don't add new crates to the workspace without discussing with Henry
  first. The current 5-crate layout is intentional.
- Don't pull in new heavy dependencies (tokio, actix, etc.) — CFR is
  CPU-bound, we don't need them.
- Don't reimplement hand evaluators or range parsers from scratch if
  existing open-source code works.
- Don't ship without a test. If you're tempted to "it's trivial," write
  the one-line test anyway.

## Escalation

- **Task is ambiguous:** ask Henry. Don't guess on a core algorithm
  choice.
- **Blocked by someone else's WIP:** find another task from the roadmap.
  Always 10+ parallel workstreams available.
- **Found a bug in someone else's work:** fix it if small, file a clear
  commit message. If large, open a GitHub issue (TBD) or tell Henry.
- **Something feels wrong about the approach:** speak up. Henry isn't a
  coder but has strong product instincts. Don't silently implement
  something you think is wrong.
