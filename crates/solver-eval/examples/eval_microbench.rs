//! Tiny standalone microbench for `eval_7`.
//!
//! Not a replacement for criterion (that's Day 3's A4 task). Just a
//! sanity check that `eval_7` is in the right ballpark (<1µs per call
//! on M1 Pro). Run with:
//!
//! ```text
//! cargo run -p solver-eval --release --example eval_microbench
//! ```

use std::hint::black_box;
use std::time::Instant;

use solver_eval::board::Board;
use solver_eval::card::Card;
use solver_eval::eval::eval_7;
use solver_eval::hand::Hand;

const ITERS: u64 = 10_000_000;

fn main() {
    // Fixed spot: AhKh vs Qs5s on Kd 9c 2h 7d 4s. Nothing exotic —
    // just a realistic one-pair-vs-high-card showdown.
    //
    // Encoding: `Card(rank << 2 | suit)`, rank 0..13 (Two=0..Ace=12),
    // suit 0..4 (Clubs=0, Diamonds=1, Hearts=2, Spades=3).
    let card = |rank: u8, suit: u8| -> Card { Card((rank << 2) | suit) };
    let hero = Hand([card(12, 2), card(11, 2)]); // Ah, Kh
    let villain = Hand([card(10, 3), card(3, 3)]); // Qs, 5s
    let board = Board {
        cards: [
            card(11, 1), // Kd
            card(7, 0),  // 9c
            card(0, 2),  // 2h
            card(5, 1),  // 7d
            card(2, 3),  // 4s
        ],
        len: 5,
    };

    // Warm-up.
    for _ in 0..100_000 {
        black_box(eval_7(black_box(&hero), black_box(&board)));
    }

    // Hero loop.
    let t0 = Instant::now();
    let mut acc: u64 = 0;
    for _ in 0..ITERS {
        let r = eval_7(black_box(&hero), black_box(&board));
        acc = acc.wrapping_add(u64::from(r.0));
    }
    let elapsed = t0.elapsed();
    black_box(acc);

    let per_call_ns = (elapsed.as_nanos() as f64) / (ITERS as f64);
    println!(
        "eval_7 hero: {} iters in {:?} → {:.2} ns/call ({:.2} Mcalls/s)",
        ITERS,
        elapsed,
        per_call_ns,
        1000.0 / per_call_ns,
    );

    // Villain loop (different hand, same board — exercises the other branch).
    let t0 = Instant::now();
    let mut acc: u64 = 0;
    for _ in 0..ITERS {
        let r = eval_7(black_box(&villain), black_box(&board));
        acc = acc.wrapping_add(u64::from(r.0));
    }
    let elapsed = t0.elapsed();
    black_box(acc);

    let per_call_ns = (elapsed.as_nanos() as f64) / (ITERS as f64);
    println!(
        "eval_7 villain: {} iters in {:?} → {:.2} ns/call ({:.2} Mcalls/s)",
        ITERS,
        elapsed,
        per_call_ns,
        1000.0 / per_call_ns,
    );
}
