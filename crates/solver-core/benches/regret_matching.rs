//! Microbench for the regret-matching inner loop.
//!
//! Isolates the 1326-wide regret-to-strategy conversion. This is the hot
//! path inside the river iteration.
//!
//! Target: < 1 µs per call (SIMD path).

use criterion::{criterion_group, criterion_main, Criterion};

fn bench_scalar(c: &mut Criterion) {
    c.bench_function("regret_matching_scalar_1326", |b| {
        b.iter(|| {
            // TODO (Day 1, agent A_main): once `regret_match` exists, bench it.
        });
    });
}

fn bench_simd(c: &mut Criterion) {
    c.bench_function("regret_matching_simd_1326", |b| {
        b.iter(|| {
            // TODO (Day 3, agent A1): once SIMD path exists, bench it.
        });
    });
}

criterion_group!(matching_benches, bench_scalar, bench_simd);
criterion_main!(matching_benches);
