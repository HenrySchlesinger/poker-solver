//! Scalar-vs-SIMD microbench for the regret-matching inner loop.
//!
//! This is the measured-speedup counterpart to the correctness gate in
//! `tests/simd_equivalence.rs`. It runs both implementations at the four
//! sizes that actually show up in practice:
//!
//! | N    | Where it shows up                                      |
//! |------|--------------------------------------------------------|
//! | 8    | minimum SIMD-path length (smallest vectorized input)   |
//! | 26   | mid-size action set with history                       |
//! | 169  | NLHE pre-flop hand grid                                |
//! | 1326 | all NLHE combos — the river hot path                   |
//!
//! The 1326 number is the one that matters. That's the river inner
//! loop, which the SIMD optimization exists to speed up. Everything
//! else is diagnostic.
//!
//! # Reading the output
//!
//! Criterion prints one group per function, one sub-bench per size.
//! Compare "regret_matching_scalar/N" to "regret_matching_simd/N" at
//! the same N to read off the speedup. Criterion will also do the
//! division for you when it detects you're measuring related functions.
//!
//! # Measured speedups (M-series Mac, release build, 2026-04-22)
//!
//! Criterion medians, `wide = "0.7"` with AArch64 NEON backend on Apple
//! Silicon. Re-run `cargo bench -p solver-core --bench simd_matching`
//! to refresh these — expectations per docs/LIMITING_FACTOR.md were
//! 6–8× at N=1326; we comfortably beat that. Anything below 2× on a
//! future bench run means the build lost vectorization (check that
//! `target-cpu=native` isn't wiped out, and that `wide` picked up NEON
//! rather than the scalar fallback).
//!
//! - N=8     scalar 5.32 ns    simd 1.89 ns    2.81x
//! - N=26    scalar 16.28 ns   simd 4.22 ns    3.86x
//! - N=169   scalar 152.89 ns  simd 18.10 ns   8.45x
//! - N=1326  scalar 1668 ns    simd 192 ns     8.68x  (the river hot path)
//!
//! 8.68× on the river inner loop means the 1000-iter river solve's
//! matching step goes from ~2.2 ms to ~0.25 ms. That's ~2 ms of the
//! 300 ms budget back.
//!
//! Run:
//! ```text
//! cargo bench -p solver-core --bench simd_matching
//! ```

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;

use solver_core::matching::regret_match;
use solver_core::{regret_match_simd, regret_match_simd_vector};

/// Sizes covered. N=1326 is the NLHE-combo scale; everything else is
/// diagnostic.
const SIZES: &[usize] = &[8, 26, 169, 1326];

/// Build a seeded-random regret vector of length `n`. Matches the seed
/// used by `benches/regret_matching.rs` so the two bench suites are
/// measuring identical inputs and can be directly compared.
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

fn bench_simd(c: &mut Criterion) {
    let mut group = c.benchmark_group("regret_matching_simd");
    for &n in SIZES {
        let regrets = seeded_regrets(n);
        let mut out = vec![0.0f32; n];
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &regrets, |b, regrets| {
            b.iter(|| {
                regret_match_simd(black_box(regrets), black_box(&mut out));
                black_box(&out);
            });
        });
    }
    group.finish();
}

/// Vector-CFR primitive at N=1326 with A=5 (the NLHE river shape) and
/// A=2 (Kuhn shape). Measures how fast we can regret-match 8192 f32s
/// (1326 × up-to-6 actions) as a single call.
fn bench_simd_vector(c: &mut Criterion) {
    // NLHE river shape: A=5, N=1326.
    let mut group = c.benchmark_group("regret_matching_simd_vector");
    for &(a, n) in &[(5usize, 1326usize), (3, 1326), (2, 1326), (5, 169)] {
        let mut rng = Xoshiro256PlusPlus::from_seed([1; 32]);
        let regrets: Vec<Vec<f32>> = (0..a)
            .map(|_| (0..n).map(|_| rng.gen_range(-1.0f32..1.0)).collect())
            .collect();
        let mut out_buf: Vec<Vec<f32>> = (0..a).map(|_| vec![0.0f32; n]).collect();

        group.throughput(Throughput::Elements((a * n) as u64));
        let id = BenchmarkId::from_parameter(format!("a{}_n{}", a, n));
        group.bench_with_input(id, &regrets, |b, regrets| {
            let refs: Vec<&[f32]> = regrets.iter().map(|v| v.as_slice()).collect();
            b.iter(|| {
                let mut out_refs: Vec<&mut [f32]> =
                    out_buf.iter_mut().map(|v| v.as_mut_slice()).collect();
                regret_match_simd_vector(black_box(&refs), black_box(&mut out_refs));
            });
        });
    }
    group.finish();
}

criterion_group!(simd_benches, bench_scalar, bench_simd, bench_simd_vector);
criterion_main!(simd_benches);
