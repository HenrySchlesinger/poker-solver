//! Full-solve benchmarks for CFR+ on Kuhn Poker.
//!
//! Kuhn is tiny (~12 info sets, 6 chance roots, ~5 terminal histories
//! per deal) so these numbers are not a direct proxy for NLHE. They're
//! here to:
//!
//! 1. Catch regressions in `CfrPlus::run_from` that would otherwise only
//!    surface once NLHE river benches exist.
//! 2. Exercise the end-to-end tree walk + regret update + strategy
//!    averaging for whatever changes Day-2/Day-3 agents make to
//!    `solver-core/src/cfr.rs`.
//! 3. Give a reproducible "iterations/sec" number that's meaningful even
//!    before SIMD / rayon / packed layouts land.
//!
//! Bench sizes: 10, 100, 1000 iterations. Each iteration is 2 tree
//! walks (Hero + Villain) × 6 chance deals, i.e. 12 full tree walks per
//! iteration.
//!
//! Run:
//! ```text
//! cargo bench -p solver-core --bench cfr_kuhn
//! ```
//!
//! Save a baseline before a refactor:
//! ```text
//! cargo bench -p solver-core --bench cfr_kuhn -- --save-baseline pre-change
//! ```
//!
//! Compare after a refactor:
//! ```text
//! cargo bench -p solver-core --bench cfr_kuhn -- --baseline pre-change
//! ```

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use solver_core::CfrPlus;

#[path = "common/kuhn.rs"]
mod kuhn;

use kuhn::KuhnPoker;

const ITERATION_COUNTS: &[u32] = &[10, 100, 1000];

/// Benchmark Kuhn CFR+ at each iteration count. Reports wall time and
/// iterations/sec via `Throughput::Elements`.
fn bench_cfr_plus_kuhn(c: &mut Criterion) {
    let roots = KuhnPoker::chance_roots();

    let mut group = c.benchmark_group("cfr_plus_kuhn");
    for &iters in ITERATION_COUNTS {
        group.throughput(Throughput::Elements(iters as u64));
        group.bench_with_input(BenchmarkId::from_parameter(iters), &iters, |b, &iters| {
            b.iter_with_setup(
                || CfrPlus::new(KuhnPoker),
                |mut solver| {
                    solver.run_from(black_box(&roots), black_box(iters));
                    // Force the average-strategy computation to run too —
                    // otherwise a clever compiler could elide the whole
                    // training pass as dead code.
                    let avg = solver.average_strategy();
                    black_box(avg.len());
                },
            );
        });
    }
    group.finish();
}

/// A simpler "how long does one `iterate_from` take" bench.
///
/// Uses `iter_batched` so each sampled iteration starts from a fresh
/// solver. The first call populates the info-set map (~12 entries for
/// Kuhn); subsequent calls inside the same measurement batch therefore
/// also measure allocation, which on a ZST game like Kuhn is
/// dominated by the HashMap churn. That's honest — the "per-iteration
/// cost" of CFR+ in the absence of warm-start logic includes that
/// allocation, and we don't expose a public `reset()` / `clone()` on
/// `CfrPlus` to cheat around it (A_main owns that API).
fn bench_single_iteration(c: &mut Criterion) {
    let roots = KuhnPoker::chance_roots();
    c.bench_function("cfr_plus_kuhn_single_iteration", |b| {
        b.iter_batched(
            || CfrPlus::new(KuhnPoker),
            |mut solver| {
                solver.iterate_from(black_box(&roots));
                black_box(solver.iterations());
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    cfr_kuhn_benches,
    bench_cfr_plus_kuhn,
    bench_single_iteration
);
criterion_main!(cfr_kuhn_benches);
