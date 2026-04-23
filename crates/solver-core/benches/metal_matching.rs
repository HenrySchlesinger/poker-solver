//! Metal vs SIMD vs scalar microbench for regret-matching at the river
//! scale.
//!
//! This is the measured-speedup companion to the correctness gate in
//! `tests/metal_equivalence.rs`. Per `docs/LIMITING_FACTOR.md`, the
//! headline number is the **speedup at N=1326** (the NLHE river
//! combo count). Task brief expects >= 3× over Rust SIMD.
//!
//! We also bench the smaller sizes that show up in non-river parts of
//! the solver — not because Metal will win there (it won't — GPU
//! dispatch overhead dominates at N<200), but so the speed-crossover
//! point is documented in the bench output for future tuning.
//!
//! | N    | Where it shows up                                   |
//! |------|------------------------------------------------------|
//! | 169  | NLHE pre-flop hand grid                              |
//! | 1326 | all NLHE combos — the river hot path                 |
//! | 4096 | oversized diagnostic                                 |
//!
//! # Reading the output
//!
//! Criterion prints three groups — `regret_matching_scalar`,
//! `regret_matching_metal`, and (when the `metal` feature is on)
//! `regret_matching_simd` if the sibling benches are also run. Compare
//! them at the same N to read the speedup. The commit message that
//! introduces this file should paste the numbers.
//!
//! # Context reuse
//!
//! The `MetalContext` is created *once* per bench group. Per-iteration
//! context creation would dominate the measurement — compiling the
//! shader library and creating the pipeline state is ~10-30ms on
//! first call. In production the context is created once per
//! `SolverHandle` and reused across all regret_match calls.
//!
//! Run:
//! ```text
//! cargo bench -p solver-core --features metal --bench metal_matching
//! ```

#![cfg(feature = "metal")]

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;

use solver_core::matching::regret_match;
use solver_core::metal::{regret_match_metal, MetalContext};

/// Sizes covered. 1326 is the river scale — that's the number that
/// matters for the v0.1 latency target.
const SIZES: &[usize] = &[169, 1326, 4096];

/// Build a seeded-random regret vector. Same seed as the other bench
/// files so numbers are directly comparable.
fn seeded_regrets(n: usize) -> Vec<f32> {
    let mut rng = Xoshiro256PlusPlus::from_seed([1; 32]);
    (0..n).map(|_| rng.gen_range(-1.0f32..1.0f32)).collect()
}

fn bench_scalar(c: &mut Criterion) {
    let mut group = c.benchmark_group("regret_matching_scalar");
    for &n in SIZES {
        let regrets = seeded_regrets(n);
        let mut out = vec![0.0f32; n];
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &regrets, |b, regrets| {
            b.iter(|| {
                regret_match(black_box(regrets), black_box(&mut out));
                black_box(&out);
            });
        });
    }
    group.finish();
}

fn bench_metal(c: &mut Criterion) {
    // If the Metal device can't be initialized (e.g. we're on a
    // CI container without Metal), skip silently. Criterion will just
    // produce a smaller report.
    let ctx = match MetalContext::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("metal bench: skipping — MetalContext::new() failed: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("regret_matching_metal");
    for &n in SIZES {
        let regrets = seeded_regrets(n);
        let mut out = vec![0.0f32; n];
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &regrets, |b, regrets| {
            b.iter(|| {
                regret_match_metal(&ctx, black_box(regrets), black_box(&mut out))
                    .expect("metal dispatch failed");
                black_box(&out);
            });
        });
    }
    group.finish();
}

criterion_group!(metal_benches, bench_scalar, bench_metal);
criterion_main!(metal_benches);
