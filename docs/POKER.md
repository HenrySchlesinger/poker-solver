# Poker for engineers who don't play poker

If you're a Rust engineer who has never played hold'em, read this before
touching any code under `crates/solver-nlhe/` or `crates/solver-eval/`.
The terminology is dense but it's a closed vocabulary — there isn't
that much of it.

Keep [GLOSSARY.md](GLOSSARY.md) open in a tab while you read this; every
bolded term here has a one-line definition there.

## The game in 60 seconds

**No-Limit Texas Hold'em (NLHE)** is a betting game with a standard 52-card
deck.

Each player gets 2 private cards (**hole cards**). Then 5 **community
cards** are revealed over 4 rounds of betting:

| Round | Community cards revealed | Cumulative |
|---|---|---|
| **Preflop** | 0 | 0 |
| **Flop** | 3 | 3 |
| **Turn** | 1 | 4 |
| **River** | 1 | 5 |

Each round ("**street**"), players take turns betting. A player can
**fold** (give up the hand), **check** (pass if no bet to them), **call**
(match the current bet), **bet** (put money in when no one else has),
**raise** (increase an existing bet), or **go all-in** (bet everything).

At showdown (after river betting), remaining players reveal hole cards.
Best 5-card hand from their 2 hole cards + 5 board cards wins the **pot**.

**Blinds** are forced bets — before cards are dealt, the player left of the
dealer posts the **small blind**, next player posts the **big blind (BB)**.
This ensures there's money to play for. Most poker numbers are denominated
in big blinds: "100bb effective stack" means each player has 100× the big
blind.

## Notation cheat-sheet

Everything the solver's inputs and outputs use, in one place.

### Card characters

| Thing | Characters | Example |
|---|---|---|
| Rank (high to low) | `A K Q J T 9 8 7 6 5 4 3 2` | `A` = ace, `T` = ten |
| Suit | `c d h s` | clubs, diamonds, hearts, spades |
| Card | rank + suit | `Ah` = ace of hearts, `2c` = two of clubs |
| Two cards | concatenated | `AhKd` = ace of hearts, king of diamonds |
| Board | concatenated cards | `AhKh2s` (flop), `AhKh2sQc` (turn), `AhKh2sQc4d` (river) |

Cards are parsed case-insensitively by `Card::parse` in
`crates/solver-eval/src/card.rs`. Ranks uppercase, suits lowercase is the
canonical form we emit.

### Hand notation (2 hole cards)

| Token | Meaning | Combos |
|---|---|---|
| `AA` | pocket aces | 6 |
| `AKs` | A-K suited (same suit) | 4 |
| `AKo` | A-K offsuit (different suits) | 12 |
| `AK` | A-K any | 16 |
| `AhKd` | one specific combo | 1 |

"Suited" means both cards share a suit. "Offsuit" means they don't. Pocket
pairs (`AA`, `KK`, etc.) are always offsuit by definition — a pair of
aces is `AhAs`, `AhAd`, etc.

### Range syntax

| Token | Expands to |
|---|---|
| `AA` | pocket aces only |
| `22+` | all pocket pairs: 22, 33, 44, …, AA |
| `77-TT` | pocket pairs 77 through TT inclusive |
| `ATs+` | A-x suited with kicker T or higher: ATs, AJs, AQs, AKs |
| `T9s+` | suited with 9, other card T or higher: T9s, J9s, Q9s, K9s, A9s |
| `AQo+` | A-x offsuit with kicker Q or higher: AQo, AKo |
| `AK` | A-K suited + offsuit (16 combos) |

The `+` means "and stronger kickers": `ATs+` holds the higher card fixed
at A and walks the kicker up from T to K. For pocket pairs, `+` means
"and higher pairs." Multiple tokens are comma-separated, whitespace
ignored: `22+, AJs+, KTs+, QTs+, AQo+`.

### Action strings

Actions appear in logs and test fixtures like `f`, `x`, `c`, `b66`, `r200`.

| Short | Long | Meaning |
|---|---|---|
| `f` | fold | give up the hand |
| `x` | check | pass, no money in (only legal when no outstanding bet) |
| `c` | call | match the current bet |
| `b<pct>` | bet *pct*% pot | e.g., `b66` = bet 66% of the pot |
| `r<pct>` | raise to *pct*% pot | e.g., `r200` = raise to a 2× pot total |
| `a` | all-in | bet remaining stack |

A full action history looks like: `x b66 c / x x / x b50 r150 c` where
`/` separates streets (flop / turn / river in this example; preflop was
already decided).

## Position: who acts when

Every subgame in `crates/solver-nlhe/` has a `to_act` player. That field
matters because **position** is strategy-defining in poker.

In **heads-up** (2-player) NLHE:

- **Button (BTN)** — the dealer. Acts **first preflop**, **last
  postflop** on every subsequent street.
- **Big Blind (BB)** — the non-dealer. Acts **last preflop**, **first
  postflop**.

The asymmetry comes from the forced blinds: BB already has money in the
pot so they get the closing action preflop.

### Why position matters to the solver

A player "in position" (acting last on the current street) sees their
opponent's action before deciding, which is a large strategic edge. Two
spots that look identical except for who's in position have different
GTO strategies.

Concretely, the subgame builder flips the action order based on `to_act`.
If `to_act = BB` and it's the flop, BB checks or bets first, then BTN
responds. On the river, same structure.

Our `Player::Hero` / `Player::Villain` enum is just a labeling convention
for the solver — it does not mean "hero is always BTN." The consumer sets
which chair is which based on the live hand.

## Hand rankings (best to worst)

Standard across all poker:

1. Straight flush (5 consecutive ranks, same suit)
2. Four of a kind
3. Full house (3 of one rank + 2 of another)
4. Flush (5 same suit, any ranks)
5. Straight (5 consecutive ranks, any suits)
6. Three of a kind
7. Two pair
8. One pair
9. High card

You make your best 5-card hand from the 2 hole cards + 5 board cards —
the evaluator picks the best 5 out of 7. See `eval_7` in
`crates/solver-eval/src/eval.rs`.

## Three worked example hands

These walk single hands from deal to showdown so you can see how the
state evolves. Chip amounts are in big blinds. "100bb effective" means
each player starts with 100bb.

### Example 1: clean river check-bet-call

**Setup:** 100bb effective. Villain is BTN, Hero is BB.

```
Preflop:  Villain raises to 2.5bb.   Hero calls.
          pot = 5bb, both have 97.5bb left.

Flop:     Jh 8h 3c (board.len = 3)
          Hero checks.  Villain bets 3bb (approx 60% pot).  Hero calls.
          pot = 11bb.

Turn:     2d (board = Jh8h3c2d, board.len = 4)
          Hero checks.  Villain checks.  (Neither player bet.)
          pot = 11bb.

River:    7s (board = Jh8h3c2d7s, board.len = 5)
          Hero checks.  Villain bets 7bb (approx 64% pot).  Hero calls.
          pot = 25bb.

Showdown: Hero shows Jc9c (one pair, jacks, 9-kicker).
          Villain shows KdQd (king high, no pair).
          Hero wins 25bb.
```

What the solver sees on the river:

- `board = Jh8h3c2d7s` (five cards fixed, no more chance nodes)
- `pot = 11bb` going into the river decision
- `effective_stack = 89bb` for each player
- `to_act = Hero` (BB acts first postflop)
- `action_history = [preflop(v_raise_2.5, h_call), flop(h_check,
  v_bet_3, h_call), turn(h_check, v_check)]`
- Both players' **ranges** are what's left after the preflop + flop +
  turn actions pruned away hands they wouldn't have played this way

The river subgame itself has up to 3 decisions: Hero (check or bet),
Villain's response (call, raise, fold, or — if Hero checked — check or
bet), and possibly Hero's further response. That's the tree
`solver-nlhe` builds.

### Example 2: turn all-in with a draw

**Setup:** 100bb effective. Hero is BTN, Villain is BB.

```
Preflop:  Hero raises to 2.5bb.  Villain re-raises ("3-bet") to 10bb.
          Hero calls.  pot = 20bb, both have 90bb left.

Flop:     9h 8h 2c
          Villain bets 13bb (65% pot).  Hero calls.  pot = 46bb.

Turn:     7h  (board = 9h8h2c7h — now a four-to-a-flush board)
          Villain bets 30bb (65% pot).  Hero goes all-in for 77bb total.
          Villain calls (77bb - 30bb = 47bb more to call for a 123bb pot).
          pot = 200bb.

River:    3d  (no more betting; both already all-in)

Showdown: Hero shows AhKh (flush, ace-high).
          Villain shows 9c9d (three of a kind, nines).
          Hero wins 200bb.
```

Things to notice:

- "3-bet" means the third bet preflop. The BB posted 1bb (blind, "bet
  1"), Hero raised (bet 2), Villain re-raised (bet 3). See the pot-type
  section below for how this cascades into `SRP`, `3BP`, `4BP` cache
  keys.
- On the turn, Hero is all-in with just a **flush draw** (9 hearts
  remain that complete the flush). They don't have a made hand yet.
  This is a **semi-bluff** — no showdown value right now, but a chance
  to improve.
- The river card doesn't matter for betting (both all-in), but it
  decides the hand. `3d` doesn't give Villain a full house, so Hero's
  flush wins.
- In the solver, a turn all-in means the turn subgame is *also*
  effectively solving the river: no more betting decisions, just
  equity-run-out against all possible river cards.

### Example 3: bluff on a paired river

**Setup:** 100bb effective. Hero is BTN, Villain is BB.

```
Preflop:  Hero raises to 2.5bb.  Villain calls.
          pot = 5bb, both have 97.5bb.

Flop:     Qs 7d 4c  (rainbow, dry, Q-high)
          Villain checks.  Hero bets 1.5bb (30% pot, "c-bet").
          Villain calls.  pot = 8bb.

Turn:     Qh  (board = Qs7d4cQh — paired on turn)
          Villain checks.  Hero bets 5bb (62% pot).  Villain calls.
          pot = 18bb.

River:    3h  (board = Qs7d4cQh3h — blank)
          Villain checks.  Hero bets 18bb (100% pot, "pot-sized overbet").
          Villain folds.
          Hero wins 18bb without showdown.
```

What Hero actually had: `8c8d` — a pair of eights that would lose to any
queen or any higher pair. Hero **bluffed**: bet as if they had a strong
hand, with no intent to go to showdown. Villain folded a hand (say
`AdTc`, ace-high, missed everything) that would have beaten Hero.

GTO relevance: on this river texture, with this action history, a
well-tuned solver finds Hero should bluff some of their busted hands
and value-bet some of their strong hands at a ratio that makes Villain
indifferent between calling and folding. The overlay shows "bet 63%,
check 37%" — that frequency *is* the whole point of this repo.

## A small ASCII game tree

This is what `solver-nlhe::NlheSubgame` builds, at the shape level.
Assume we're solving a **river** subgame: both players still have stacks
left, pot is already 20bb, Hero acts first. Bet tree allows
`{check, bet 66% pot, bet pot, all-in}` and the response set at a face-
of-bet node is `{fold, call, raise, all-in}`.

```
                        river decision (Hero)
                                 |
       +-------------------+-----+-----+-------------------+
       |                   |           |                   |
     check              bet 66%      bet pot            all-in
       |                   |           |                   |
 +-----+-----+        +----+----+  +---+---+            +--+--+
 |     |     |        |    |    |  |   |   |            |     |
 x    b66   bet      fold call  r fold call r          fold  call
 |    pot   pot       |   [SD]  |  |  [SD]  |          [FLD] [SD]
[SD]  Hero  Hero     [FLD]      H [FLD]     H
      resp  resp                resp        resp
       .     .                   .           .
```

Legend:

- `x`, `b66`, `bet pot`, `all-in` — action edges taken from the bet
  tree. Each edge is an action the player *can* take at this node.
- `[SD]` — **terminal node, showdown**. Solver calls
  `Game::utility(state, player)`, which returns `+pot` for the winner
  and `-pot` for the loser, by running `range_vs_range_equity` on both
  ranges on this exact board.
- `[FLD]` — **terminal node, fold**. Whoever did not fold wins the pot
  as it stands. No cards revealed.
- `Hero resp` / `H resp` — Hero's response to a raise is another
  decision node with its own `{fold, call, reraise, all-in}` branches.
  Trees grow recursively until stacks run out or someone folds.

At each **decision node**, CFR stores a `regret_sum` and `strategy_sum`
per action per **info set**. An info set is keyed by
`(public action history, acting player's hole cards)` — up to 1326 info
sets per decision node, one per possible hole-card combo the acting
player might hold. At a river with ~10 decision nodes and a 3-action
tree, that's ~30k `(info_set × action)` regret cells, updated every
iteration.

The tree grows faster on earlier streets because each **chance node**
(turn card, river card) branches the tree further. That's why the turn
subgame has ~46× the work of its subordinate river subgames, and the
flop has ~45 × 46 × river-work. See [ARCHITECTURE.md](ARCHITECTURE.md)
for the crate-level view and [ALGORITHMS.md](ALGORITHMS.md) for how
CFR+ consumes this tree.

## Combinatorics

There are:

- **52** × **51** / 2 = **1326 possible hole-card combos**
- **22,100** distinct 3-card flops (including suits)
- **C(52, 2) × C(50, 3) × 47 × 46 ≈ 2.78 × 10¹²** full showdown combos

For solvers, 1326 is the magic number. A **range** is a vector of 1326
weights (each 0 to 1) representing the probability the player holds each
combo. See `crates/solver-nlhe/src/range.rs`.

## Bet sizing

Bets are typically expressed as a fraction of the pot:

- **33% pot** — a cautious **c-bet** (continuation bet — the preflop
  raiser continues aggression on the flop)
- **66% pot** — a standard bet
- **Pot** (100%) — a polarizing bet (very strong or bluff)
- **Overbet** (150% pot, 2× pot) — big bet, usually river
- **All-in** — bet everything remaining

Our bet-tree abstraction discretizes continuous bet sizes to 3–5 buckets
per street. See `solver-nlhe::BetTree`. Villain's real bet gets snapped
to the nearest bucket by pot-fraction ratio.

Why a fraction of the pot, not a fixed amount? Strategies are
pot-relative: betting 10bb into a 5bb pot is an **overbet**; betting
10bb into a 100bb pot is a tiny probe. The fraction generalizes across
pot sizes.

## Streets and their subgame structure

- **Preflop**: ranges are typically fixed by position. We ship static
  precomputed ranges.
- **Flop**: 3 community cards revealed. This is the most important street
  for deep strategy. The spot "flop c-bet vs check-raise" is heavily
  solved.
- **Turn**: 1 more card. Pot is usually big now; stakes are rising.
- **River**: last card. No more chance nodes. This is a pure showdown
  game with just check/bet/call/raise/fold.

The river is the easiest to solve fast. Every "solve the turn" is really
"solve the turn, which includes solving ~46 possible river subgames."
Every "solve the flop" includes ~45 turn subgames each with their own
river subgames. This is why memory blows up by street. See
[LIMITING_FACTOR.md](LIMITING_FACTOR.md) — the river inner loop is
where every optimization lives.

## Pot types: SRP, 3BP, 4BP

These acronyms show up in `crates/solver-nlhe/src/cache.rs` and in Colab
precompute keys:

- **SRP** — **Single-Raised Pot**. Preflop action was one raise, then
  called. Most common pot type. Pot going to flop is ~5.5bb at 100bb
  stacks.
- **3BP** — **3-bet Pot**. Raise, re-raise ("3-bet"), call. Bigger pot,
  tighter ranges. Pot going to flop is ~20bb.
- **4BP** — **4-bet Pot**. Raise, re-raise, re-re-raise, call. Very big
  pot, very narrow ranges (mostly premium hands). Pot going to flop is
  ~45bb.

The same flop texture plays very differently across these because the
**SPR** (stack-to-pot ratio) differs dramatically. The cache keys on
the tuple `(canonical_board, SPR_bucket, pot_type, bet_tree)`.

## SPR (Stack-to-Pot Ratio)

`SPR = effective_stack / pot`. It controls how much room there is for
action. An SPR of 1 means a single pot-sized bet puts both players
all-in; an SPR of 10 means lots of room for multiple streets of betting.

Rough SPR bands and what they play like:

| SPR | Street commitment | Strategy shape |
|---|---|---|
| ≤ 1 | 1 bet = all-in | Mostly binary: shove or fold. |
| 2–4 | 2 bets gets all-in | Tight-value-heavy, few bluffs. |
| 5–10 | Multi-street | Full poker, rich bluff/value mix. |
| 15+ | Very deep | Implied odds matter, sets chase sets. |

SPR is a useful bucketing axis for cache keys because two spots with
similar SPR play similarly even if the absolute chip counts differ.

## Equity, EV, GTO — the three concepts the overlay shows

- **Equity** — "given both players run out the board, how often do I
  win?" Trivially computed by Monte Carlo or exact enumeration. Poker
  Panel already ships this.
- **EV (Expected Value)** — "what's the expected chip outcome of taking
  this action, against villain's range?" Depends on villain's strategy.
- **GTO (Game-Theory Optimal)** — "the frequency of each action that
  makes me unexploitable against any opponent strategy." This is what
  our solver produces.

The overlay Henry wants to add shows GTO frequencies ("call 62%, raise
18%, fold 20%"). That's what every line of Rust in this repo exists to
compute.

## Board texture

Not all flops are strategically equal. Broad buckets:

- **Dry board** — e.g., `AhKd7c`. Few draws, pairs matter.
- **Wet board** — e.g., `JhTh9c`. Tons of draws (flushes, straights),
  aggressive play.
- **Monotone** — three of same suit. Flushes dominate.
- **Rainbow** — three different suits. No flush draws (yet).
- **Paired** — e.g., `8h8c3d`. Trips/full houses matter.
- **Connected** — ranks close together (`9h8s7d`). Straight draws
  matter.
- **Dynamic** — multiple drawy possibilities.

Board texture affects strategy heavily. Our cache bucketing uses a
texture hash function (see `solver-eval::texture`) to group
strategically similar boards.

## Why heads-up (2 players) first

v0.1 solves only heads-up (two-player) pots. Why:

- Multi-way math is qualitatively harder (no Nash equilibrium that's
  simple to compute; usually multiple).
- Broadcast-relevant hands are 80% heads-up by the river anyway (people
  fold before then).
- Heads-up keeps the game tree tractable for live solving.

Multi-way is a v0.3 feature, not v0.1. See
[REQUIREMENTS.md](REQUIREMENTS.md) for the full out-of-scope list.

## Common hand types for reasoning

Shorthand that shows up constantly in comments, test fixtures, and
commit messages:

- **Premium pair** — AA, KK, QQ, JJ.
- **Broadway** — A, K, Q, J, T (the "big five" ranks).
- **Suited connectors** — 65s, 76s, 87s, etc. Draw-heavy.
- **Speculative hands** — small pocket pairs (22–55), suited gappers.
- **Junk** — hands like 72o. Always fold.
- **Nuts** — the best possible hand given the board. "Nut flush" = the
  ace-high flush on a non-paired board.
- **Draw** — a not-yet-made hand with cards that could improve it.
  "Flush draw" = 4 cards to a flush, needs one more on the next street.
- **Bluff** — betting with a hand that can't win at showdown, aiming
  to make villain fold better.
- **Value bet** — betting with a strong hand aiming to be called by
  worse.
- **Semi-bluff** — betting with a draw: wins if villain folds, might
  improve to a winner if called.

## Further reading

- *Mathematics of Poker* (Chen & Ankenman) — theoretical foundation
- *Modern Poker Theory* (Acevedo) — GTO-era strategy reference
- [PokerStrategy 101 videos](https://www.pokerstrategy.com/) — entry-level
- [ALGORITHMS.md](ALGORITHMS.md) — CFR/MCCFR/Vector CFR reference
- [ARCHITECTURE.md](ARCHITECTURE.md) — how these concepts become crates
- [GLOSSARY.md](GLOSSARY.md) — single-line lookups for every term here
</content>
</invoke>