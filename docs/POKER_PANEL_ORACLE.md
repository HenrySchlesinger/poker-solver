# Poker Panel → Poker Solver oracle map

> *Day 1, 2026-04-22 — Agent A8*

This document catalogs what poker-math code lives in
`~/Desktop/Poker Panel/` and how we port it into a Rust test oracle at
`crates/solver-eval-reference/` for differential testing of
`solver-eval`.

Poker Panel is the macOS streaming app that will consume this solver
via FFI. Its poker-math code has been running on live streams against
real hand detections for months. That makes it a valuable second
opinion when we're in week 1 of writing a brand-new Rust solver and
need to be sure `eval_7`, `equity`, and friends are correct.

## TL;DR: what to port, what to skip

| Concept | Poker Panel has it? | Worth porting? |
|---|---|---|
| 5-card / 7-card hand evaluation | Yes (via `treys`) | **Yes** — core oracle |
| Heads-up exact river equity | Yes | **Yes** |
| Monte Carlo equity (pre-river) | Yes | **Yes** |
| Full enumeration equity | Yes | **Yes** |
| Preflop 169-class canonicalization | Yes | **Yes** |
| Showdown winner tie-breaking | Yes | **Yes** |
| Range parser (`"AA, KK, AKs, T9s+"`) | **No** — deals only in concrete 2-card hands | N/A |
| Bet-tree / action-sequence logic | Implicit in `engine/betting.py` but fused with chip accounting, not GTO | Skip |
| Pot-odds / EV | Not as a reusable function | Skip |
| Side-pot allocation | Yes (`holdem.py::allocate_side_pots`, `pot_calculator.py`) | Skip (not solver math) |
| ICM / multi-way | No | N/A (out of v0.1 scope anyway) |
| PLO hand evaluation | Yes (`plo.py::evaluate_plo_hand`) | Skip (not NLHE v0.1 scope) |

## File-by-file survey

All paths are inside `~/Desktop/Poker Panel/`. **Read-only**.

### `engine/rules/holdem.py` — 290 LOC, Python

Texas Hold'em game-level helpers. Wraps `treys.Evaluator` from the
[treys](https://github.com/ihendley/treys) library, which is Poker
Panel's canonical hand evaluator.

Relevant functions:

* `code_to_treys(code: str) -> int` *(L69)* — converts our internal
  card-string format (e.g. `"AS"`) into the `treys` integer
  representation. Same idea as `solver_eval::eval::to_rs`.
* `evaluate_best_five_of_seven(hole, board) -> Tuple[int, List[int]]`
  *(L81)* — the hot path. Takes 2 hole cards + 5 board cards and
  returns `(score, [])` where `score` is `treys`' 1..=7462
  lower-is-better ranking.
* `calculate_hand_odds(hole_cards, board, num_opponents, simulations)`
  *(L88)* — Monte Carlo equity against N *random* opponent hands.
  Used by `ingest/sim_worker.py` for stream-overlay "chance of
  winning" badges with unknown villain hole cards.
* `calculate_player_percentages(seats, board, street, simulations)`
  *(L159)* — N-way Monte Carlo equity with *known* hole cards per
  seat. This is the live-stream equity bar shown on screen. In
  practice it's overridden by the faster
  `equity_optimized.py::get_cached_equity`.

Edge cases worth knowing about (from the Python comments):
* On river with a full board, it evaluates exact winners via
  `treys.evaluate` instead of sampling.
* It handles split pots via `win_share = 1.0 / len(winners)`.
* `treys.evaluate` can raise on bad input; the Python catches
  `(ValueError, KeyError, IndexError)` and assigns score `99999`
  (worst possible). Our Rust types make this category of failure
  impossible at construction, so we don't model it.

### `engine/rules/equity.py` — 493 LOC, Python

First-generation equity calculator. Superseded in practice by
`equity_optimized.py` but the code is still loaded as a fallback.

Relevant functions (duplicated, roughly, in `equity_optimized.py`):

* `PREFLOP_HEADS_UP` *(L21)* — hardcoded lookup table of classic
  matchup equities ("AA vs KK = 81.9/0.5", etc.). ~80 entries.
  Poker Panel checks this first for heads-up preflop with no dead
  cards.
* `normalize_hand(cards: List[str]) -> str` *(L140)* — canonicalize
  concrete cards to a 169-class string (`"AA"`, `"AKs"`, `"AKo"`).
* `exact_river_equity(players, board)` *(L194)* — river enumeration.
* `fast_enumeration_equity(players, board, max_combinations=1000)`
  *(L232)* — exhaustive enumeration when feasible; returns `None` if
  combinations > cap, caller falls through to Monte Carlo.
* `optimized_simulation_equity(players, board, simulations=1000)`
  *(L316)* — vanilla Monte Carlo, single-threaded.
* `calculate_fast_equity(seats, board, street, dead_cards=None)`
  *(L391)* — dispatches to the right strategy.

### `engine/rules/equity_optimized.py` — 666 LOC, Python

The production equity path. Same public interface, but:
* Loads a full 169×169 JSON preflop lookup table from disk (built by
  `scripts/generate_preflop_table.py` with 50k sims per matchup).
* Uses `ThreadPoolExecutor` for parallel Monte Carlo (4 workers).
* LRU cache (max 500 entries) with exact-player-set invalidation.
* Handles **dead cards** — folded players' known hole cards are
  removed from the deck before sampling.

Relevant functions (these are what `solver-eval-reference` mirrors):

* `normalize_hand(cards)` *(L95)* — same as `equity.py` version.
* `exact_river_equity(players, board)` *(L141)* — same as `equity.py`
  version, slightly more defensive error handling.
* `_run_monte_carlo(players, board, simulations, dead_cards=None)`
  *(L179)* — single-threaded MC worker. Our Rust port is
  deterministic (seeded PRNG) because tests need reproducibility; the
  Python uses `random.seed(hash((id(players), time.time())))` which
  is intentionally non-deterministic for live play.
* `threaded_simulation_equity(players, board, total_simulations)`
  *(L254)* — fan out `_run_monte_carlo` across 4 threads. We don't
  port the threading — the oracle runs serial. Correctness is
  identical; only speed differs.
* `fast_enumeration_equity(players, board, max_combinations=2000)`
  *(L294)* — exhaustive enumeration with a combo cap. Our port drops
  the cap because the oracle only gets called with flops/turns where
  C(deck_remaining, cards_needed) is already small (<1000).
* `calculate_fast_equity(seats, board, street, dead_cards)` *(L367)*
  — the main dispatcher. Too policy-heavy to port literally; the
  underlying building blocks we do port are enough to recompose it in
  a differential test if needed.

Worth noting: the Python code has a well-tested asymmetry where
river (board=5) skips the preflop-lookup path and goes straight to
exact evaluation, and dead-card handling is only valid pre-river.
Our solver is heads-up-only for v0.1 so this policy doesn't apply to
us; we just keep the primitive functions.

### `engine/rules/plo.py` — 543 LOC, Python

Pot-Limit Omaha. Not in v0.1 scope, not ported. Contains a classic
"best-of-C(4,2)·C(5,3)=60" PLO evaluator wrapping the same `treys`
library. Listed here so that when we do v0.3 PLO support, this is
the reference to mirror.

### `engine/showdown.py` — 793 LOC, Python

Mixin that runs the tail end of a hand (all-in checks, river
complete, side pot splitting, auto-payout). Mostly chip accounting
and game-state transitions — not GTO-relevant math. The one
GTO-relevant pattern is the "find winners" loop:

```python
scores = {seat: treys.evaluate(board, seat.hole) for seat in players}
best = min(scores.values())
winners = [sid for sid, s in scores.items() if s == best]
```

That pattern recurs in ~4 places (`showdown.py`, both equity files,
`holdem.calculate_player_percentages`) and is captured in our
`reference_showdown_winners`.

### `scripts/generate_preflop_table.py` — 190 LOC, Python

Builds the 169×169 preflop lookup table with 50k-sim Monte Carlo per
matchup. Not ported as-is (it writes JSON to disk; we don't need
that), but its **canonical-hand enumeration** is ported as
`reference_build_canonical_hands_169` because it defines the
canonical ordering of preflop classes.

Key function:
* `build_canonical_hands() -> List[str]` *(L37)* — yields 169 strings
  in "Ace-first" order (`"AA"`, `"AKs"`, `"AKo"`, `"AQs"`, …,
  `"22"`).
* `canonical(cards) -> str` *(L50)* — same logic as `normalize_hand`.

### `tests/engine/test_equity_accuracy.py` — 530 LOC, Python

**Not ported**, but this file is valuable context. It validates Poker
Panel's equity calculator against known-good PokerStove / Equilab
reference values:

```
AA vs KK on empty board: expected 81.9% / 18.1%, tolerance 2.5%
```

That means when we say "Poker Panel is a good oracle," we have
evidence: it has been pinned to ground-truth values for months. If
our Rust solver diverges from the Rust port of that same code, one
of:
1. We ported wrong,
2. Poker Panel has regressed (unlikely — the test above would catch
   it),
3. Our solver has a real bug.

### `macos/Poker Panel/*.swift`

**No relevant Swift poker-math code.** The macOS app renders an
overlay with equity percentages fetched from the Python backend; it
does not compute any poker math itself. `GameState.swift` contains
only `win_percentage` as a decoded JSON field. A grep for
`func\s+(evaluate|equity|rankHand|…)` across `macos/**/*.swift`
returns only SwiftUI view-layer functions (`equityBadgeBackground`,
`EnvironmentChecker.evaluate`).

So: the entire poker-math surface in Poker Panel lives in Python,
under `engine/rules/`. That's a good thing — a single source of
truth to port.

## Rust crate: `crates/solver-eval-reference`

Structure:

```
crates/solver-eval-reference/
├── Cargo.toml
└── src/
    ├── lib.rs       — top-level module exports
    ├── eval.rs      — reference_eval_5, reference_eval_7
    ├── equity.rs    — reference_exact_river_equity,
    │                  reference_equity_monte_carlo,
    │                  reference_fast_enumeration_equity
    ├── preflop.rs   — reference_normalize_hand_169,
    │                  reference_build_canonical_hands_169
    └── showdown.rs  — reference_showdown_winners
```

### Ported functions

| Name | From (Python) | Status |
|---|---|---|
| `reference_eval_7` | `holdem.evaluate_best_five_of_seven` | Done |
| `reference_eval_5` | (microbench helper) | Done |
| `reference_exact_river_equity` | `equity_optimized.exact_river_equity` | Done (heads-up) |
| `reference_equity_monte_carlo` | `equity_optimized._run_monte_carlo` | Done (heads-up, seeded) |
| `reference_fast_enumeration_equity` | `equity_optimized.fast_enumeration_equity` | Done (heads-up, exact) |
| `reference_normalize_hand_169` | `equity_optimized.normalize_hand` | Done |
| `reference_build_canonical_hands_169` | `scripts/generate_preflop_table.build_canonical_hands` | Done |
| `reference_showdown_winners` | recurring `argmin/winners` pattern | Done |

### Independence from `rs_poker`

The reference evaluator is a **from-scratch implementation** in
`eval.rs`, not a wrapper around `rs_poker`. This is deliberate:

* `solver-eval` wraps `rs_poker`. If `solver-eval-reference` also
  wrapped `rs_poker` we would "bless" any upstream bug on both
  sides and never catch it via differential testing.
* The categorical 5-card evaluator (straight flush → high card) plus
  C(7,5)=21 best-of for 7-card evaluation is ~200 LOC of obvious,
  auditable code. Slow compared to rs_poker's bit tricks, but this
  is an oracle — clarity beats speed.
* It also sidesteps the Day-1 toolchain issue (see "Known gaps"
  below).

### Differential tests

`crates/solver-eval/tests/differential.rs` wires both crates together
and runs:

1. `eval_7_matches_reference_on_random_inputs` — 10,000 seeded
   random (hand, river-board) pairs, assert
   `eval_7(h,b) == reference_eval_7(h,b)` exactly.
2. `eval_7_ordering_matches_reference_on_random_pairs` — 2,000 pairs
   of scenarios, assert the two evaluators produce the same
   ordering (catches bugs where both sides compute the same wrong
   monotonic value).

As more `solver-eval` pieces land (Day 2+: `equity` module), we
extend this file with:
* Concrete-hand equity matches between our production MC and the
  reference MC (same seed).
* Exact river equity matches `reference_exact_river_equity`.
* `combo_index` round-trips through `reference_normalize_hand_169`.

## Known gaps / risks

### 1. rs_poker 4.1.0 requires Rust 1.85+, workspace pinned to 1.82

This blocks `cargo check -p solver-eval` (and therefore the
differential test, which dev-depends on solver-eval-reference, which
depends on solver-eval types).

* Root cause: `rs_poker = "4.1"` in
  `crates/solver-eval/Cargo.toml`, but `rust-toolchain.toml`
  pins `1.82.0`. rs_poker 4.1.0 is declared `edition = "2024"`.
* Resolution (for A2 / toolchain owner):
  * Either bump `rust-toolchain.toml` to `1.85.0`,
  * Or downgrade `rs_poker` to the last edition-2021 release that
    builds on stable (2.1.1 — but the API is older, would need
    eval.rs rewriting; likely just bump the toolchain).
* Impact on this crate: our `solver-eval-reference` has no
  rs_poker dep, so it *would* compile — except that cargo resolves
  the whole workspace before checking any individual crate.
* Verified independently: I built a scratch copy of
  `solver-eval-reference` (with a stubbed solver-eval dep) on the
  pinned 1.82 toolchain. All 22 unit tests in this crate pass.

### 2. N-way equity not ported

Poker Panel supports up to 8 players; our solver is heads-up in v0.1
(see `docs/ARCHITECTURE.md`). The oracle mirrors that — only
heads-up. Extending to N-way is a 20-line change per function if
we ever need it, but premature work now.

### 3. Range parsing has no oracle

Poker Panel does not implement a string-range parser
(`"AA, KK, AKs, T9s+"`). That's because Poker Panel deals in
concrete hands (seat has 2 specific cards), not solver-style range
vectors. Agent A3 (Day 1) has to write this from scratch;
`crates/solver-eval-reference` cannot provide an oracle. The
canonical-169 enumeration we port IS useful as a sanity check for
"does A3's parser accept every hand that appears in Poker Panel's
preflop_table.json", but that's a separate test.

### 4. The Python `catch (ValueError, KeyError, IndexError)` pattern

Poker Panel's equity functions defensively wrap evaluator calls and
assign a sentinel "worst hand" score (`99999`) on parsing errors.
Our Rust types (`Hand`, `Board`) make this class of error
impossible at construction, so the oracle doesn't model the
sentinel. If the production path ever introduces a path where an
invalid board can reach the evaluator, the oracle won't catch it —
but the production path would panic, which is louder and better.

## Handoff notes for Days 2–7

* When A3 lands `solver_eval::equity::hand_vs_hand_equity`, append
  differential tests to `differential.rs` using
  `reference_exact_river_equity` as truth (river, exact) and
  `reference_equity_monte_carlo` as truth (MC, same seed).
* When A3 lands `solver_eval::equity::range_vs_range_equity`, there
  is no direct oracle in Poker Panel — that's a new algorithm.
  Instead, validate via the relationship
  `range_vs_range_equity = weighted_sum(reference_equity for every
  concrete pair)`. That's a 1326² = 1.76M concrete equities per
  range-pair test which is slow but exact.
* If anyone ports the 169-class preflop equity *table* into Rust
  (shipping as a static binary asset), use
  `reference_build_canonical_hands_169` as the ordering pin.
  Mis-ordering that array would be invisible until Day 7 validation.
* `reference_showdown_winners` returns a `Vec<usize>` of indices;
  when side-pot logic lands in `solver-nlhe` (not in v0.1 scope),
  this is the function shape to mirror.
