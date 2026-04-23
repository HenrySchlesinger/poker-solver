//! Criterion bench for the Vector CFR river subgame solver.
//!
//! Mirrors the three benches in `river.rs` (canonical, degenerate,
//! wet-board) but drives `CfrPlusVector` / `NlheSubgameVector`. This is
//! the v0.2 hot path — a single batched walk per CFR+ iteration with
//! SIMD regret matching across the 1326 combo lanes.
//!
//! Run:
//! ```text
//! cargo bench -p solver-core --bench river_vector
//! ```

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};

use solver_core::{CfrPlusVector, Player};

use solver_eval::board::Board;
use solver_eval::card::Card;
use solver_eval::combo::combo_index;
use solver_nlhe::{BetTree, NlheSubgameVector, Range};

/// CFR+ iteration count for heavy benches. Post-M4 this matches
/// `river.rs`'s `HEAVY_ITERATIONS` for easy cross-comparison.
const HEAVY_ITERATIONS: u32 = 100;

/// Count matching `river.rs`'s degenerate bench (sub-microsecond per
/// iter, so more iterations for better signal).
const TRIVIAL_ITERATIONS: u32 = 1000;

fn build_canonical() -> NlheSubgameVector {
    let board = Board::parse("AhKhQh2d4s").expect("canonical board");
    let hero = Range::parse("AA,KK,AKs").expect("hero range");
    let villain = Range::parse("22+,AJs+,KQs").expect("villain range");
    NlheSubgameVector::new(
        board,
        hero,
        villain,
        100,
        500,
        Player::Hero,
        BetTree::default_v0_1(),
    )
}

fn build_degenerate() -> NlheSubgameVector {
    let board = Board::parse("2c7d9hTsJs").expect("degenerate board");
    let mut hero = Range::empty();
    let mut villain = Range::empty();
    hero.weights[combo_index(Card::parse("Ah").unwrap(), Card::parse("Kh").unwrap())] = 1.0;
    villain.weights[combo_index(Card::parse("As").unwrap(), Card::parse("Ad").unwrap())] = 1.0;
    NlheSubgameVector::new(
        board,
        hero,
        villain,
        1000,
        0,
        Player::Hero,
        BetTree::default_v0_1(),
    )
}

fn build_wet_board() -> NlheSubgameVector {
    let board = Board::parse("JhTh9c8h7s").expect("wet board");
    let hero = Range::parse("AA,AKs,QTs").expect("hero range");
    let villain = Range::parse("22+,AQs+").expect("villain range");
    NlheSubgameVector::new(
        board,
        hero,
        villain,
        100,
        500,
        Player::Hero,
        BetTree::default_v0_1(),
    )
}

#[inline(always)]
fn run_one(sg: NlheSubgameVector, iterations: u32) {
    let mut solver = CfrPlusVector::new(sg);
    solver.run(black_box(iterations));
    let avg = solver.average_strategy();
    black_box(avg.len());
}

fn bench_vector_canonical(c: &mut Criterion) {
    c.bench_function("river_canonical_spot_vector", |b| {
        b.iter_with_setup(build_canonical, |sg| run_one(sg, HEAVY_ITERATIONS));
    });
}

fn bench_vector_degenerate(c: &mut Criterion) {
    c.bench_function("river_degenerate_spot_vector", |b| {
        b.iter_with_setup(build_degenerate, |sg| run_one(sg, TRIVIAL_ITERATIONS));
    });
}

fn bench_vector_wet_board(c: &mut Criterion) {
    c.bench_function("river_wet_board_vector", |b| {
        b.iter_with_setup(build_wet_board, |sg| run_one(sg, HEAVY_ITERATIONS));
    });
}

criterion_group!(
    river_vector_benches,
    bench_vector_canonical,
    bench_vector_degenerate,
    bench_vector_wet_board,
);
criterion_main!(river_vector_benches);
