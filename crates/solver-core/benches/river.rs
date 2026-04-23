//! Criterion benchmark for river-subgame CFR+ performance.
//!
//! This file hosts the primary KPI: `river_canonical_spot`. Regressions
//! here block merges.
//!
//! ## Status (Day 1 / Day 2)
//!
//! NLHE river subgames don't exist yet — `solver-nlhe::NlheSubgame` is
//! the Day 3 deliverable. Until that lands, `river_canonical_spot` is
//! marked `#[ignore]` (by putting it behind an env-gated code path) and
//! the "river" bench group runs **Kuhn Poker CFR+** as a placeholder so
//! the harness itself is exercised, wired up, and measured.
//!
//! Why Kuhn as a placeholder: Kuhn is what the Day-1 CFR+ implementation
//! is already correct on (see `tests/kuhn.rs`). It's a honest, tiny,
//! reproducible number. Once the NLHE river subgame lands, the
//! `river_canonical_spot` bench gets swapped in and this placeholder
//! gets removed (or relegated to `cfr_kuhn.rs`).
//!
//! ## Day-3 agents: what needs to change here
//!
//! When `solver-nlhe::NlheSubgame` exists:
//! 1. Add `solver-nlhe` to `[dev-dependencies]` in
//!    `crates/solver-core/Cargo.toml`.
//! 2. Replace the body of `bench_river_canonical_spot` below with a
//!    real construction of the canonical spot:
//!    - Board: AhKh2s-Qh-4d
//!    - Hero range: AA, KK, AK
//!    - Villain range: broadway
//!    - Pot: 100, stacks: 500 behind
//!    - Bet tree: {check, bet 33%, bet 66%}
//!    - 1000 CFR+ iterations
//! 3. Remove the `skip_river_bench()` gate.
//! 4. Add `bench_river_degenerate_spot` (all-in-pre runout) and
//!    `bench_river_wet_board` (JhTh9c-8h-7c) in the same style.
//! 5. Target M1 Pro: < 300 ms for canonical, < 50 ms for degenerate,
//!    < 500 ms for wet board. Hard limit 1 s on canonical.
//!
//! Run:
//! ```text
//! cargo bench -p solver-core --bench river
//! ```
//!
//! Force the ignored stub to still execute (useful once NLHE lands but
//! before we flip the gate):
//! ```text
//! SOLVER_RUN_RIVER_BENCH=1 cargo bench -p solver-core --bench river
//! ```

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};

use solver_core::CfrPlus;

#[path = "common/kuhn.rs"]
mod kuhn;

use kuhn::KuhnPoker;

/// Returns true when the river-canonical bench should stay a no-op.
/// Flipped off by env var once the NLHE river subgame is ready.
fn skip_river_bench() -> bool {
    std::env::var_os("SOLVER_RUN_RIVER_BENCH").is_none()
}

/// Placeholder bench: Kuhn Poker CFR+, 1000 iterations. This is the
/// wiring for `bench_river_canonical_spot` — same budget, real game,
/// just a tiny one.
fn bench_river_placeholder_kuhn(c: &mut Criterion) {
    let roots = KuhnPoker::chance_roots();
    c.bench_function("river_placeholder_kuhn_1000_iters", |b| {
        b.iter_with_setup(
            || CfrPlus::new(KuhnPoker),
            |mut solver| {
                solver.run_from(black_box(&roots), black_box(1000));
                // Prevent solver from being optimized away.
                black_box(solver.iterations());
            },
        );
    });
}

/// Stub for the real river spot. See module docs for what Day-3 agents
/// need to change. Until NLHE river subgame lands, this bench is an
/// `#[ignore]`-equivalent no-op (criterion doesn't support `#[ignore]`
/// directly on bench functions, so we gate on an env var).
fn bench_river_canonical_spot(c: &mut Criterion) {
    if skip_river_bench() {
        // Intentionally skip — NLHE river subgame doesn't exist yet.
        // Register the bench function name so `target/criterion` listings
        // surface the stub, but return immediately.
        eprintln!(
            "river_canonical_spot: SKIPPED (set SOLVER_RUN_RIVER_BENCH=1 \
             once solver-nlhe::NlheSubgame is implemented)"
        );
        return;
    }

    // TODO (Day 3, A_main or whoever owns solver-nlhe): swap in real
    // river subgame. See module-level docs for spec.
    c.bench_function("river_canonical_spot", |b| {
        b.iter(|| {
            // Placeholder path exercised when the env var is set — still
            // Kuhn for now so nothing panics.
            let mut solver = CfrPlus::new(KuhnPoker);
            solver.run_from(&KuhnPoker::chance_roots(), 1000);
            black_box(solver.iterations());
        });
    });
}

/// Stub for the degenerate all-in-pre river spot (showdown-only).
/// Left as a no-op until `solver-nlhe::NlheSubgame` is implemented.
fn bench_river_degenerate_spot(c: &mut Criterion) {
    if skip_river_bench() {
        eprintln!("river_degenerate_spot: SKIPPED (NLHE river subgame not ready)");
        return;
    }
    // TODO: fill in once NLHE lands.
    let _ = c; // keep the signature criterion-compatible
}

/// Stub for the wet-board river spot (JhTh9c-8h-7c style).
/// Left as a no-op until `solver-nlhe::NlheSubgame` is implemented.
fn bench_river_wet_board(c: &mut Criterion) {
    if skip_river_bench() {
        eprintln!("river_wet_board: SKIPPED (NLHE river subgame not ready)");
        return;
    }
    // TODO: fill in once NLHE lands.
    let _ = c;
}

criterion_group!(
    river_benches,
    bench_river_placeholder_kuhn,
    bench_river_canonical_spot,
    bench_river_degenerate_spot,
    bench_river_wet_board,
);
criterion_main!(river_benches);
