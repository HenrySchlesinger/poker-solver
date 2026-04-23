//! Head-to-head bench: `CfrPlus` (HashMap) vs `CfrPlusFlat` (flat tables).
//!
//! Per `docs/LIMITING_FACTOR.md`, step #1 on the river-inner-loop
//! optimization ladder is replacing the `HashMap<InfoSetId, Vec<f32>>`
//! bookkeeping with cache-friendly flat arrays. Expected speedup at
//! inner-loop scale is 3–5×. This bench makes that number measurable on
//! Kuhn, the smallest fixture where the claim applies at all.
//!
//! Kuhn is small (~12 info sets, 5 terminal histories per deal) — far
//! smaller than an NLHE river subgame — but the relative cost shape is
//! what matters. If flat is faster on Kuhn, it'll be faster on NLHE too;
//! a layout that loses on Kuhn can't win on NLHE.
//!
//! Run:
//! ```text
//! cargo bench -p solver-core --bench flat_vs_hashmap
//! ```
//!
//! The bench reports three groups:
//! - `hashmap`: `CfrPlus::run_from` at 1000 iters (the baseline).
//! - `flat`:    `CfrPlusFlat::run_from` at 1000 iters (the contender).
//! - `flat_with_enumeration`: construction + 1000 iters. Pays the
//!   one-time info-set enumeration cost so the speedup claim can't be
//!   gamed by amortizing it out.
//!
//! Criterion's summary numbers contain the mean times. To read off the
//! speedup factor, divide `hashmap` / `flat`.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use solver_core::{CfrPlus, CfrPlusFlat};

#[path = "common/kuhn.rs"]
mod kuhn;

use kuhn::KuhnPoker;

const ITERATIONS: u32 = 1000;

fn bench_hashmap(c: &mut Criterion) {
    let roots = KuhnPoker::chance_roots();
    let mut group = c.benchmark_group("hashmap");
    group.throughput(Throughput::Elements(ITERATIONS as u64));
    group.bench_with_input(
        BenchmarkId::from_parameter(ITERATIONS),
        &ITERATIONS,
        |b, &iters| {
            b.iter_with_setup(
                || CfrPlus::new(KuhnPoker),
                |mut solver| {
                    solver.run_from(black_box(&roots), black_box(iters));
                    // Force the average-strategy computation so a clever
                    // optimizer can't elide the whole training pass.
                    let avg = solver.average_strategy();
                    black_box(avg.len());
                },
            );
        },
    );
    group.finish();
}

fn bench_flat(c: &mut Criterion) {
    let roots = KuhnPoker::chance_roots();
    // Pre-enumerate once so the bench isolates run_from cost (the apples-
    // to-apples comparison against `bench_hashmap`, which also amortizes
    // its first-visit allocations over warm-up). The
    // `flat_with_enumeration` group below measures the full cold path.
    let descriptors = solver_core::enumerate_info_sets_from_roots(&KuhnPoker, &roots);

    let mut group = c.benchmark_group("flat");
    group.throughput(Throughput::Elements(ITERATIONS as u64));
    group.bench_with_input(
        BenchmarkId::from_parameter(ITERATIONS),
        &ITERATIONS,
        |b, &iters| {
            b.iter_with_setup(
                || CfrPlusFlat::new(KuhnPoker, &descriptors),
                |mut solver| {
                    solver.run_from(black_box(&roots), black_box(iters));
                    let avg = solver.average_strategy();
                    black_box(avg.len());
                },
            );
        },
    );
    group.finish();
}

/// The "honest" bench: construction + enumeration + solve in one timing.
/// This is what a first-time caller pays; it's what the speedup claim
/// must survive when the caller can't amortize the table allocation.
fn bench_flat_with_enumeration(c: &mut Criterion) {
    let roots = KuhnPoker::chance_roots();
    let mut group = c.benchmark_group("flat_with_enumeration");
    group.throughput(Throughput::Elements(ITERATIONS as u64));
    group.bench_with_input(
        BenchmarkId::from_parameter(ITERATIONS),
        &ITERATIONS,
        |b, &iters| {
            b.iter(|| {
                let mut solver = CfrPlusFlat::from_roots(KuhnPoker, black_box(&roots));
                solver.run_from(black_box(&roots), black_box(iters));
                let avg = solver.average_strategy();
                black_box(avg.len());
            });
        },
    );
    group.finish();
}

criterion_group!(
    flat_vs_hashmap_benches,
    bench_hashmap,
    bench_flat,
    bench_flat_with_enumeration
);
criterion_main!(flat_vs_hashmap_benches);
