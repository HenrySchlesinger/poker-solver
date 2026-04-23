//! Microbench for the regret-matching inner loop.
//!
//! Isolates the N-wide regret-to-strategy conversion at the sizes that
//! actually show up in practice:
//!
//! | N    | Where it shows up                                   |
//! |------|------------------------------------------------------|
//! | 3    | tiny bet-tree spot (check/bet/raise)                 |
//! | 8    | a wider bet-sizing tree                              |
//! | 26   | up to 2*13 — Kuhn-ish action sets with history       |
//! | 169  | NLHE pre-flop hand grid                              |
//! | 1326 | all NLHE combos — the river hot path                 |
//!
//! Inputs are seeded random f32 regrets (mix of positive and negative)
//! generated from `rand_xoshiro::Xoshiro256PlusPlus` with the fixed seed
//! `[1; 32]`. Numbers are therefore reproducible across runs on the same
//! hardware.
//!
//! Target (release, M-series Mac):
//! - N=1326 scalar: serves as the Day-1 baseline. Day-3 SIMD work
//!   (agent A20) is expected to get this under 1 µs.
//!
//! Agent A20 owns a separate `_simd` bench file — DO NOT add SIMD
//! variants here. This file is the clean scalar baseline A20 compares
//! against.
//!
//! Run:
//! ```text
//! cargo bench -p solver-core --bench regret_matching
//! ```

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;

use solver_core::matching::regret_match;

/// Sizes covered. The last (1326) matches NLHE-combo scale.
const SIZES: &[usize] = &[3, 8, 26, 169, 1326];

/// Build a seeded-random regret vector of length `n`. Roughly half the
/// entries are negative so the regret-matching branch is exercised
/// realistically (pure-positive input skips the uniform-fallback path).
fn seeded_regrets(n: usize) -> Vec<f32> {
    let mut rng = Xoshiro256PlusPlus::from_seed([1; 32]);
    (0..n).map(|_| rng.gen_range(-1.0f32..1.0f32)).collect()
}

fn bench_scalar(c: &mut Criterion) {
    let mut group = c.benchmark_group("regret_matching_scalar");
    for &n in SIZES {
        let regrets = seeded_regrets(n);
        let mut out = vec![0.0f32; n];
        // Throughput in elements/sec so criterion reports per-element cost
        // alongside per-iteration ns.
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

criterion_group!(matching_benches, bench_scalar);
criterion_main!(matching_benches);
