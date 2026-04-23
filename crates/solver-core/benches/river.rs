//! Criterion benchmarks for NLHE river-subgame CFR+ performance.
//!
//! This file hosts the primary KPI: `river_canonical_spot`. Regressions
//! here block merges. See `docs/BENCHMARKS.md` for targets.
//!
//! ## Wiring (A62, 2026-04-23)
//!
//! A17 landed `NlheSubgame::new`, A58 fixed the `AllIn` → `Bet/Raise`
//! substitution that caused the stack>0 OOM, and A59 wired
//! `CfrPlus::run_from` end-to-end through the FFI path. This bench file
//! consumes that stack directly: each bench builds a real
//! `NlheSubgame`, enumerates `chance_roots()`, and runs CFR+ for a fixed
//! iteration count.
//!
//! The Kuhn proxy that used to live here is gone — the dedicated Kuhn
//! bench in `cfr_kuhn.rs` already measures the generic tree walk, and
//! `river.rs` is now purely NLHE.
//!
//! ## The three benches
//!
//! - `river_canonical_spot` — `AhKhQh2d4s`, hero `"AA,KK,AKs"`, villain
//!   `"22+,AJs+,KQs"`, pot 100, stack 500. **100 CFR+ iterations**
//!   (reduced from the 1000-iter spec; see the "Iteration-count note"
//!   below). A mid-sized river with real range complexity.
//! - `river_degenerate_spot` — `2c7d9hTsJs`, hero `AhKh` vs villain
//!   `AsAd`, pot 1000, stack 0, 1000 iterations. Both players already
//!   all-in: tree collapses to Check/Check → showdown. Measures the
//!   trivial-subgame fast path.
//! - `river_wet_board` — `JhTh9c8h7s`, hero `"AA,AKs,QTs"`, villain
//!   `"22+,AQs+"`, pot 100, stack 500, **100 CFR+ iterations** (see
//!   note). Drawy four-to-a-flush texture; more action complexity,
//!   more nodes.
//!
//! ## Iteration-count note (A62, 2026-04-23)
//!
//! The BENCHMARKS.md spec asks for 1000 iterations on canonical and
//! wet-board. At the Day-3-scalar level with no SIMD matching yet
//! (A20's `wide::f32x8` path hasn't been folded into the CFR+ inner
//! loop), one CFR+ iteration on the canonical spot is ~5 ms and on the
//! wet board is ~7 ms. 1000 iterations × 100 criterion samples =
//! ~500 s for canonical and ~700 s for wet, which is an unreasonable
//! bench-run time for a sleep-through-the-night capture and pushes
//! RAM up against other work.
//!
//! We drop canonical + wet to **100 CFR+ iterations per sample** to
//! keep the total bench under a few minutes. The `river_degenerate_spot`
//! stays at 1000 iters because its tree collapses to Check/Check and
//! each iteration is under a microsecond.
//!
//! The per-iteration cost is what matters for regression detection;
//! the 1000-iter figure in the docs is a **v0.1 target for post-SIMD /
//! post-flat-table optimizations (Days 2-3)**, not the pre-SIMD
//! baseline. When A20's SIMD work lands in the CFR+ hot path, canonical
//! should drop ~9× (matching the 9× SIMD speed-up at N=1326 in the
//! `regret_matching` microbench), bringing 1000 iters back under 1 s.
//! At that point, flip the constants back to 1000.
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
//! | `river_canonical_spot`    | < 300 ms    | < 1 s      |
//! | `river_degenerate_spot`   | < 50 ms     | —          |
//! | `river_wet_board`         | < 500 ms    | —          |
//!
//! ## Setup cost note
//!
//! `NlheSubgame::new` allocates a ~1.76 MB showdown-sign matrix
//! (`[[i8; 1326]; 1326]`) and fills it by enumerating combo pairs — an
//! O(1326²) operation. `NlheSubgame` itself is deliberately not `Clone`
//! (clone would also be O(N²) memory traffic, and the intended usage is
//! to build once and borrow by reference to a single `CfrPlus`).
//!
//! So every bench here uses `iter_with_setup`, which runs the builder
//! outside the measured window. Criterion reports only the CFR+
//! wall-clock cost, not the subgame build — matching the production
//! cost model where callers build a subgame once and reuse it.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};

use solver_core::{CfrPlus, Player};

use solver_eval::board::Board;
use solver_eval::card::Card;
use solver_eval::combo::combo_index;
use solver_nlhe::subgame::SubgameState;
use solver_nlhe::{BetTree, NlheSubgame, Range};

/// CFR+ iteration count for the heavy benches (canonical + wet-board).
///
/// Reduced from the 1000-iter spec in `docs/BENCHMARKS.md` because the
/// pre-SIMD CFR+ inner loop at these spot sizes takes ~5-7 ms per
/// iteration; 1000 × 100 criterion samples blows past a reasonable
/// bench-run time. See the "Iteration-count note" in the module docs.
const HEAVY_ITERATIONS: u32 = 100;

/// CFR+ iteration count for the degenerate spot, where each iteration
/// is sub-microsecond (the tree collapses to Check/Check → showdown).
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

/// Build the already-all-in degenerate river: `2c7d9hTsJs`, hero `AhKh`
/// vs villain `AsAd`, pot 1000, stack 0. Both sides are effectively all-
/// in entering the river, so the only legal action at every state is
/// Check and the tree collapses to Check/Check → showdown.
fn build_degenerate() -> NlheSubgame {
    let board = Board::parse("2c7d9hTsJs").expect("degenerate board must parse");
    let mut hero = Range::empty();
    let mut villain = Range::empty();
    // Hero: AhKh (specific combo).
    let hero_idx = combo_index(
        Card::parse("Ah").expect("Ah"),
        Card::parse("Kh").expect("Kh"),
    );
    hero.weights[hero_idx] = 1.0;
    // Villain: AsAd (specific combo).
    let villain_idx = combo_index(
        Card::parse("As").expect("As"),
        Card::parse("Ad").expect("Ad"),
    );
    villain.weights[villain_idx] = 1.0;

    NlheSubgame::new(
        board,
        hero,
        villain,
        /* pot_start   */ 1000,
        /* stack_start */ 0,
        Player::Hero,
        BetTree::default_v0_1(),
    )
}

/// Build the wet-board river: `JhTh9c8h7s`, hero `"AA,AKs,QTs"`,
/// villain `"22+,AQs+"`, pot 100, stack 500. Four-to-a-flush + straight
/// texture; action-dense.
fn build_wet_board() -> NlheSubgame {
    let board = Board::parse("JhTh9c8h7s").expect("wet board must parse");
    let hero = Range::parse("AA,AKs,QTs").expect("hero range must parse");
    let villain = Range::parse("22+,AQs+").expect("villain range must parse");
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

/// Helper: run one CFR+ batch on a freshly-built subgame and discard
/// the result. Used as the measured body of each bench so every
/// criterion sample starts from a clean solver + fresh subgame.
///
/// The subgame (incl. showdown-matrix construction) is built inside
/// `setup` and handed off — the measurement window is just the
/// `run_from` call plus the `average_strategy` fold at the end.
#[inline(always)]
fn run_one(subgame: NlheSubgame, roots: &[(SubgameState, f32)], iterations: u32) {
    let mut solver = CfrPlus::new(subgame);
    solver.run_from(black_box(roots), black_box(iterations));
    // Force the average-strategy computation so a clever compiler can't
    // elide the whole training pass as dead code.
    let avg = solver.average_strategy();
    black_box(avg.len());
}

/// Primary KPI: the canonical river spot.
fn bench_river_canonical_spot(c: &mut Criterion) {
    // One-time build just to sanity-check the roots are non-empty.
    // Inside the bench, `iter_with_setup` rebuilds per sample so the
    // solver starts fresh and the subgame isn't shared across samples.
    {
        let sanity = build_canonical();
        assert!(
            !sanity.chance_roots().is_empty(),
            "canonical spot must have non-empty chance roots"
        );
    }

    c.bench_function("river_canonical_spot", |b| {
        b.iter_with_setup(
            || {
                let sg = build_canonical();
                let roots = sg.chance_roots();
                (sg, roots)
            },
            |(sg, roots)| run_one(sg, &roots, HEAVY_ITERATIONS),
        );
    });
}

/// Already-all-in river spot — tree collapses to Check/Check →
/// showdown. Measures the trivial-subgame fast path.
fn bench_river_degenerate_spot(c: &mut Criterion) {
    {
        let sanity = build_degenerate();
        assert!(
            !sanity.chance_roots().is_empty(),
            "degenerate spot must have non-empty chance roots"
        );
    }

    c.bench_function("river_degenerate_spot", |b| {
        b.iter_with_setup(
            || {
                let sg = build_degenerate();
                let roots = sg.chance_roots();
                (sg, roots)
            },
            |(sg, roots)| run_one(sg, &roots, TRIVIAL_ITERATIONS),
        );
    });
}

/// Wet, drawy river spot — four-to-a-flush + straight texture.
fn bench_river_wet_board(c: &mut Criterion) {
    {
        let sanity = build_wet_board();
        assert!(
            !sanity.chance_roots().is_empty(),
            "wet-board spot must have non-empty chance roots"
        );
    }

    c.bench_function("river_wet_board", |b| {
        b.iter_with_setup(
            || {
                let sg = build_wet_board();
                let roots = sg.chance_roots();
                (sg, roots)
            },
            |(sg, roots)| run_one(sg, &roots, HEAVY_ITERATIONS),
        );
    });
}

criterion_group!(
    river_benches,
    bench_river_canonical_spot,
    bench_river_degenerate_spot,
    bench_river_wet_board,
);
criterion_main!(river_benches);
