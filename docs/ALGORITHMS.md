# Algorithms

A self-contained reference for the algorithms this solver implements. If
you haven't read [POKER.md](POKER.md), read that first.

## The core idea: CFR

**Counterfactual Regret Minimization** finds a Nash equilibrium by
iterated self-play. The core loop, in plain English:

1. Start with random strategies for both players.
2. Play every possible hand against itself. At each decision node, compute:
   - What did I actually get (utility of the strategy I played)?
   - What *would* I have gotten if I'd played differently (utility of each
     alternative action)?
3. The difference is **regret**: "I regret not taking action X by this much."
4. Accumulate regrets over all iterations. The next iteration's strategy
   is proportional to the **positive** regret: play the actions you wish
   you'd played more.
5. Average the strategies over all iterations. That average is provably
   close to Nash equilibrium.

That's the whole thing. The tricky part is scale.

## Vanilla CFR — pseudocode

```
fn cfr(history, reach_probs) -> utility per player:
    if is_terminal(history):
        return payoff(history)

    current_player = player_to_act(history)
    info_set = info_set_for(history, current_player)
    strategy = regret_matching(info_set.regret_sum)

    action_utils = zeros(num_actions)
    node_util   = 0

    for action in legal_actions(history):
        next_history = history.append(action)
        new_reach    = reach_probs.copy()
        new_reach[current_player] *= strategy[action]

        sub_util = cfr(next_history, new_reach)
        action_utils[action] = sub_util[current_player]
        node_util += strategy[action] * action_utils[action]

    # Update regrets for THIS info set
    for action in legal_actions(history):
        counterfactual_reach = prod(reach_probs[i] for i != current_player)
        regret = action_utils[action] - node_util
        info_set.regret_sum[action] += counterfactual_reach * regret
        info_set.strategy_sum[action] += reach_probs[current_player]
                                          * strategy[action]

    return node_util (for all players)
```

`regret_matching(r)` is:
```
positive = [max(r_i, 0) for r_i in r]
total    = sum(positive)
if total > 0:
    return [p / total for p in positive]
else:
    return uniform(len(r))
```

At the end, the **average strategy** (cumulative `strategy_sum` normalized)
converges to Nash equilibrium.

That's ~40 lines. It works. For Kuhn Poker it converges in seconds. For
NLHE river subgames it works fine. For NLHE flop subgames you need the
tricks below.

## CFR+ — what we actually ship

Two modifications to vanilla CFR that converge much faster:

1. **Regret matching+** — clamp cumulative regrets to zero on each
   iteration: `info_set.regret_sum[action] = max(0, info_set.regret_sum[action] + new_regret)`.
2. **Linear averaging** — weight recent iterations more heavily in the
   strategy average: iteration `t` contributes with weight `t`.

Published result: CFR+ reaches exploitability levels 1–2 orders of
magnitude faster than vanilla CFR, on the same problem. **This is our
default algorithm.**

## Discounted CFR / Discounted CFR+ — post-v0.1

Brown & Sandholm 2019. Further modification of CFR+ with a discount term
on accumulated regret that varies by iteration. Converges faster still.
Drop-in replacement for CFR+ — same code structure, different update rule.

Plan: ship CFR+ in v0.1, swap in Discounted CFR+ in v0.2 if it measurably
improves convergence on real spots.

## Monte Carlo CFR (MCCFR)

Vanilla CFR traverses the full game tree every iteration. For NLHE turn
subgames, that's prohibitive. MCCFR samples the tree:

- **Outcome Sampling:** sample a single trajectory per iteration. Fastest
  but high variance.
- **External Sampling:** enumerate hero's actions (we need their regrets),
  sample villain's actions. Best bang-for-buck for poker. **This is what
  we use on turn.**
- **Chance Sampling:** sample chance-node outcomes (board cards),
  enumerate everything else. Used when the chance branching factor
  dominates.

External Sampling pseudocode is identical to CFR above, except the loop
over villain's actions picks ONE action weighted by her current strategy,
instead of summing over all. Convergence is slower per iteration but each
iteration is much cheaper — net win when the tree is big.

## Vector CFR — the river hot path

At the river, the game is just a showdown: both players' hands are fixed,
the board is fixed, and the only decisions are checks/bets/calls/folds.
No more chance nodes.

Observation: instead of looping over hero's 1326 combos one at a time,
represent hero's strategy as a **vector** of length 1326 and do the whole
regret update as a matrix operation.

The payoff matrix `P` is `1326 × 1326`:
`P[i][j] = payoff to hero when hero holds combo i, villain holds combo j`.
Precomputed once per river subgame.

Then:
- `hero_equity[i] = dot(P[i], villain_strategy)` — vectorized, trivial
  SIMD
- `regret_update[i] = hero_equity[i] - current_util[i]` — vectorized
- Parallelize across action nodes with `rayon`

On M-series Apple Silicon:
- Pure Rust with `std::simd::f32x8`: expected ~1–2 seconds for 1000 iters
- With Metal compute shader: expected ~100–300 ms for 1000 iters
- Memory: ~20 MB per river subgame

## Card isomorphism

The key observation: **suit labels don't matter if the board doesn't use
them**. On an off-suit flop (say JhTs4c), swapping any two suits gives a
strategically identical game. We canonicalize:

```
fn canonical_board(board: &Board) -> Board:
    # Assign suit labels based on first appearance, by rank desc.
    # E.g., highest rank sees suit A, next new suit sees suit B, etc.
    renumber_suits(board)
```

For a flop, this reduces ~22k distinct boards to ~1,755 strategically
distinct boards. ~12× collapse on flop, more on paired/flushy boards.

The cache is keyed by canonical board + bet tree + SPR bucket. A cache
hit returns the strategy; the consumer un-canonicalizes to the actual
board's suit labels.

## Bet-tree abstraction

Real NLHE has continuous bet sizes. We discretize. For v0.1:

- **Flop:** 33% pot, 66% pot, pot, all-in
- **Turn:** 50% pot, pot, 2× pot, all-in
- **River:** 33% pot, 66% pot, pot, 2× pot, all-in

When villain makes a bet that isn't in our tree (e.g., 47% pot), we snap
to the nearest bucket by ratio. Small accuracy loss, massive speed gain.

The bet tree is a parameter to `HandState` — the consumer can override
defaults per broadcast. E.g., a high-stakes event with unusual sizings
can provide its own tree.

## Convergence metric: exploitability

How do we know when CFR has converged? **Exploitability** measures how
much a best-response opponent could exploit our strategy:

```
exploitability = (max_util_best_response_vs_hero
                   + max_util_best_response_vs_villain) / 2
```

In bb/100 (big blinds per 100 hands). For a Nash strategy, this is 0.
We target < 1% of pot for v0.1.

Computed via a single pass of the "best response" algorithm (a simplified
CFR variant where one player plays deterministically).

## Precomputation as an algorithm

For the flop and preflop, we don't solve live — we precompute offline and
look up at runtime. The "algorithm" becomes:

1. Generate a grid of (canonical_board, SPR_bucket, bet_tree) tuples
   that covers the common broadcast spots.
2. Solve each offline on Colab with high iteration count (5000+ for
   accuracy).
3. Serialize results to a hashmap: `tuple → strategy`.
4. Ship the hashmap as a binary file.
5. At runtime: compute the lookup key from the input `HandState`, fetch.

Cache size vs coverage is a tradeoff we'll tune across v0.1 → v0.2.
Initial ship: ~500 MB, covering the top 80% of tournament-poker spots.

## Validation

The algorithm implementation is validated against TexasSolver:

```
cargo run -p solver-cli -- validate --spot tests/fixtures/spot_001.json
# diffs action_frequencies, ev_per_action, hero_equity
# passes if all within tolerance (5% freq, 0.1bb EV)
```

20 canonical spots must pass. If any diverge, the issue is one of:
- Bug in regret update (most common)
- Bet-tree mismatch (different bets discretized)
- Iteration count differences (run both to convergence)
- Card isomorphism bug (rare but possible)

## References

- Zinkevich et al., *Regret Minimization in Games with Incomplete Information* (2008) — original CFR
- Tammelin, *Solving Large Imperfect Information Games Using CFR+* (2014) — CFR+
- Brown & Sandholm, *Solving Imperfect-Information Games via Discounted Regret Minimization* (2019)
- Brown & Sandholm, *Superhuman AI for heads-up no-limit poker: Libratus beats top professionals* (2017) — MCCFR at scale
- Moravčík et al., *DeepStack: Expert-level artificial intelligence in heads-up no-limit poker* (2017) — CFR-D for real-time
