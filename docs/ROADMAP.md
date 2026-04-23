# 7-day sprint roadmap

**Sprint starts:** 2026-04-22
**v0.1 ships:** 2026-04-29

The plan is 10 parallel agents working during Henry's awake hours, plus
Colab jobs running overnight for precompute. The critical path is the
river inner loop; everything else is done in parallel.

## Day 1 — 2026-04-22 (Tuesday)

**Main critical path (Henry's direct attention):**
- Repo scaffold landed (this file, among others) ✅
- CFR+ implemented on Kuhn Poker (toy 3-card game) in `solver-core`
- Convergence test: Kuhn equilibrium matches published value within 1%

**Parallel agents:**

| Agent | Task | Crate |
|---|---|---|
| A1 | Card type, board type, deck utilities | `solver-eval` |
| A2 | Hand evaluator (wrap `rs-poker` or port 5-card lookup table) | `solver-eval` |
| A3 | Range parser ("AA, KK, AKs, T9s+") → 1326 weight vector | `solver-nlhe` |
| A4 | Board isomorphism tables (suit canonicalization) | `solver-eval` |
| A5 | Docs pass: fill gaps in `POKER.md` and `GLOSSARY.md` | `docs/` |
| A6 | Preflop range ingestion (import Monker/TexasSolver exports) | `solver-nlhe` |

**End of day:** CFR+ works on Kuhn. All supporting primitives compile and
have unit tests.

## Day 2 — 2026-04-23 (Wednesday)

**Main path:**
- NLHE river subgame data structures (`InfoSet`, `Strategy`, `Regrets`)
- Vanilla CFR+ runs on a real NLHE river spot (AhKh2s, two static ranges,
  one bet size, 100bb pot)
- Convergence test: output within 5% of TexasSolver on this spot

**Parallel agents:**

| Agent | Task | Crate |
|---|---|---|
| A1 | Bet-tree builder (discretize to 3 sizes: 33%, 66%, pot) | `solver-nlhe` |
| A2 | Action history tracker | `solver-nlhe` |
| A3 | Equity calculator (hand vs hand, hand vs range) | `solver-eval` |
| A4 | Validation harness: run TexasSolver + ours, diff outputs | `solver-cli` |
| A5 | `solver-cli solve` subcommand with JSON output | `solver-cli` |
| A6 | Start drafting C FFI header shape | `solver-ffi` |

**End of day:** River solves a canonical spot correctly. CLI usable.

## Day 3 — 2026-04-24 (Thursday)

**Main path:** THE LIMITING FACTOR DAY.
- **Vector CFR** on river. Inner loop becomes SIMD matrix ops.
- Target: < 1s river solve at 1000 iterations on M1 Pro
- Stretch: < 500ms
- Measured via criterion bench `crates/solver-core/benches/river.rs`

**Parallel agents:**

| Agent | Task | Crate |
|---|---|---|
| A1 | `std::simd` f32x8 regret update on river | `solver-core` |
| A2 | Cache-friendly layout: flatten regret tables to packed arrays | `solver-core` |
| A3 | Parallelize across info sets with `rayon` | `solver-core` |
| A4 | Criterion benchmark setup: `cargo bench` is the truth | `solver-core` |
| A5 | Profiling harness: `cargo flamegraph` reproducibility | `benches/` |
| A6 | Start Swift FFI: `cbindgen` config + generated header | `solver-ffi` |

**End of day:** River converges fast enough. We know the latency number and
it's below the 1s hard limit.

## Day 4 — 2026-04-25 (Friday)

**Main path:**
- Turn subgame: tree with nested river subgames
- **MCCFR (External Sampling)** for turn to keep memory bounded
- Target: 500 iters in < 30s on M1 Pro
- Swift FFI binding: `solve_hand_state` callable end-to-end

**Parallel agents:**

| Agent | Task | Crate |
|---|---|---|
| A1 | Turn betting tree construction | `solver-nlhe` |
| A2 | MCCFR sampler, deterministic PRNG | `solver-core` |
| A3 | Turn-specific criterion bench | `solver-core` |
| A4 | Swift example app consuming the FFI (test harness, not Poker Panel!) | `crates/solver-ffi/examples/` |
| A5 | Metal compute shader scoping (decision point: do we need it?) | `solver-core` |
| A6 | Colab notebook v1: build + run on remote Linux | `colab/` |

**End of day:** Turn solver works. FFI callable from Swift. Metal decision
made (build it, or skip until v0.2).

## Day 5 — 2026-04-26 (Saturday)

**Main path:**
- Flop subgame solver (not for live — for Colab precompute use)
- Preflop ranges ingested and shipped as a data file
- **Colab precompute kicks off overnight:** flop grid generation starts

**Parallel agents:**

| Agent | Task | Crate |
|---|---|---|
| A1 | Flop tree construction (nested turn subgames) | `solver-nlhe` |
| A2 | Flop-cache format: (board texture hash, SPR, pot type) → strategy | `solver-nlhe` |
| A3 | Cache lookup code in runtime path | `solver-core` |
| A4 | Preflop range lookup: static data loader | `solver-nlhe` |
| A5 | Colab notebook v2: parallel flop solving, output to Google Drive | `colab/` |
| A6 | Docs: `COLAB.md`, `BENCHMARKS.md` finalization | `docs/` |

**End of day:** Colab is solving flops overnight. Preflop works.

## Day 6 — 2026-04-27 (Sunday)

**Main path:**
- End-to-end integration test: mock Poker Panel consumer calls FFI,
  renders a fake overlay, validates output
- Convergence validation against TexasSolver on 20 canonical spots

**Parallel agents:**

| Agent | Task | Crate |
|---|---|---|
| A1 | 20-spot validation battery (diff vs TexasSolver) | `solver-cli` |
| A2 | Criterion benchmark CI: ensure no regressions | `benches/` |
| A3 | FFI safety audit: memory lifetime, thread safety | `solver-ffi` |
| A4 | Cleanup: `cargo clippy --all-targets -D warnings` passes | all crates |
| A5 | Edge cases: all-in spots, split pots, dead cards | `solver-nlhe` |
| A6 | Load first batch of Colab-generated flop cache | `data/` |

**End of day:** v0.1 is testable end-to-end. All 20 validation spots pass.

## Day 7 — 2026-04-28 (Monday)

**Main path:**
- **Ship v0.1.** Tag `v0.1.0`. Lock benchmarks. Document known gaps.
- First integration spike with Poker Panel (Henry tests consumption)
- Start of continuous flop precompute (Colab runs for the next 2 weeks
  populating the cache)

**Parallel agents:**

| Agent | Task |
|---|---|
| A1 | Release notes: what ships, what doesn't |
| A2 | Regression test suite: all test spots pinned |
| A3 | Poker Panel integration spec doc (how Swift consumes us) |
| A4 | Performance report: final numbers vs targets |
| A5 | v0.2 planning doc: Metal, ICM, multi-way, PLO |
| A6 | Flop cache population monitor |

**End of day:** v0.1 tagged. Poker Panel integration path clear. Colab
running in the background to grow the flop cache.

## What's NOT in the sprint

- Metal compute shaders (v0.2 if river is already fast enough in SIMD)
- ICM tournament math (v0.2)
- Multi-way (v0.3)
- PLO (v0.3+)
- GUI (never — that's Poker Panel's job)
- Per-hand exploit detection (post-v1.0)
- Web API (never — local-only is a hard requirement)

## Parallel-agent discipline

With 10 agents running at once:

1. **Claim a task** by adding your initial next to it in this file (or via
   git commit message). Don't double-claim.
2. **Commit to `main` small and often.** Target: commit every 30–60
   minutes. Merge conflicts with 10 agents are the #1 risk.
3. **Each task is scoped to one crate.** Cross-crate refactors block other
   agents — do them serialized, not parallel.
4. **If you're blocked, grab a docs task.** There's always doc work to do.
5. **Benchmark before + after** any performance claim. Paste numbers in
   the commit message.
