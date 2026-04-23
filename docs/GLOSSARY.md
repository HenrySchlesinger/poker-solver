# Glossary

Terms you'll run into across docs and code. Organized by domain. Longer
prose lives in the dedicated docs — this file is for quick lookups.

Cross-references use "see also: [term]" for terms defined elsewhere in
this file, or `[FILE.md](FILE.md)` for doc links.

## Poker terminology

- **3-bet** — the third wager preflop. Raise is the first *raise*, but
  counts as bet 2 (the big blind is bet 1). A 3-bet is the re-raise over
  the initial raise. See also: [4-bet](#poker-terminology), [SRP](#poker-terminology),
  [3BP](#poker-terminology). Visualized in [POKER.md](POKER.md#three-worked-example-hands)
  example 2.
- **3BP** — **3-bet pot**. Preflop saw a raise and a re-raise, then a
  call. Used as a cache-key component. See also: [SRP](#poker-terminology),
  [4BP](#poker-terminology), [SPR](#poker-terminology).
- **4-bet** — the fourth wager preflop: raise, re-raise ("3-bet"),
  re-re-raise ("4-bet"). Usually premium hands only.
- **4BP** — **4-bet pot**. Preflop saw three raises plus a call. Very
  narrow ranges. See also: [SRP](#poker-terminology), [3BP](#poker-terminology).
- **Action** — fold, check, call, bet, raise, all-in. A discrete decision
  a player makes. See [POKER.md](POKER.md#action-strings) for the
  notation we use in logs. See also: [decision node](#solver--algorithm-terminology).
- **Big blind (bb)** — the larger of the two forced blinds. Most poker
  amounts are denominated in big blinds (e.g., "100bb stack"). See also:
  [small blind](#poker-terminology), [BB (Big Blind position)](#poker-terminology).
- **BB (Big Blind position)** — in heads-up, the non-dealer. Acts last
  preflop, first postflop. Contrast with [BTN](#poker-terminology).
- **Bluff** — betting with a hand that can't win at showdown, aiming to
  make villain fold better. See also: [semi-bluff](#poker-terminology),
  [value bet](#poker-terminology). Example in [POKER.md](POKER.md#example-3-bluff-on-a-paired-river)
  example 3.
- **Board** — the community cards face up in the middle. 3 after flop,
  4 after turn, 5 after river. Represented as `Board` in
  `crates/solver-eval/src/board.rs`. See also: [community card](#poker-terminology),
  [board texture](#poker-terminology).
- **Board texture** — abstract classification of a board's strategic
  character (dry, wet, paired, monotone, rainbow, connected). See also:
  [texture](#implementation--rust-terminology), [dry board](#poker-terminology),
  [wet board](#poker-terminology).
- **Broadway** — the five highest ranks: A, K, Q, J, T. Cards and hands
  built from these play similarly enough to deserve a label.
- **BTN (Button)** — the dealer position. In heads-up, acts last
  postflop, first preflop. See also: [BB](#poker-terminology),
  [position](#poker-terminology).
- **C-bet (continuation bet)** — a bet on the flop by the preflop
  aggressor, continuing aggression from preflop. The most common flop
  bet type.
- **Call** — match the current bet, without raising. See also: [check](#poker-terminology),
  [raise](#poker-terminology).
- **Chance node** — see entry in [solver / algorithm terminology](#solver--algorithm-terminology).
  In poker terms: a point where the next card is dealt from the deck.
- **Check** — pass the action with no money in, only legal when no bet
  is currently facing you. See also: [call](#poker-terminology).
- **Combo** — one specific 2-card hand. `AhKh` is one combo of AK. There
  are 1326 unique combos (52 × 51 / 2). Indexed 0..1326 in
  `crates/solver-eval/src/combo.rs`. See also: [hole cards](#poker-terminology),
  [range](#poker-terminology).
- **Community card** — a face-up card on the board, shared by both
  players. See also: [hole card](#poker-terminology), [board](#poker-terminology).
- **Connected (board)** — three board ranks close together, e.g.,
  `9h8s7d`. Lots of straight-draw potential. See also: [board texture](#poker-terminology).
- **Dead card** — a card known to be out of the deck (in a player's
  hand or on the board), so it can't appear again. Equity / combo
  counts must exclude dead cards. See also: [card removal](#poker-terminology).
- **Card removal** — the effect of known dead cards on a range: if
  Hero holds `AhKh`, Villain's range can't include any combo
  containing `Ah` or `Kh`. This is why combo counts in live spots are
  not simply `C(52,2)`.
- **Decision node** — see entry in [solver / algorithm terminology](#solver--algorithm-terminology).
  In poker terms: a point where a player must choose fold/check/call/bet/raise/all-in.
- **Draw** — a not-yet-made hand with outs that can complete it next
  street. "Flush draw" = four to a flush; "straight draw" = four to a
  straight. See also: [semi-bluff](#poker-terminology), [nuts](#poker-terminology).
- **Dry board** — a flop with few draws and few ways for hands to
  collide (e.g., `AhKd7c`). Contrast with [wet board](#poker-terminology).
  See also: [board texture](#poker-terminology).
- **Effective stack** — the smaller of the two players' stacks. The
  most anyone can win from anyone. See also: [SPR](#poker-terminology),
  [stack](#poker-terminology).
- **Equity** — probability of winning if all cards run out with no more
  betting. A static number given the hands and board. See also:
  [EV](#poker-terminology), [runout](#poker-terminology), [range-vs-range equity](#solver--algorithm-terminology).
- **EV (Expected Value)** — expected chip outcome of an action, given
  villain's strategy. Changes with opponent modeling. See also:
  [equity](#poker-terminology), [GTO](#poker-terminology).
- **Flop** — the first 3 community cards, and the betting round after
  them. See also: [street](#poker-terminology), [preflop](#poker-terminology),
  [turn](#poker-terminology).
- **Flush draw** — four cards of the same suit; one more of that suit
  on the next street makes a flush. See also: [draw](#poker-terminology),
  [semi-bluff](#poker-terminology).
- **Fold** — give up the hand, forfeit chips already in the pot.
- **GTO (Game-Theory Optimal)** — a strategy that is unexploitable
  against any opponent. Formally, one half of a Nash equilibrium. What
  the solver output approximates. See also: [Nash equilibrium](#solver--algorithm-terminology),
  [exploitability](#solver--algorithm-terminology), [EV](#poker-terminology).
- **Heads-up (HU)** — a 2-player pot. v0.1 only solves heads-up. See
  also: [position](#poker-terminology), [BTN](#poker-terminology),
  [BB](#poker-terminology).
- **Hero** — convention: the player we're computing strategy for.
  Contrast with [villain](#poker-terminology).
- **Hole cards** — a player's 2 private cards. See also: [combo](#poker-terminology),
  [community card](#poker-terminology).
- **In position** — the player acting last on a street. Has an
  informational edge. Contrast with out of position. See also:
  [position](#poker-terminology), [BTN](#poker-terminology).
- **Junk** — hands like 72o that fold under any reasonable range.
- **Kicker** — the tie-breaking card when both players have the same
  primary made-hand category. E.g., `AhKc` and `AhQc` on `A72rainbow`
  both have a pair of aces, but the first wins with the K kicker.
- **Monotone (board)** — three cards of the same suit on the flop.
  Flushes dominate. See also: [board texture](#poker-terminology),
  [rainbow](#poker-terminology).
- **NLHE** — No-Limit Texas Hold'em. The game we solve. See [POKER.md](POKER.md)
  for the rules in 60 seconds.
- **Nuts** — the best possible hand given the board. "Nut flush" = the
  A-high flush on a non-paired board. See also: [draw](#poker-terminology).
- **Offsuit (o)** — two cards of different suits. Written e.g., `AKo`.
  Contrast with [suited](#poker-terminology).
- **Overbet** — a bet larger than the current pot (150%, 2×, etc.).
  Usually polarizing (very strong or bluff). See [POKER.md](POKER.md#bet-sizing).
- **Paired (board)** — a board where two community cards share a rank,
  e.g., `Qs7dQh` (paired Qs). Trips/full houses matter.
- **Pocket pair** — two hole cards of the same rank, e.g., `8h8c`. All
  6 combos per rank written as `88`.
- **Position** — who acts last on each street. In heads-up, the
  [BTN](#poker-terminology) is in position postflop; the [BB](#poker-terminology)
  is in position preflop. See [POKER.md](POKER.md#position-who-acts-when)
  for why the solver cares.
- **Pot** — the chips in the middle. Winner takes.
- **Preflop** — the betting round before any community cards. See also:
  [street](#poker-terminology), [flop](#poker-terminology).
- **Premium pair** — AA, KK, QQ, JJ. Shorthand for "very strong
  preflop hand."
- **Rainbow (board)** — three cards of three different suits on the
  flop. No flush draws. See also: [monotone](#poker-terminology),
  [board texture](#poker-terminology).
- **Raise** — increase an existing bet. Contrast with [call](#poker-terminology).
- **Range** — the set of hands a player might hold, weighted by
  probability. Represented as `[f32; 1326]` in `Range::weights`. See
  [POKER.md](POKER.md#range-syntax) for the parser notation. See also:
  [combo](#poker-terminology), [card removal](#poker-terminology).
- **River** — the 5th and final community card, and the betting round
  after it. No more chance nodes. This is THE hot path
  (see [LIMITING_FACTOR.md](LIMITING_FACTOR.md)).
- **Runout** — the sequence of board cards dealt in a given hand; or
  "run the board out" means reveal the remaining cards with no more
  betting. Equity Monte Carlo works by sampling random runouts.
- **Semi-bluff** — betting with a draw: wins if villain folds, might
  improve to a winner if called. See [POKER.md](POKER.md#example-2-turn-all-in-with-a-draw)
  example 2. See also: [bluff](#poker-terminology), [draw](#poker-terminology).
- **Showdown** — revealing hole cards at the end to determine the
  winner. Terminal leaf of the game tree for non-fold outcomes. See
  also: [terminal node](#solver--algorithm-terminology).
- **Small blind (sb)** — the smaller forced blind, posted left of
  dealer. See also: [big blind](#poker-terminology).
- **SPR (Stack-to-Pot Ratio)** — `effective_stack / pot`. Key parameter
  for cache bucketing. Low SPR = shove-or-fold; high SPR = multi-street
  play. See [POKER.md](POKER.md#spr-stack-to-pot-ratio) for the bands.
- **SRP** — **Single-Raised Pot**. Preflop action was one raise, then
  called. Most common pot type. See also: [3BP](#poker-terminology),
  [4BP](#poker-terminology).
- **Stack** — how much money a player has left, in chips or big blinds.
  See also: [effective stack](#poker-terminology).
- **Straight draw** — four cards toward a straight; one more rank
  completes it. Open-ended (two ways to hit) or gutshot (one way).
  See also: [draw](#poker-terminology).
- **Street** — a betting round: preflop, flop, turn, or river.
  Enumerated as `Street` in `crates/solver-nlhe/src/action.rs`.
- **Suited (s)** — two cards of the same suit. Written e.g., `AKs`.
  Contrast with [offsuit](#poker-terminology).
- **Suited connectors** — suited hands with adjacent ranks, e.g.,
  `65s`, `76s`. Draw-heavy.
- **Turn** — the 4th community card and the betting round after it.
  See also: [street](#poker-terminology).
- **Value bet** — betting with a strong hand aiming to be called by
  worse. See also: [bluff](#poker-terminology), [semi-bluff](#poker-terminology).
- **Villain** — convention: the opponent. Contrast with [hero](#poker-terminology).
- **Wet board** — a flop with many draws (flushes, straights), e.g.,
  `JhTh9c`. Contrast with [dry board](#poker-terminology).

## Solver / algorithm terminology

- **Abstraction** — collapsing semantically similar situations into a
  single representative. Card abstraction groups hands; bet abstraction
  discretizes bet sizes. See also: [bet tree](#implementation--rust-terminology),
  [isomorphism](#implementation--rust-terminology), [texture](#implementation--rust-terminology).
- **Average strategy** — the mean of all per-iteration strategies,
  which converges to Nash (not the last iteration, which oscillates).
  See `CfrPlus::average_strategy` in `crates/solver-core/src/cfr.rs`.
- **Best response** — the strategy that maximally exploits a given
  opponent strategy. Used to compute exploitability. See also:
  [exploitability](#solver--algorithm-terminology), [Nash equilibrium](#solver--algorithm-terminology).
- **Canonical board** — the canonicalized (suit-renamed) form of a
  board used as a cache key. Two boards with the same canonical form
  are strategically identical. See also: [isomorphism](#implementation--rust-terminology).
  Implemented in `crates/solver-eval/src/iso.rs`.
- **CFR (Counterfactual Regret Minimization)** — the algorithm family
  this solver uses. Iterates self-play to find Nash equilibrium. See
  also: [CFR+](#solver--algorithm-terminology), [MCCFR](#solver--algorithm-terminology),
  [Vector CFR](#solver--algorithm-terminology), [ALGORITHMS.md](ALGORITHMS.md).
- **CFR+** — the variant we ship. Regrets clamped to non-negative;
  linear averaging. Converges faster than vanilla CFR. Implemented in
  `crates/solver-core/src/cfr.rs`. See also: [Discounted CFR+](#solver--algorithm-terminology).
- **Chance node** — a node in the game tree whose successor is chosen
  randomly, not by a player (e.g., the turn card being dealt). See
  also: [decision node](#solver--algorithm-terminology),
  [terminal node](#solver--algorithm-terminology).
- **Chance sampling** — MCCFR variant that samples chance outcomes
  (board cards) while enumerating player actions. Used when chance
  branching dominates.
- **Convergence** — the process of an iterated algorithm approaching
  its fixed point. Measured by exploitability. See also:
  [exploitability](#solver--algorithm-terminology),
  [Nash equilibrium](#solver--algorithm-terminology).
- **Counterfactual reach** — the product of reach probabilities for
  all players *other than* the updating one. The "counterfactual"
  weighting the regret update uses: value at this node assuming the
  update-target reaches here with probability 1 while opponents play
  their current strategy. See `CfrPlus::walk` in
  `crates/solver-core/src/cfr.rs`.
- **Decision node** — a node in the game tree where a player must
  choose an action. Has one child per legal action. Contrast with
  [chance node](#solver--algorithm-terminology) and
  [terminal node](#solver--algorithm-terminology).
- **Discounted CFR+** — Brown & Sandholm 2019. CFR+ with a discount
  factor on accumulated regret that varies by iteration. Drop-in
  replacement. Post-v0.1. See [ALGORITHMS.md](ALGORITHMS.md#discounted-cfr--discounted-cfr-post-v01).
- **Exploitability** — how much a best-response opponent could beat
  us for. Measured in bb/100. Lower = closer to Nash. For a true Nash
  strategy it's 0. Our v0.1 target: < 1% of pot on river. See also:
  [best response](#solver--algorithm-terminology).
- **External sampling** — MCCFR variant where hero's actions are
  enumerated (so we get regrets for all of them) but villain's actions
  are sampled. Best bang-per-buck for NLHE; used on turn. See
  `crates/solver-core/src/mccfr.rs`. See also:
  [outcome sampling](#solver--algorithm-terminology),
  [chance sampling](#solver--algorithm-terminology).
- **Game tree** — the tree of all possible sequences of actions and
  chance events. Leaves are showdowns or folds. See
  [POKER.md](POKER.md#a-small-ascii-game-tree) for an ASCII picture.
- **Info set (Information set)** — the set of game-tree states that
  are indistinguishable from the perspective of one player. In NLHE,
  an info set is `(your hand, public action history, board)`.
  Identified by `InfoSetId` in `crates/solver-core/src/game.rs`.
- **Iteration** — one pass of CFR+ through the game tree, updating
  regrets and strategy sums. Our river target is 1000 iterations.
- **Kuhn Poker** — a toy 3-card poker game with a known analytical
  Nash equilibrium. Used as a CFR correctness fixture in
  `crates/solver-core/tests/kuhn.rs`. If your CFR impl doesn't
  converge on Kuhn, don't try it on NLHE.
- **Linear averaging** — the "+" in CFR+: iteration `t` contributes
  to `strategy_sum` with weight `t`, so later iterations dominate the
  average. Gives faster convergence than unweighted averaging.
- **MCCFR (Monte Carlo CFR)** — CFR variant that samples trajectories
  instead of enumerating the full tree. Needed for turn subgames. See
  also: [external sampling](#solver--algorithm-terminology),
  [outcome sampling](#solver--algorithm-terminology),
  [chance sampling](#solver--algorithm-terminology).
- **Mixed strategy** — a strategy that assigns positive probability
  to multiple actions (e.g., "bet 60%, check 40%"). Contrast with
  [pure strategy](#solver--algorithm-terminology). GTO strategies are
  often mixed.
- **Nash equilibrium** — a strategy profile where no player can
  improve by unilaterally deviating. The fixed point CFR converges to.
- **Node** — a vertex in the game tree. Three kinds: chance,
  decision, terminal.
- **Outcome sampling** — MCCFR variant that samples a single
  trajectory per iteration. Fastest per-iter, high variance.
- **Pure strategy** — a deterministic strategy: one action with
  probability 1, others 0. Contrast with
  [mixed strategy](#solver--algorithm-terminology).
- **Range-vs-range equity** — the expected showdown outcome when
  both players' hands are sampled from weighted ranges on a fixed
  board. A 1326×1326 matmul. The heart of Vector CFR's river update.
  See `range_vs_range_equity` in `crates/solver-eval/src/equity.rs`.
- **Reach probability** — the product of all players' action
  probabilities that lead to a given info set. See also:
  [counterfactual reach](#solver--algorithm-terminology).
- **Regret** — "how much better would I have done if I'd played
  differently?" Accumulated across iterations. `action_util[a] -
  node_util` per action per info set, weighted by counterfactual reach.
- **Regret matching** — converting cumulative regrets to a strategy:
  `strategy[a] = max(regret[a], 0) / sum_positive_regrets`. If no
  positive regrets, uniform fallback. Implemented in
  `crates/solver-core/src/matching.rs`.
- **Regret matching+** — the CFR+ version: clamp cumulative regrets
  to zero after each update, preventing negative regrets from
  accumulating.
- **Subgame** — a connected subtree of the full game tree. We solve
  one subgame at a time (river from some decision point, turn from
  some decision point, etc.). See `NlheSubgame` in
  `crates/solver-nlhe/src/subgame.rs`.
- **Terminal node** — a leaf in the game tree: showdown or everyone
  folded except one. Solver calls `Game::utility` here.
- **Two-player zero-sum** — the game class CFR is guaranteed to
  converge on. Heads-up NLHE qualifies (chips won by one player equal
  chips lost by the other). See also: [NLHE](#poker-terminology),
  [Nash equilibrium](#solver--algorithm-terminology).
- **Utility** — the payoff at a terminal node. In poker, chips won
  (or lost) by the player in question. See `Game::utility` in
  `crates/solver-core/src/game.rs`.
- **Vector CFR** — reformulation of CFR where per-hand operations
  become matrix-vector ops. The river hot path. See
  [ALGORITHMS.md](ALGORITHMS.md#vector-cfr--the-river-hot-path) and
  [LIMITING_FACTOR.md](LIMITING_FACTOR.md).

## Implementation / Rust terminology

- **Bet tree** — the discretized set of allowed bet sizes per street.
  E.g., `{33% pot, 66% pot, pot, 2× pot, all-in}`. Parameter of the
  subgame. Implemented in `crates/solver-nlhe/src/bet_tree.rs`. See
  also: [abstraction](#solver--algorithm-terminology),
  [snap](#implementation--rust-terminology).
- **Bucketing** — grouping strategically-similar situations under a
  single key for caching. See also: [texture](#implementation--rust-terminology),
  [SPR bucket](#implementation--rust-terminology), [abstraction](#solver--algorithm-terminology).
- **cbindgen** — tool that generates C headers from Rust source. Used
  by `solver-ffi` to produce `solver.h` for Swift consumption. See
  [ARCHITECTURE.md](ARCHITECTURE.md#the-ffi-contract-solver-ffi).
- **Criterion** — the Rust benchmarking crate we use. Runs
  statistical timing tests, produces HTML reports. See
  [BENCHMARKS.md](BENCHMARKS.md).
- **FFI (Foreign Function Interface)** — the C-compatible boundary
  between Rust and Swift. Implemented in `solver-ffi`. See also:
  [repr(C)](#implementation--rust-terminology),
  [cbindgen](#implementation--rust-terminology),
  [SolverHandle](#implementation--rust-terminology).
- **HandState** — the `#[repr(C)]` input struct to the FFI.
  Describes a spot to solve. See
  [REQUIREMENTS.md](REQUIREMENTS.md#functional) for the layout.
- **Info set index** — integer key into our flat regret tables.
  Computed from the decision-node number + hero's combo. See also:
  [info set](#solver--algorithm-terminology).
- **Isomorphism** — two situations are isomorphic if a suit/card
  renaming maps one to the other. Exploited to collapse the cache by
  ~100×. Implemented in `crates/solver-eval/src/iso.rs`. See also:
  [canonical board](#solver--algorithm-terminology).
- **Rayon** — data-parallelism crate. Used to parallelize across info
  sets in the CFR inner loop.
- **`repr(C)`** — Rust attribute that forces a struct to use
  C-compatible layout. Required for FFI. See
  [ARCHITECTURE.md](ARCHITECTURE.md#the-ffi-contract-solver-ffi).
- **SIMD** — Single Instruction Multiple Data. `f32x8` ops via
  `std::simd`. Used in the river inner loop. See
  [LIMITING_FACTOR.md](LIMITING_FACTOR.md#2-simd-inner-loop-day-3).
- **Snap** — quantize a real-valued bet to the nearest bet-tree
  bucket. When Villain makes a 47%-pot bet and our tree only has
  33%/66%/100%, we snap to 33%. See `BetTree::snap` in
  `crates/solver-nlhe/src/bet_tree.rs`.
- **SolveResult** — the `#[repr(C)]` output struct from the FFI.
  Contains action frequencies, EVs, equity, convergence delta. See
  [REQUIREMENTS.md](REQUIREMENTS.md#functional).
- **SolverHandle** — opaque pointer type in the FFI. Owns scratch
  memory for reuse across calls. See
  [ARCHITECTURE.md](ARCHITECTURE.md#why-opaque-handles-not-free-functions).
- **SPR bucket** — a discretized SPR value (e.g., 1, 3, 6, 10, 20, 50)
  used as a cache-key component. See also: [SPR](#poker-terminology),
  [bucketing](#implementation--rust-terminology).
- **Texture (board texture)** — abstract classification of a board's
  strategic character. Used for cache bucketing. Implemented in
  `crates/solver-eval/src/texture.rs`. See also:
  [board texture](#poker-terminology).

## Product / business terminology

- **CardEYE** — Poker Panel's CV-based hole-card detection system.
  Feeds hand-state data into the overlay.
- **Overlay** — the live graphics rendered on top of the camera feed.
  GTO frequencies, equity bars, pot size, etc. The consumer of this
  solver's output.
- **Poker Panel** — the macOS streaming app that consumes this solver.
  `~/Desktop/Poker Panel/`. Shipping now; do not modify during this
  sprint.
- **PokerGFX** — the incumbent poker-streaming graphics product. $999–
  $9,999/yr. Our primary competitor. Ships no GTO.
- **TexasSolver** — open-source NLHE solver we validate against on the
  20 canonical spots. See [REQUIREMENTS.md](REQUIREMENTS.md#quality-targets)
  and [ALGORITHMS.md](ALGORITHMS.md#validation).
- **Vizrt** — enterprise broadcast-graphics vendor. $1,000+/mo. Not a
  direct competitor (no poker offering) but the price anchor.
- **v0.1** — the ship target for this 7-day sprint. See
  [ROADMAP.md](ROADMAP.md).
</content>
</invoke>