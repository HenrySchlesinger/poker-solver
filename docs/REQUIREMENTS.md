# Requirements

## Functional

Given a `HandState`:

```
struct HandState {
    board: [Card; 5]          // with valid_len 0..=5
    hero_range: Range          // 1326 weighted combos
    villain_range: Range       // 1326 weighted combos
    pot: u32                   // in chips
    effective_stack: u32       // in chips
    to_act: Player             // Hero | Villain
    action_history: ActionLog  // who did what on which street
    bet_tree: BetTree          // discretized allowed bet sizes
}
```

Return a `SolveResult`:

```
struct SolveResult {
    action_frequencies: {Action: f32}  // sums to 1.0
    ev_per_action: {Action: f32}       // in big blinds
    hero_equity: f32                   // vs villain's range
    hero_range_strategy: [[f32; N]; 1326]  // per-combo frequencies
    convergence_delta: f32             // exploitability, lower = better
    iterations_used: u32
    compute_ms: u32
}
```

The solver must produce this for any legal NLHE spot, any board, any
action history, any stack depth 10–500bb.

## Performance targets

Measured on M1 Pro, 10-core, 16GB RAM. Scale up for M3 Max etc.

| Scope | Iterations | Target latency | Rationale |
|---|---|---|---|
| **River subgame** | 1000 | **< 300 ms** | Fire on villain bet, overlay visible before next action. This is THE target. |
| **Turn subgame** | 500 | < 30 s | Fire on villain turn bet, ready by showdown. Hands average 60–90s. |
| **Flop subgame** | — | Cache lookup < 10 ms | Precomputed offline; live = O(hashmap lookup). |
| **Preflop** | — | Lookup < 5 ms | Static ranges from shipped data file. |

**Stretch** (Metal compute shader path):
- River: < 100 ms at 1000 iterations
- Turn: < 10 s at 500 iterations

**Hard limits** (we don't ship below these):
- River must be < 1 s at 1000 iterations, or we precompute river too
- FFI call overhead < 10 µs per call (measure this separately)
- Memory per solve: < 500 MB for river, < 2 GB for turn

## Quality targets

- **Convergence:** exploitability < 1% of pot on river at 1000 iters
- **Strategy accuracy vs TexasSolver:** per-action frequency within 5%
  (absolute) on 20 canonical test spots
- **EV accuracy:** within 0.1 bb of TexasSolver on the same test spots
- **No crashes, no panics, no undefined behavior.** Run
  `cargo test --workspace` with address sanitizer before releasing.

## Non-functional

### Runs local
- No network calls at runtime. Ever. Zero cloud dependency.
- All data files ship with the app or are generated on first run.
- Colab is **offline precompute only** — runs on Henry's time, outputs
  static files that ship with Poker Panel.

### Platform
- Primary: macOS 13+ on Apple Silicon (M1, M2, M3, M4)
- Secondary: macOS 13+ on Intel (Rosetta-compatible; unoptimized path)
- **Not supported:** Windows, Linux, iOS. Server-side compilation is fine
  (Colab runs Linux x86_64) but the runtime is Mac-only.

### FFI surface
- The public API is a handful of `extern "C"` functions. See
  `crates/solver-ffi/src/lib.rs`.
- Callable from Swift via `cbindgen`-generated header + Swift Package
  Manager binary target.
- Thread-safe: the solver must tolerate being called from any thread and
  being called concurrently from multiple threads (each call uses its own
  scratch memory).

### Determinism
- Given the same `HandState` and the same PRNG seed, we must produce
  bit-identical output. This is load-bearing for validation tests.
- MCCFR uses explicit seeds (not time-based). Default seed = 0.

### Size
- Binary `.dylib` size: < 10 MB
- Shipped data files (preflop ranges + iso tables): < 200 MB
- Flop cache: grows over time, CDN-downloadable on first run; initial
  ship < 500 MB subset.

## What's explicitly OUT of scope for v0.1

- **PLO (Pot-Limit Omaha).** NLHE only for v0.1. Omaha bolts on later;
  the CFR core is game-agnostic but the evaluator and range representation
  need work.
- **Multi-way (3+ player) solves.** Heads-up only for v0.1. Tournament
  broadcasts are 80% heads-up pots anyway.
- **Tournament ICM math.** Cash-game-style EV only for v0.1. ICM is a
  v0.2+ feature.
- **Node locking** (forcing a specific strategy at a node). Solver-coaching
  feature, irrelevant for broadcast.
- **Rake modeling.** Assume zero rake. Add in v0.2 if broadcast partners
  ask.
- **Exploitative deviations.** GTO only. No opponent-modeling logic.
- **Live real-time learning.** Each solve is independent. No online updates.
