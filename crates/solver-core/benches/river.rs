//! Criterion benchmark for river-subgame CFR+ performance.
//!
//! This is the primary KPI. Regressions here block merges.
//!
//! Targets (M1 Pro, 1000 iterations):
//! - `river_canonical_spot`: < 300 ms (hard limit 1 s)
//! - `river_degenerate_spot`: < 50 ms
//! - `river_wet_board`: < 500 ms

use criterion::{criterion_group, criterion_main, Criterion};

fn bench_canonical(c: &mut Criterion) {
    c.bench_function("river_canonical_spot", |b| {
        b.iter(|| {
            // TODO (Day 3, agent A_main): populate with a canonical river spot
            // - Board: AhKh2s-Qh-4d
            // - Hero range: AA, KK, AK (tight)
            // - Villain range: broadway
            // - Pot: 100, stack: 500
            // - Bet tree: {check, bet 33%, bet 66%}
            // - 1000 CFR+ iterations
        });
    });
}

fn bench_degenerate(c: &mut Criterion) {
    c.bench_function("river_degenerate_spot", |b| {
        b.iter(|| {
            // TODO: all-in-preflop river (showdown-only).
        });
    });
}

fn bench_wet_board(c: &mut Criterion) {
    c.bench_function("river_wet_board", |b| {
        b.iter(|| {
            // TODO: wet board with multiple draws (e.g., JhTh9c-8h-7c).
        });
    });
}

criterion_group!(river_benches, bench_canonical, bench_degenerate, bench_wet_board);
criterion_main!(river_benches);
