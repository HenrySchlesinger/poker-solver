# Hardware

## Target: Apple Silicon Mac

**Development machine:** Henry's M-series MacBook (exact model TBD, treat
as M1 Pro-class baseline).

**Production machines:** any Mac running Poker Panel. v0.1 supports macOS
13+ on M1, M2, M3, M4 (any variant: base, Pro, Max, Ultra). Intel Macs
work via Rosetta but unoptimized (no AVX2 SIMD path; pure scalar fallback).

## Why Apple Silicon is actually great for this

Three properties of M-series matter for CFR:

1. **Unified memory architecture.** CPU and GPU share the same RAM, no
   copy overhead. For Vector CFR, where we're moving matrices between
   CPU setup and GPU kernel launches, this saves orders of magnitude of
   PCIe latency compared to discrete-GPU x86 systems.
2. **Wide SIMD registers.** ARM NEON is 128-bit, but the M-series has
   multiple NEON execution units per core. With `std::simd::f32x8` we
   get 8-wide vectorization that maps cleanly to two NEON register ops
   per cycle. M3/M4 add SVE2 support in future compilers.
3. **High memory bandwidth.** M1 Pro: ~200 GB/s. M3 Max: ~400 GB/s. M4
   Max: ~540 GB/s. The regret-matching inner loop is memory-bandwidth
   bound at 1326×1326 matrix sizes; wide memory = fast CFR iterations.

## Performance tiers

Expected river-solve latency (1000 iterations, after Day 3 optimization):

| Machine | Cores | RAM | BW | Expected river | Notes |
|---|---|---|---|---|---|
| M1 | 8 | 8–16 GB | 68 GB/s | ~2 s | Base baseline |
| M1 Pro | 10 | 16–32 GB | 200 GB/s | ~500 ms | **Primary target** |
| M1 Max | 10 | 32–64 GB | 400 GB/s | ~300 ms | |
| M2 Pro | 10–12 | 16–32 GB | 200 GB/s | ~400 ms | |
| M3 Max | 14–16 | 36–128 GB | 300–400 GB/s | ~200 ms | |
| M4 Max | 14–16 | 36–128 GB | 540 GB/s | ~150 ms | Best |

**Hard requirement for v0.1:** < 1 s river on M1 Pro. If we miss this, we
pivot to precomputing river subgames too.

## Memory budget

Per solve, on-stack + heap:

| Scope | Regret tables | Strategy sum | Scratch | Total |
|---|---|---|---|---|
| River | ~10 MB | ~10 MB | ~5 MB | ~25 MB |
| Turn (MCCFR) | ~500 MB | ~500 MB | ~100 MB | ~1.1 GB |
| Flop (precompute only, Colab) | ~5 GB | ~5 GB | ~500 MB | ~11 GB |

A concurrent pool of 4 river solvers on an M1 Pro (16 GB machine) easily
fits. 8 concurrent = ~200 MB, still fine. Poker Panel should create a
`SolverHandle` pool sized to `availableProcessors() / 2` by default.

## Metal compute shaders — decision point

The v0.1 plan is **Rust SIMD first**. Metal is a Day 4 stretch goal, and
the decision to build it depends on how Day 3 lands:

- If Rust SIMD hits river < 500 ms: skip Metal, ship v0.1, revisit in v0.2
- If Rust SIMD hits 500 ms – 1 s: build Metal for river only
- If Rust SIMD > 1 s: Metal is mandatory for v0.1

Metal compute kernel sketch (if built):
- `f32` textures for hero strategy, villain strategy, payoff matrix
- One thread per hero combo (1326 threads total)
- Threadgroup size 32 or 64
- Dispatched from Swift or Objective-C wrapper in `solver-ffi`
- Expected speedup: 3–10× over Rust SIMD

Written in Metal Shading Language (.metal files), compiled at build time
into `solver.metallib`, loaded at runtime via Metal framework.

## Rosetta / Intel fallback

Intel Macs run the `x86_64` target via Rosetta or natively. Our CI builds
both slices and `lipo` them into a universal binary. On Intel, AVX2
intrinsics via `std::arch::x86_64` give us 8-wide f32. Performance is
~2× slower than M1 Pro for equivalent clock, mostly due to memory
bandwidth.

We don't optimize the Intel path. It works, it's slower, that's fine for
v0.1.

## Colab hardware (for offline precompute only)

**Free tier only.** Henry's rule: no paid services. Free tier gives us:
- 2-core Xeon CPU (usually ~2.2 GHz)
- ~13 GB RAM
- Optional T4 GPU (availability varies)
- 12-hour session max

**For our precompute workload,** free tier with CPU-only is fine for most
flop solves (each flop is ~2–10 min on a single Colab CPU core). Running
multiple free-tier flops in parallel across multiple browser sessions is
how we go fast — it's embarrassingly parallel, so horizontal scale beats
vertical scale on a paid tier.

If a free session expires mid-batch (12-hour cap), the resumable logic in
our Colab notebooks skips already-complete output files and continues
from where it left off. No paid upgrade needed.

**Fallback if free tier is insufficient:** run the same batch overnight
on Henry's Mac — the solver-cli binary is the same Rust code either way.

## Compiler toolchain

- **Rust:** stable (1.75+). `rust-toolchain.toml` pins the version.
- **Cargo:** workspace-aware. See `Cargo.toml` root for profiles.
- **cbindgen:** 0.26+, for C header generation in `solver-ffi`.
- **Xcode 15+** for Swift integration on Poker Panel side.
- **Metal shader compiler** (bundled with Xcode): only if we build the
  Metal path.

## CI

Not set up in Day 1. Deferred to post-v0.1. Initial testing is:
- Pre-commit: `cargo fmt --all && cargo clippy --all-targets -D warnings`
- Pre-merge: `cargo test --workspace`
- Pre-release: `cargo bench -p solver-core` benchmarks must match or beat
  last-release numbers
