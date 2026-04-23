//! Ported from `engine/rules/equity_optimized.py`. These functions are
//! the heart of Poker Panel's live equity bar — what the viewer sees
//! during a broadcast. They have been cross-validated against
//! PokerStove / Equilab in Poker Panel's own
//! `tests/engine/test_equity_accuracy.py`, which makes them a solid
//! oracle for the solver's internal utility calculations.
//!
//! Scope:
//! * Heads-up (2 players) only. The Python version handles N-way, but
//!   our solver is heads-up for v0.1 and the differential test only
//!   needs to validate that specific case.
//! * Concrete hands only (2 specific hole cards per player). The
//!   Python version has no notion of *ranges* — that's the solver's
//!   job, and range-vs-range equity is validated elsewhere (combine
//!   concrete-vs-concrete oracle + `combo_index` bijection).

use rand::seq::SliceRandom;
use rand::SeedableRng;
use rand_xoshiro::Xoshiro256StarStar;

use solver_eval::{Board, Card, Hand};

use crate::eval::reference_eval_7;
use crate::showdown::reference_showdown_winners;

/// Equity result for two players on a given board. `hero + villain +
/// tie` should always sum to 1.0 (up to floating-point rounding).
///
/// Matches the shape of Poker Panel's Python dict
/// `{seat_id: win_percentage}` but specialized to heads-up and kept as
/// fractions (not percent points) because the solver stores utilities
/// in [0, 1].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Equity {
    /// Fraction of the board space where hero wins outright.
    pub hero: f32,
    /// Fraction of the board space where villain wins outright.
    pub villain: f32,
    /// Fraction where the hands chop (counted separately so downstream
    /// code can split the tie however it wants).
    pub tie: f32,
}

impl Equity {
    /// Hero's share with ties split evenly. This matches what Poker
    /// Panel's UI shows when you toggle "treat ties as half-wins"
    /// (which is the GTO convention).
    pub fn hero_with_ties_split(&self) -> f32 {
        self.hero + self.tie * 0.5
    }
}

/// Reference river-exact equity. Literal port of
/// `engine/rules/equity_optimized.py::exact_river_equity`, specialized
/// to heads-up.
///
/// ```python
/// scores = {p: evaluator.evaluate(board, p.hole) for p in players}
/// best = min(scores.values())
/// winners = [p for p, s in scores.items() if s == best]
/// ```
pub fn reference_exact_river_equity(hero: &Hand, villain: &Hand, board: &Board) -> Equity {
    assert_eq!(board.len, 5, "river equity requires a full 5-card board",);
    // Note: Python *silently* returns 0.0 for each player if a card-
    // parsing error raises (the `except (ValueError, KeyError,
    // IndexError)` catch-all). We don't model that because our
    // `Hand`/`Board` constructors already guarantee validity — there
    // is no parsing to fail.
    let hero_rank = reference_eval_7(hero, board);
    let villain_rank = reference_eval_7(villain, board);

    let winners = reference_showdown_winners(&[hero_rank, villain_rank]);

    match winners.len() {
        1 if winners[0] == 0 => Equity {
            hero: 1.0,
            villain: 0.0,
            tie: 0.0,
        },
        1 => Equity {
            hero: 0.0,
            villain: 1.0,
            tie: 0.0,
        },
        // Both indices present: chop.
        2 => Equity {
            hero: 0.0,
            villain: 0.0,
            tie: 1.0,
        },
        // `winners` can only be 0, 1, or 2 for a 2-element input, so
        // `_` is unreachable — but we take the safe path and treat it
        // as a chop. Matches the Python's "no winners → everyone 0.0"
        // behavior's *spirit*: don't crash on malformed input.
        _ => Equity {
            hero: 0.0,
            villain: 0.0,
            tie: 0.0,
        },
    }
}

/// Ported from `equity_optimized.py::_run_monte_carlo`, specialized to
/// heads-up. Samples `samples` random runouts of the remaining board
/// cards and returns the empirical equity split.
///
/// Deterministic: give the same `seed` and you get the same result,
/// which matters for differential testing. Poker Panel's version uses
/// `random.seed(hash((id(players), time.time())))` which is obviously
/// not deterministic — we substitute a seedable PRNG because tests
/// need reproducibility. The *math* is identical.
pub fn reference_equity_monte_carlo(
    hero: &Hand,
    villain: &Hand,
    board: &Board,
    samples: u32,
    seed: u64,
) -> Equity {
    // Python:
    //     known_cards = set(board + hero.cards + villain.cards)
    //     remaining_deck = [c for c in all 52 if c not in known_cards]
    //
    // Rust: we use a 52-bit bitmask for the known-set, then walk 0..52.
    let mut known: u64 = 0;
    for c in hero
        .0
        .iter()
        .chain(villain.0.iter())
        .chain(board.as_slice().iter())
    {
        known |= 1u64 << c.0;
    }
    let mut deck: Vec<Card> = Vec::with_capacity(52);
    for i in 0..52u8 {
        if (known >> i) & 1 == 0 {
            deck.push(Card(i));
        }
    }
    let cards_needed = 5 - board.len as usize;
    if deck.len() < cards_needed {
        // Matches Python's "not enough cards → 0% for everyone".
        return Equity {
            hero: 0.0,
            villain: 0.0,
            tie: 0.0,
        };
    }

    // Early-out for the river case: no sampling needed, just score and
    // return. This isn't in the Python `_run_monte_carlo` (it special-
    // cases river upstream), but preserving the same output for
    // `samples = 0` falls out naturally below, so we match behavior by
    // accident if not by structure.
    if cards_needed == 0 {
        return reference_exact_river_equity(hero, villain, board);
    }

    let mut rng = Xoshiro256StarStar::seed_from_u64(seed);

    let mut hero_wins: f64 = 0.0;
    let mut villain_wins: f64 = 0.0;
    let mut ties: f64 = 0.0;

    // Reusable scratch space — a mutable permutation of the deck. The
    // Python version does `random.shuffle(deck)` per sample; same idea.
    let mut scratch = deck.clone();

    for _ in 0..samples {
        scratch.shuffle(&mut rng);
        // Take the first `cards_needed` cards as the runout.
        let mut full_board = *board;
        for k in 0..cards_needed {
            full_board.cards[(board.len as usize) + k] = scratch[k];
        }
        full_board.len = 5;

        let hero_rank = reference_eval_7(hero, &full_board);
        let villain_rank = reference_eval_7(villain, &full_board);
        let winners = reference_showdown_winners(&[hero_rank, villain_rank]);
        match winners.as_slice() {
            [0] => hero_wins += 1.0,
            [1] => villain_wins += 1.0,
            [0, 1] => ties += 1.0,
            _ => {} // unreachable for heads-up
        }
    }

    let s = samples as f64;
    if s == 0.0 {
        return Equity {
            hero: 0.0,
            villain: 0.0,
            tie: 0.0,
        };
    }
    Equity {
        hero: (hero_wins / s) as f32,
        villain: (villain_wins / s) as f32,
        tie: (ties / s) as f32,
    }
}

/// Ported from `equity_optimized.py::fast_enumeration_equity`,
/// specialized to heads-up and made *exact* (the Python version has a
/// `max_combinations` cap above which it returns `None` and falls
/// through to Monte Carlo; we do exhaustive enumeration here because
/// this is an oracle, not a live path).
///
/// Enumerates C(47 - #board_dealt, 5 - #board_dealt) remaining boards.
/// For flop (3 board cards) that's C(45, 2) = 990; for turn (4 board
/// cards) that's C(44, 1) = 44; for river it's 1 (trivial). Preflop
/// (0 board cards) is C(48, 5) = 1,712,304 — slow but exact, and the
/// test harness can choose whether to call the MC or enumeration
/// version.
pub fn reference_fast_enumeration_equity(hero: &Hand, villain: &Hand, board: &Board) -> Equity {
    // Same "build remaining deck" setup as Monte Carlo. The Python
    // code duplicates this block across functions; we do too because
    // clarity matters more than DRY in a test oracle.
    let mut known: u64 = 0;
    for c in hero
        .0
        .iter()
        .chain(villain.0.iter())
        .chain(board.as_slice().iter())
    {
        known |= 1u64 << c.0;
    }
    let mut deck: Vec<Card> = Vec::with_capacity(52);
    for i in 0..52u8 {
        if (known >> i) & 1 == 0 {
            deck.push(Card(i));
        }
    }

    let cards_needed = 5 - board.len as usize;

    if cards_needed == 0 {
        return reference_exact_river_equity(hero, villain, board);
    }

    if deck.len() < cards_needed {
        return Equity {
            hero: 0.0,
            villain: 0.0,
            tie: 0.0,
        };
    }

    // Python:
    //     for combo in itertools.combinations(full_deck, cards_needed):
    //         full_board = board_cards + list(combo)
    //         ... evaluate and tally ...
    //
    // Rust: we implement combinations by nested indices. Up to
    // 5-deep for preflop, but `cards_needed` is at most 5 so this is
    // fine.
    let mut hero_wins: u64 = 0;
    let mut villain_wins: u64 = 0;
    let mut ties: u64 = 0;
    let mut total: u64 = 0;

    enumerate_combos(&deck, cards_needed, &mut |runout| {
        let mut full_board = *board;
        for (k, &c) in runout.iter().enumerate() {
            full_board.cards[(board.len as usize) + k] = c;
        }
        full_board.len = 5;

        let hero_rank = reference_eval_7(hero, &full_board);
        let villain_rank = reference_eval_7(villain, &full_board);
        let winners = reference_showdown_winners(&[hero_rank, villain_rank]);
        match winners.as_slice() {
            [0] => hero_wins += 1,
            [1] => villain_wins += 1,
            [0, 1] => ties += 1,
            _ => {}
        }
        total += 1;
    });

    if total == 0 {
        return Equity {
            hero: 0.0,
            villain: 0.0,
            tie: 0.0,
        };
    }
    let t = total as f32;
    Equity {
        hero: hero_wins as f32 / t,
        villain: villain_wins as f32 / t,
        tie: ties as f32 / t,
    }
}

/// Iterate over all `C(deck.len(), k)` combinations, calling `yield_`
/// with each. Tiny recursive implementation; clarity > performance.
fn enumerate_combos<F>(deck: &[Card], k: usize, yield_: &mut F)
where
    F: FnMut(&[Card]),
{
    // Stack of picked cards. Max depth is 5 (preflop runout).
    let mut stack: Vec<Card> = Vec::with_capacity(k);
    fn recurse<F: FnMut(&[Card])>(
        deck: &[Card],
        start: usize,
        k: usize,
        stack: &mut Vec<Card>,
        yield_: &mut F,
    ) {
        if stack.len() == k {
            yield_(stack);
            return;
        }
        let need = k - stack.len();
        // Skip positions that don't leave enough cards.
        for i in start..=(deck.len() - need) {
            stack.push(deck[i]);
            recurse(deck, i + 1, k, stack, yield_);
            stack.pop();
        }
    }
    if k == 0 {
        yield_(&[]);
        return;
    }
    if deck.len() >= k {
        recurse(deck, 0, k, &mut stack, yield_);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aces_beat_kings_on_blank_river() {
        // AA vs KK on 7c 2d 5h 9s Jd — aces full nothing.
        let hero = Hand::parse("AsAc").unwrap();
        let villain = Hand::parse("KsKc").unwrap();
        let board = Board::parse("7c2d5h9sJd").unwrap();
        let e = reference_exact_river_equity(&hero, &villain, &board);
        assert_eq!(e.hero, 1.0);
        assert_eq!(e.villain, 0.0);
        assert_eq!(e.tie, 0.0);
    }

    #[test]
    fn chop_the_board_gives_tie() {
        // Board is a royal flush in spades — the best possible hand is
        // already on the board. Neither player's hole cards can
        // improve it (they're not spades), so both players "play the
        // board" and the pot chops.
        let hero = Hand::parse("2h3h").unwrap();
        let villain = Hand::parse("4c5c").unwrap();
        let board = Board::parse("AsKsQsJsTs").unwrap();
        let e = reference_exact_river_equity(&hero, &villain, &board);
        assert_eq!(e.tie, 1.0);
        assert_eq!(e.hero, 0.0);
        assert_eq!(e.villain, 0.0);
    }

    #[test]
    fn monte_carlo_agrees_with_exact_on_river() {
        // On the river there's no sampling space, so MC should return
        // exactly the same result.
        let hero = Hand::parse("AsAc").unwrap();
        let villain = Hand::parse("KsKc").unwrap();
        let board = Board::parse("7c2d5h9sJd").unwrap();
        let exact = reference_exact_river_equity(&hero, &villain, &board);
        let mc = reference_equity_monte_carlo(&hero, &villain, &board, 100, 12345);
        assert_eq!(exact, mc);
    }

    #[test]
    fn enumeration_on_turn_is_exact() {
        // 2-card matchup with one board card left → we can enumerate
        // all 44 rivers and get exact equity. Sanity: AA vs KK on
        // an all-blank turn — AA should win close to 100%.
        let hero = Hand::parse("AsAc").unwrap();
        let villain = Hand::parse("KsKc").unwrap();
        let board = Board::parse("7c2d5h9s").unwrap();
        let e = reference_fast_enumeration_equity(&hero, &villain, &board);
        // AA vs KK on this blank turn: villain can only win with a K
        // (3 outs out of 44). So hero ≥ 40/44 ≈ 0.909.
        assert!(e.hero > 0.9, "hero equity {}", e.hero);
        // Sum should be ≈ 1.
        let sum = e.hero + e.villain + e.tie;
        assert!((sum - 1.0).abs() < 1e-5, "sum = {sum}");
    }

    #[test]
    fn monte_carlo_determinism() {
        // Same seed → same result, bit-for-bit.
        let hero = Hand::parse("AsAc").unwrap();
        let villain = Hand::parse("KsKc").unwrap();
        let board = Board::parse("7c2d5h9s").unwrap();
        let a = reference_equity_monte_carlo(&hero, &villain, &board, 200, 42);
        let b = reference_equity_monte_carlo(&hero, &villain, &board, 200, 42);
        assert_eq!(a, b);
    }
}
