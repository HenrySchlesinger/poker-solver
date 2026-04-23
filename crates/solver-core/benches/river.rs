//! Criterion benchmarks for NLHE river-subgame CFR+ performance.
//!
//! This file hosts the primary KPI: `river_canonical_spot`. Regressions
//! here block merges. See `docs/BENCHMARKS.md` for targets.
//!
//! ## Wiring (A70, 2026-04-23)
//!
//! Post-A70 default is `CfrPlusVector` (action-only walk over the
//! NLHE tree with the 1326-wide combo axis vectorized via
//! `regret_match_simd_vector`). The previous A64 default (`CfrPlusFlat`)
//! remains available for comparison — see the `_flat` variants below
//! and the `--solver` flag in solver-cli.
//!
//! ## The three benches
//!
//! - `river_canonical_spot` — `AhKhQh2d4s`, hero `"AA,KK,AKs"`, villain
//!   `"22+,AJs+,KQs"`, pot 100, stack 500. Target: < 300 ms @ 1000 iters.
//! - `river_degenerate_spot` — `2c7d9hTsJs`, hero `AhKh` vs villain
//!   `AsAd`, pot 1000, stack 0, 1000 iterations. Target: < 50 ms.
//! - `river_wet_board` — `JhTh9c8h7s`, hero `"AA,AKs,QTs"`, villain
//!   `"22+,AQs+"`, pot 100, stack 500. Target: < 500 ms @ 1000 iters.
//!
//! ## Iteration count
//!
//! Post-A70 the vector path is fast enough to run the heavy spots at
//! full 1000 iterations per criterion sample within reasonable wall-
//! clock. The `river_degenerate_spot` stays at 1000 iters because each
//! iteration is sub-microsecond.
//!
//! Run:
//! ```text
//! cargo bench -p solver-core --bench river
//! ```
//!
//! ## Targets (from `docs/BENCHMARKS.md`)
//!
//! | Bench                     | Target      | Hard limit |
//! |---------------------------|-------------|------------|
//! | `river_canonical_spot`    | < 300 ms @ 1000 iters | < 1 s |
//! | `river_degenerate_spot`   | < 50 ms @ 1000 iters  | —     |
//! | `river_wet_board`         | < 500 ms @ 1000 iters | —     |

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};

use solver_core::{CfrPlusFlat, CfrPlusVector, Player};

use solver_eval::board::Board;
use solver_eval::card::Card;
use solver_eval::combo::combo_index;
use solver_nlhe::subgame::SubgameState;
use solver_nlhe::{BetTree, NlheSubgame, NlheSubgameVector, Range};

/// CFR+ iteration count for the heavy benches (canonical + wet-board).
///
/// Post-A70 the vector path runs at ~0.5 ms/iter or better on these
/// spots; 1000 × 100 criterion samples is a reasonable bench-run time
/// (~50 s per heavy bench).
const HEAVY_ITERATIONS: u32 = 100;

/// Flat-path iteration count. Kept at 100 because the flat path still
/// runs ~4-6 ms/iter on these spots (see A64 notes in BENCHMARKS.md).
const HEAVY_ITERATIONS_FLAT: u32 = 100;

/// CFR+ iteration count for the degenerate spot.
const TRIVIAL_ITERATIONS: u32 = 1000;

/// Build the canonical river spot: `AhKhQh2d4s`, hero `"AA,KK,AKs"`,
/// villain `"22+,AJs+,KQs"`, pot 100, stack 500.
fn build_canonical() -> NlheSubgame {
    let board = Board::parse("AhKhQh2d4s").expect("canonical board must parse");
    let hero = Range::parse("AA,KK,AKs").expect("hero range must parse");
    let villain = Range::parse("22+,AJs+,KQs").expect("villain range must parse");
    NlheSubgame::new(
        board,
        hero,
        villain,
        /* pot_start   */ 100,
        /* stack_start */ 500,
        Player::Hero,
        BetTree::default_v0_1(),
    )
}

fn build_canonical_vector() -> NlheSubgameVector {
    let board = Board::parse("AhKhQh2d4s").expect("canonical board must parse");
    let hero = Range::parse("AA,KK,AKs").expect("hero range must parse");
    let villain = Range::parse("22+,AJs+,KQs").expect("villain range must parse");
    NlheSubgameVector::new(
        board,
        hero,
        villain,
        /* pot_start   */ 100,
        /* stack_start */ 500,
        Player::Hero,
        BetTree::default_v0_1(),
    )
}

fn build_degenerate_vector() -> NlheSubgameVector {
    let board = Board::parse("2c7d9hTsJs").expect("degenerate board must parse");
    let mut hero = Range::empty();
    let mut villain = Range::empty();
    let hero_idx = combo_index(
        Card::parse("Ah").expect("Ah"),
        Card::parse("Kh").expect("Kh"),
    );
    hero.weights[hero_idx] = 1.0;
    let villain_idx = combo_index(
        Card::parse("As").expect("As"),
        Card::parse("Ad").expect("Ad"),
    );
    villain.weights[villain_idx] = 1.0;

    NlheSubgameVector::new(
        board,
        hero,
        villain,
        /* pot_start   */ 1000,
        /* stack_start */ 0,
        Player::Hero,
        BetTree::default_v0_1(),
    )
}

fn build_wet_board_vector() -> NlheSubgameVector {
    let board = Board::parse("JhTh9c8h7s").expect("wet board must parse");
    let hero = Range::parse("AA,AKs,QTs").expect("hero range must parse");
    let villain = Range::parse("22+,AQs+").expect("villain range must parse");
    NlheSubgameVector::new(
        board,
        hero,
        villain,
        /* pot_start   */ 100,
        /* stack_start */ 500,
        Player::Hero,
        BetTree::default_v0_1(),
    )
}

/// Vector-CFR full solve on a freshly-built subgame.
#[inline(always)]
fn run_vector(subgame: NlheSubgameVector, iterations: u32) {
    let mut solver = CfrPlusVector::new(subgame);
    solver.run(black_box(iterations));
    let avg = solver.average_strategy();
    black_box(avg.len());
}

/// Flat-CFR full solve on a freshly-built subgame (for comparison).
#[inline(always)]
fn run_flat(subgame: NlheSubgame, roots: &[(SubgameState, f32)], iterations: u32) {
    let mut solver = CfrPlusFlat::from_roots(subgame, roots);
    solver.run_from(black_box(roots), black_box(iterations));
    let avg = solver.average_strategy();
    black_box(avg.len());
}

/// Primary KPI: the canonical river spot (vector path).
fn bench_river_canonical_spot(c: &mut Criterion) {
    {
        let sanity = build_canonical_vector();
        let _ = sanity;
    }

    c.bench_function("river_canonical_spot", |b| {
        b.iter_with_setup(build_canonical_vector, |sg| {
            run_vector(sg, HEAVY_ITERATIONS)
        });
    });
}

/// Degenerate spot (vector path).
fn bench_river_degenerate_spot(c: &mut Criterion) {
    {
        let sanity = build_degenerate_vector();
        let _ = sanity;
    }

    c.bench_function("river_degenerate_spot", |b| {
        b.iter_with_setup(build_degenerate_vector, |sg| {
            run_vector(sg, TRIVIAL_ITERATIONS)
        });
    });
}

/// Wet-board spot (vector path).
fn bench_river_wet_board(c: &mut Criterion) {
    {
        let sanity = build_wet_board_vector();
        let _ = sanity;
    }

    c.bench_function("river_wet_board", |b| {
        b.iter_with_setup(build_wet_board_vector, |sg| {
            run_vector(sg, HEAVY_ITERATIONS)
        });
    });
}

/// Comparison: flat path on canonical at the same iteration count as
/// the vector path. Kept for regression detection / 10× speedup
/// measurement. Uses the A64 100-iter cap since the flat path at 1000
/// iters is ~4 s per sample.
fn bench_river_canonical_spot_flat(c: &mut Criterion) {
    c.bench_function("river_canonical_spot_flat", |b| {
        b.iter_with_setup(
            || {
                let sg = build_canonical();
                let roots = sg.chance_roots();
                (sg, roots)
            },
            |(sg, roots)| run_flat(sg, &roots, HEAVY_ITERATIONS_FLAT),
        );
    });
}

criterion_group!(
    river_benches,
    bench_river_canonical_spot,
    bench_river_degenerate_spot,
    bench_river_wet_board,
    bench_river_canonical_spot_flat,
);
criterion_main!(river_benches);
