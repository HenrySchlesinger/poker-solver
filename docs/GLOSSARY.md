# Glossary

Terms you'll run into across docs and code. Organized by domain.

## Poker terminology

- **Action** — fold, check, call, bet, raise, all-in. A discrete decision
  a player makes.
- **Big blind (bb)** — the larger of the two forced blinds. Most poker
  amounts are denominated in big blinds (e.g., "100bb stack").
- **Board** — the community cards face up in the middle. 3 after flop,
  4 after turn, 5 after river.
- **Button (BTN)** — the dealer position. In heads-up, acts last
  postflop, first preflop.
- **Combo** — one specific 2-card hand. "AhKh" is one combo of AK. There
  are 1326 unique combos (52 × 51 / 2).
- **Effective stack** — the smaller of the two players' stacks. The most
  anyone can win from anyone.
- **Equity** — probability of winning if all cards run out with no more
  betting. A static number given the hands and board.
- **EV (Expected Value)** — expected chip outcome of an action, given
  villain's strategy. Changes with opponent modeling.
- **Flop** — the first 3 community cards.
- **GTO (Game-Theory Optimal)** — a strategy that is unexploitable against
  any opponent. Formally, one half of a Nash equilibrium.
- **Heads-up (HU)** — a 2-player pot. v0.1 only solves heads-up.
- **Hero** — convention: the player we're computing strategy for.
- **Hole cards** — a player's 2 private cards.
- **NLHE** — No-Limit Texas Hold'em. The game we solve.
- **Pot** — the chips in the middle. Winner takes.
- **Preflop** — the betting round before any community cards.
- **Range** — the set of hands a player might hold, weighted by
  probability. Represented as `[f32; 1326]` in code.
- **River** — the 5th and final community card, and the betting round
  after it.
- **Showdown** — revealing hole cards at the end to determine the winner.
- **Small blind (sb)** — the smaller forced blind, posted left of dealer.
- **SPR (Stack-to-Pot Ratio)** — `effective_stack / pot`. Key parameter
  for cache bucketing.
- **Street** — a betting round: preflop, flop, turn, or river.
- **Turn** — the 4th community card and the betting round after it.
- **Villain** — convention: the opponent.

## Solver / algorithm terminology

- **Abstraction** — collapsing semantically similar situations into a
  single representative. Card abstraction groups hands; bet abstraction
  discretizes bet sizes.
- **Best response** — the strategy that maximally exploits a given
  opponent strategy. Used to compute exploitability.
- **CFR (Counterfactual Regret Minimization)** — the algorithm family
  this solver uses. Iterates self-play to find Nash equilibrium.
- **CFR+** — the variant we ship. Regrets clamped to non-negative;
  linear averaging. Converges faster than vanilla CFR.
- **Convergence** — the process of an iterated algorithm approaching its
  fixed point. Measured by exploitability in bb or bb/100.
- **Decision node** — a point in the game tree where a player must choose
  an action.
- **Exploitability** — how much a best-response opponent could beat us
  for. Measured in bb/100. Lower = closer to Nash.
- **Game tree** — the tree of all possible sequences of actions and
  chance events. Leaves are showdowns.
- **Info set (Information set)** — the set of game-tree states that are
  indistinguishable from the perspective of one player. In NLHE, an
  info set is (your hand, public action history, board).
- **Iteration** — one pass of CFR+ through the game tree. Our river
  target is 1000 iterations.
- **MCCFR** — Monte Carlo CFR. Samples trajectories instead of
  enumerating. External Sampling is the variant we use on turn.
- **Nash equilibrium** — a strategy profile where no player can improve
  by unilaterally deviating. The fixed point CFR converges to.
- **Node** — a vertex in the game tree. Chance nodes (cards dealt),
  decision nodes (player actions), terminal nodes (showdown or fold).
- **Reach probability** — the product of all players' action
  probabilities that lead to a given info set.
- **Regret** — "how much better would I have done if I'd played
  differently?" Accumulated across iterations.
- **Regret matching** — converting cumulative regrets to a strategy:
  `strategy[a] = max(regret[a], 0) / sum_positive_regrets`.
- **Subgame** — a connected subtree of the full game tree. We solve one
  subgame at a time (river from some decision point, turn from some
  decision point, etc.).
- **Terminal node** — a leaf in the game tree: showdown or everyone
  folded except one.
- **Utility** — the payoff at a terminal node. In poker, chips won (or
  lost) by the player in question.
- **Vector CFR** — reformulation of CFR where per-hand operations become
  matrix-vector ops. The river hot path.

## Implementation / Rust terminology

- **Bet tree** — the discretized set of allowed bet sizes per street.
  E.g., `{33% pot, 66% pot, pot, 2× pot, all-in}`. Parameter of the
  subgame.
- **cbindgen** — tool that generates C headers from Rust source. Used by
  `solver-ffi` to produce `solver.h` for Swift consumption.
- **Criterion** — the Rust benchmarking crate we use. Runs statistical
  timing tests, produces HTML reports.
- **FFI (Foreign Function Interface)** — the C-compatible boundary
  between Rust and Swift. Implemented in `solver-ffi`.
- **HandState** — the `#[repr(C)]` input struct to the FFI. Describes
  a spot to solve.
- **Info set index** — integer key into our flat regret tables. Computed
  from the decision-node number + hero's combo.
- **Isomorphism** — two situations are isomorphic if a suit/card
  renaming maps one to the other. Exploited to collapse the cache by
  ~100×.
- **Rayon** — data-parallelism crate. Used to parallelize across info
  sets.
- **`repr(C)`** — Rust attribute that forces a struct to use C-compatible
  layout. Required for FFI.
- **SIMD** — Single Instruction Multiple Data. f32x8 ops via
  `std::simd`. Used in the river inner loop.
- **SolveResult** — the `#[repr(C)]` output struct from the FFI.
- **SolverHandle** — opaque pointer type in the FFI. Owns scratch memory
  for reuse across calls.
- **Texture (board texture)** — abstract classification of a board's
  strategic character. Used for cache bucketing.

## Product / business terminology

- **CardEYE** — Poker Panel's CV-based hole-card detection system. Feeds
  hand-state data into the overlay.
- **Overlay** — the live graphics rendered on top of the camera feed.
  GTO frequencies, equity bars, pot size, etc.
- **Poker Panel** — the macOS streaming app that consumes this solver.
  `~/Desktop/Poker Panel/`. Shipping now; do not modify during this
  sprint.
- **PokerGFX** — the incumbent poker-streaming graphics product. $999–
  $9,999/yr. Our primary competitor.
- **Vizrt** — enterprise broadcast-graphics vendor. $1,000+/mo. Not a
  direct competitor (no poker offering) but the price anchor.
- **v0.1** — the ship target for this 7-day sprint.
