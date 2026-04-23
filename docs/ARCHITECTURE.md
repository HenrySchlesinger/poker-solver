# Architecture

## Workspace layout

```
poker-solver/
├── Cargo.toml                  # workspace root
├── crates/
│   ├── solver-core/            # CFR+, MCCFR, regret matching, Vector CFR
│   ├── solver-nlhe/            # NLHE tree, bet-tree, ranges, action history
│   ├── solver-eval/            # hand evaluator, equity, isomorphism
│   ├── solver-ffi/             # C FFI surface (the contract with Poker Panel)
│   └── solver-cli/             # dev harness + Colab driver
├── benches/                    # cross-crate criterion benches
├── data/
│   ├── preflop-ranges/         # shipped with app
│   ├── iso-tables/             # generated at build
│   └── flop-cache/             # Colab output, subset shipped
├── colab/                      # Jupyter notebooks for precompute
└── docs/
```

## Crate responsibilities

### `solver-core`
Algorithm-only. Knows nothing about poker. Implements:
- `trait Game` — generic extensive-form game with imperfect information
- `CfrPlus` — vanilla CFR+ with regret matching
- `MCCfr` — external-sampling Monte Carlo CFR
- `VectorCfr` — SIMD-accelerated river solve
- Regret tables, strategy accumulators, convergence metrics

**No dependencies on `solver-nlhe` or `solver-eval`.** Generic over any
game that implements `trait Game`. Kuhn Poker is a test fixture in
`tests/kuhn.rs`.

### `solver-nlhe`
NLHE-specific primitives:
- `impl Game for NlheSubgame` — connects the algorithm to our poker game
- `BetTree` — discretized bet sizes per street
- `Range` — 1326-combo weight vector
- `Range::from_str("AA, KK, AKs, T9s+")` — range parser
- `ActionLog` — action history, used to reconstruct pot/stack state

Depends on `solver-core` (for `trait Game`) and `solver-eval` (for equity).

### `solver-eval`
Pure poker primitives, zero algorithm:
- `Card(u8)` — rank in high 4 bits, suit in low 2 bits. Values 0..52.
- `Hand([Card; 2])` — hole cards
- `Board([Card; 5], u8 len)`
- `HandRank` — 5-card evaluator output (straight flush, four of a kind, etc.)
- `eval_7(cards: [Card; 7]) -> HandRank` — the hot path
- `equity(hero: Hand, villain: Hand, board: Board) -> f32`
- `range_vs_range_equity(hero: &Range, villain: &Range, board: &Board) -> f32`
- `iso::canonical_board(b: &Board) -> Board` — suit-canonicalize for caching

Depends on nothing else in this workspace. Pure leaf crate.

### `solver-ffi`
The bridge to Swift. Consists of:
- `#[repr(C)]` structs matching the Swift side byte-for-byte
- `extern "C" fn` functions that accept/return those structs
- No Rust types cross the boundary — only pointers, ints, floats, and
  C-compatible structs
- Generates `solver.h` via `cbindgen` at build time

**This is the contract with Poker Panel.** Changes here are breaking
changes to the consumer.

### `solver-cli`
Dev-only tool. Subcommands:
- `solve` — solve a spot, output JSON
- `bench` — run benchmarks, output JSON
- `validate` — diff against TexasSolver on canonical spots
- `precompute` — batch-solve for Colab

Never shipped with Poker Panel. Strictly a dev harness.

## The FFI contract (`solver-ffi`)

Minimal surface. Public symbols:

```c
// Opaque handle to solver memory — one per concurrent caller.
typedef struct SolverHandle SolverHandle;

SolverHandle* solver_new(void);
void solver_free(SolverHandle* handle);

// Primary entry point.
int solver_solve(
    SolverHandle* handle,
    const HandState* input,
    SolveResult* output_buffer,
    uint32_t output_buffer_capacity
);

// Cache-lookup fast path for precomputed spots.
int solver_lookup_cached(
    const HandState* input,
    SolveResult* output_buffer,
    uint32_t output_buffer_capacity
);

// Version string for logging.
const char* solver_version(void);
```

Return codes: 0 = success, positive = cache miss / needs live solve,
negative = error. No exceptions. No Rust panics cross the boundary (we
`catch_unwind` at the FFI edge and return an error code).

### Why opaque handles, not free functions

Each concurrent call needs scratch memory (tens of MB for river, GB for
turn). Allocating per-call is prohibitive. `SolverHandle` owns the scratch
and can be reused across calls from the same thread.

Swift side:
```swift
let handle = solver_new()
defer { solver_free(handle) }
for hand in stream {
    var result = SolveResult()
    if solver_solve(handle, &hand, &result, MemoryLayout<SolveResult>.size) == 0 {
        overlay.render(result)
    }
}
```

## Data flow (live path)

```
Poker Panel (Swift)
    │
    │  CardEYE detects hole cards
    │  Event bus produces HandState
    │
    ▼
solver-ffi (C boundary)
    │
    ▼
solver-core::Solver::solve(hand_state)
    │
    │  1. Check flop-cache for precomputed subgame
    │     ↳ HIT: return cached strategy (< 10ms)
    │     ↳ MISS: fall through to live solve
    │  2. Build subgame tree (solver-nlhe)
    │  3. Run CFR+ / MCCFR / Vector CFR iterations
    │  4. Compute final strategy, EV, equity
    │
    ▼
SolveResult struct
    │
    ▼
solver-ffi (C boundary)
    │
    ▼
Poker Panel renders overlay
```

## Data flow (offline precompute)

```
Colab notebook
    │
    │  1. git clone poker-solver
    │  2. cargo build --release -p solver-cli
    │  3. Python script generates grid:
    │     [(board_texture, SPR, pot_type, bet_tree)] × N flops
    │  4. Parallel: `solver-cli precompute --flop <flop.json>`
    │  5. Outputs JSON / binary per flop to Google Drive
    │
    ▼
Local workstation
    │
    │  Downloads flop cache from Drive, dedupes,
    │  packs into binary format, commits to data/flop-cache/
    │
    ▼
Poker Panel release
    │
    │  Ships subset of cache in .app bundle
    │  Rest downloaded on first run from our CDN
```

## Concurrency model

- **Each `SolverHandle` is single-threaded.** One caller at a time.
- **Concurrent callers = multiple handles.** Poker Panel creates a pool.
- **Inside a solve**, we parallelize with `rayon` across info sets. That's
  intra-solve parallelism; it doesn't require multiple handles.
- **No shared mutable state.** Every `solve` call is independent.
- **No async.** CFR is CPU-bound, async adds overhead without benefit.

## Error handling

- `Result<T, SolverError>` inside Rust.
- `SolverError` → error code at the FFI boundary.
- Panics caught at FFI boundary, logged, returned as error code -1.
- Assertion failures in debug, non-panicking fallbacks in release where
  safe (e.g., cache miss returns "needs live solve" not a panic).

## Versioning

- `v0.X.Y` while pre-1.0.
- `v0.1.0` = first working ship (end of week 1).
- `solver_version()` returns the exact tag for logging.
- FFI struct layouts are versioned; Poker Panel reads the version string
  and refuses to load a mismatch.
