# Why we're building this

## The product it feeds

[Poker Panel](~/Desktop/Poker%20Panel) is a $60/mo macOS live-streaming app
for poker tournaments. It captures up to 8 cameras, detects hole cards with
computer vision ("CardEYE"), renders live overlays, and auto-generates
~20 captioned clips per hour. First-to-market in poker streaming.

The product already has:
- Multi-camera capture and composition
- CardEYE CV-based hole-card detection
- Live equity overlays (Monte Carlo — "AA 82% vs KK")
- Hand-history event bus
- Auto-clip generation
- Stripe-based license gate

What it doesn't have, and nobody in the poker streaming space has:
- **Real-time GTO strategy overlays** — "correct play is call 62%, raise
  18%, fold 20%" with the hero's range pie-chart

That's the moat this repo exists to build.

## The market reality (as of 2026-04-22)

From research we did on 2026-04-22:

**PokerGFX** — the incumbent. Basic $999/yr, Pro $3,999/yr, Enterprise
$9,999/yr. Ships graphics + hole cards + scorebug. **Ships no GTO.**

**Vizrt** — broadcast enterprise. Floor at $1,000/mo (Viz Trio Go),
"affordable" tier at $2,995/mo (Viz Vectar Plus). Zero poker-specific
products. Their sports AR/graphics stack assumes continuous playing fields
and moving broadcast cameras — not a felt with chips.

**GTO Wizard** — has the world's largest precomputed GTO database. Exclusive
partnership with GGPoker for GGMillion$ broadcast graphics (30-min delay).
Will not license to $60/mo competitors.

**TexasSolver** (open-source) — AGPL with author carve-out for binary
integration. We could license it (~$500–$5k one-time, email icybee@yeah.net).
That path was the recommended fallback.

**Deepsolver** — commercial HTTP API. $1,875–$6,750/mo base cost. ToS has a
"no live-table assistance" clause that creates broadcast-overlay legal risk.
At ~32 paying users to break even on the API alone, margins suck.

**postflop-solver / WASM-Postflop** — open-source Rust, AGPL with NO
dual-license path. Author went commercial and will not grant one. License-
infects Poker Panel's source if we link.

## Why we're building, not buying

Three reasons:

1. **Economics.** $60/mo × ~40 users = $2,400/mo, which is exactly Deepsolver's
   Production tier. Buying the compute eats the entire subscription. Owning
   it means each new user is pure margin.
2. **Legal certainty.** No ToS risk. No license-infection from AGPL. We own
   the code and the outputs, full stop.
3. **Product control.** Our bet-tree abstraction, our board-texture
   bucketing, our overlay-specific approximations. We tune the tradeoffs for
   "looks great on a broadcast" vs "wins a HUNL match" — those are different
   quality functions.

## Why poker is uniquely solvable on a MacBook

NLHE has ~10^17 decision nodes, which sounds impossible. But:

1. **Poker is a "solve a subgame" problem, not "solve the whole game."** On
   a broadcast, we always know the board, the action history, and the
   player ranges. That collapses the tree to a subgame with ~10^6–10^8
   nodes — very tractable.
2. **Card isomorphism collapses ~100×.** AsKs on JhTh4c is strategically
   identical to AhKh on JsTs4d. Exploit this.
3. **Bet-tree abstraction collapses ~1000×.** 3–5 discrete bet sizes
   (33%, 66%, pot, 2×, all-in) instead of every integer amount.
4. **Vector CFR on the river collapses ~50×.** At the river, every hand
   matchup is a showdown — the whole regret update is a 1326×1326 matrix
   op, which SIMD and Metal eat for breakfast.

Put together: full real-time GTO on a $1,500 MacBook is not only possible,
it's pleasant.

## Why 7 days is realistic

- The CFR algorithm itself is ~50–300 lines of Rust.
- Card/range/equity primitives are ~1,000 lines, well-trodden ground.
- Bet-tree abstraction is ~500 lines and there's public research on how to
  do it right.
- Swift FFI bindings are a standard `cbindgen` + SPM pattern.
- Convergence validation against TexasSolver is a weekend of shell scripting.
- Metal acceleration is optional for v0.1 — if Rust SIMD hits the river
  latency target, we ship with that and add Metal in v0.2.
- Flop precompute runs overnight on Colab while Henry sleeps.

The unknowns are:
- CFR convergence bugs (budget 1 day)
- Swift FFI threading weirdness (budget half a day)
- Bet-tree abstraction edge cases (budget 1 day)

Total slip budget: 2.5 days, which fits inside the 7-day sprint if the
critical path stays clean.

## The honest risks

1. **The river inner loop might not hit sub-300ms in pure Rust.** Mitigation:
   precompute river solutions too, turn the live solve into a cache lookup.
   Slower ship, but still shippable.
2. **CFR+ might have convergence issues on degenerate boards.** Mitigation:
   fall back to vanilla CFR, accept slower convergence, use MCCFR for turn.
3. **Flop precompute might take longer on Colab than estimated.** Mitigation:
   ship v0.1 with a small cache and let it grow over subsequent weeks.
4. **Swift FFI might require more work than expected.** Mitigation: the FFI
   surface is tiny (one function, one struct in, one struct out). This is
   not a big unknown.

None of these are existential. They slip the ship date by days, not weeks.
