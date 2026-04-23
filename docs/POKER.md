# Poker for engineers who don't play poker

If you're a Rust engineer who has never played hold'em, read this before
touching any code under `crates/solver-nlhe/` or `crates/solver-eval/`.
The terminology is dense but it's a closed vocabulary — there isn't
that much of it.

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

You make your best 5-card hand from the 2 hole cards + 5 board cards.

## Key terms

- **Hole cards** — your 2 private cards. Abbreviated e.g., "AhKd" (ace of
  hearts, king of diamonds).
- **Board** — the 5 community cards. Sometimes "flop + turn + river"
  collectively.
- **Pot** — the money in the middle that the winner takes.
- **Stack** — how much money a player has left, in chips or big blinds.
- **Effective stack** — the smaller of the two players' stacks (that's
  the most anyone can win from anyone).
- **Position** — who acts last on each street. In heads-up play the
  **Button (BTN)** acts last postflop and first preflop. **Big Blind (BB)**
  is the opposite.
- **Hero / Villain** — convention: "hero" is the player we're solving for,
  "villain" is the opponent.

## Combinatorics

There are:
- **52 cards** × **51** / 2 = **1326 possible hole-card combos**
- **22,100** distinct 3-card flops (including suits)
- **C(52, 2) × C(50, 3) × 47 × 46 ≈ 2.78 × 10¹²** full showdown combos

For solvers, 1326 is the magic number. A **range** is a vector of 1326
weights (each 0 to 1) representing the probability the player holds each
combo.

## Ranges and notation

A **range** describes what hands a player might have. Written compactly:

- **AA** — pocket aces (6 combos: AhAs, AhAd, AhAc, AsAd, AsAc, AdAc)
- **AKs** — A-K suited (4 combos, one per suit)
- **AKo** — A-K offsuit (12 combos)
- **AK** — A-K any (16 combos: 4 suited + 12 offsuit)
- **T9s+** — T9s, J9s, Q9s, K9s, A9s (all suited with 9, nine or higher)
- **22+** — all pocket pairs 22 through AA
- **88-TT** — pocket pairs 88, 99, TT

A typical preflop UTG open range: `22+, AJs+, KTs+, QTs+, AQo+`.

Our `Range` type is a `[f32; 1326]`, with indexing defined in
`solver-eval`. A weight of 1.0 means "always this hand," 0.5 means "half
the time this hand," 0 means "never."

## Bet sizing

Bets are typically expressed as a fraction of the pot:

- **33% pot** — a cautious C-bet (continuation bet)
- **66% pot** — a standard bet
- **Pot** (100%) — a polarizing bet (very strong or bluff)
- **Overbet** (150% pot, 2× pot) — big bet, usually river
- **All-in** — bet everything remaining

Our bet-tree abstraction discretizes continuous bet sizes to 3–5 buckets
per street. See `solver-nlhe::BetTree`.

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
river subgames. This is why memory blows up by street.

## SPR (Stack-to-Pot Ratio)

`SPR = effective_stack / pot`. It controls how much room there is for
action. An SPR of 1 means a single pot-sized bet puts both players all-in;
an SPR of 10 means lots of room for multiple streets of betting.

SPR is a useful bucketing axis for cache keys because two spots with
similar SPR play similarly even if the absolute chip counts differ.

## Equity, EV, GTO — the three concepts the overlay shows

- **Equity** — "given both players run out the board, how often do I win?"
  Trivially computed by Monte Carlo or exact enumeration. Poker Panel
  already ships this.
- **EV (Expected Value)** — "what's the expected chip outcome of taking
  this action, against villain's range?" Depends on villain's strategy.
- **GTO (Game-Theory Optimal)** — "the frequency of each action that makes
  me unexploitable against any opponent strategy." This is what our
  solver produces.

The overlay Henry wants to add shows GTO frequencies ("call 62%, raise
18%, fold 20%"). That's what every line of Rust in this repo exists to
compute.

## Board texture

Not all flops are strategically equal. Broad buckets:

- **Dry board** — e.g., AhKd7c. Few draws, pairs matter.
- **Wet board** — e.g., JhTh9c. Tons of draws (flushes, straights),
  aggressive play.
- **Monotone** — three of same suit. Flushes dominate.
- **Paired** — e.g., 8h8c3d. Trips/full houses matter.
- **Dynamic** — multiple drawy possibilities.

Board texture affects strategy heavily. Our cache bucketing uses a texture
hash function (see `solver-eval::texture`) to group strategically similar
boards.

## Why heads-up (2 players) first

v0.1 solves only heads-up (two-player) pots. Why:

- Multi-way math is qualitatively harder (no Nash equilibrium that's
  simple to compute; usually multiple).
- Broadcast-relevant hands are 80% heads-up by the river anyway (people
  fold before then).
- Heads-up keeps the game tree tractable for live solving.

Multi-way is a v0.3 feature, not v0.1.

## Common hand types for reasoning

- **Premium pair** — AA, KK, QQ, JJ
- **Broadway** — A, K, Q, J, T (the "big five")
- **Suited connectors** — 65s, 76s, 87s, etc. Draw-heavy.
- **Speculative hands** — small pocket pairs (22–55), suited gappers.
- **Junk** — hands like 72o. Always fold.

When reading range strings, these shortcuts show up constantly.

## Further reading

- *Mathematics of Poker* (Chen & Ankenman) — theoretical foundation
- *Modern Poker Theory* (Acevedo) — GTO-era strategy reference
- [PokerStrategy 101 videos](https://www.pokerstrategy.com/) — entry-level
- For the CFR/algorithm side, see [ALGORITHMS.md](ALGORITHMS.md)
