# Canonical NLHE solver fixtures

Twenty golden spots the solver must handle within tolerance vs TexasSolver.
Each fixture is a `spot_NNN.json` file whose schema is documented in
[SCHEMA.md](SCHEMA.md). The schema-validation test lives at
`crates/solver-cli/tests/fixtures_parse.rs` (owned by fixtures agent /
A15). The differential runner (actual solver + diff vs TexasSolver) is
Agent A14's responsibility and lives in the `solver-cli validate`
subcommand.

These fixtures are the basis for the "convergence validated against
TexasSolver on 20 canonical spots within 5% strategy delta" acceptance
criterion in [docs/ROADMAP.md](../../../../docs/ROADMAP.md) (Day 6 /
Day 7).

## Chip convention

All pots and stacks are expressed in chips at **1 BB = 10 chips**, to
match TexasSolver's default 5/10 blinds. A `pot: 60` fixture is a 6bb
pot; `effective_stack: 970` is a ~100bb remaining stack (post-flop
after a 3bb open + 3bb call). This keeps the chip numbers compatible
with the reference solver's UI without any conversion.

## Category coverage

The 20 spots intentionally span the strategically distinct board /
action-history axes the solver has to handle. Three fixtures per
category is not quite enough for "coverage" in a statistical sense, but
it is enough to surface qualitatively different bugs (e.g. a flush-draw
regret bug surfaces on wet boards; a paired-board eval bug surfaces on
paired boards; an SPR-handling bug surfaces on 3-bet pots and deep
stacks).

| Category | Count | Spots |
|---|---|---|
| Dry flops | 4 | `spot_001`..`spot_004` |
| Wet flops | 4 | `spot_005`..`spot_008` |
| Paired boards | 3 | `spot_009`..`spot_011` |
| 3-bet pots | 3 | `spot_012`..`spot_014` |
| River scenarios | 4 | `spot_015`..`spot_018` |
| Extreme SPR | 2 | `spot_019`..`spot_020` |

## The twenty

| ID | Street | Board | Pot | Eff. stack | Scenario |
|---|---|---|---|---|---|
| spot_001 | flop | AhKd2c | 60 | 970 | BB vs BTN SRP, broadway-heavy ranges, dry A-high |
| spot_002 | flop | Qs7d2h | 60 | 970 | BTN c-bet spot, polarized on low-connectivity board |
| spot_003 | flop | Ah5c2d | 60 | 970 | Ace-high dry with wheel cards (A5s / A4s value, low-card equity) |
| spot_004 | flop | 8h7c3d | 60 | 970 | Mid, dry-ish connected — low broadway whiff |
| spot_005 | flop | JhTh9c | 60 | 970 | Classic wet two-tone, every draw live |
| spot_006 | flop | 9h8h7c | 60 | 970 | Low connected two-tone; tons of straight / flush draws |
| spot_007 | flop | QhJhTs | 60 | 970 | Two-tone broadway with straight ladder up-and-down |
| spot_008 | flop | 7s6s5s | 60 | 970 | Monotone straight ladder — flush everywhere, straight draws |
| spot_009 | flop | 8h8c3d | 60 | 970 | Small paired board; pairs+kicker value, trips rare |
| spot_010 | flop | AhAc7d | 60 | 970 | Paired ace; strong-heavy ranges collapse on kicker |
| spot_011 | flop | KhKdKc | 60 | 970 | Trips on board; pure kicker / counterfeit equity |
| spot_012 | flop | AhKd2c | 180 | 910 | 3-bet pot, same board as spot_001; tighter ranges, lower SPR |
| spot_013 | flop | QhJhTs | 180 | 910 | 3-bet pot wet board; premiums vs draws |
| spot_014 | flop | 8h8c3d | 180 | 910 | 3-bet pot paired small board; narrow premiums range |
| spot_015 | river | AhKd2cQc4d | 100 | 950 | Dry runout; busted draws slim, value is top-pair+ |
| spot_016 | river | JhTh9c8h7c | 100 | 950 | Wet runout; made straights / flushes, busted combo draws |
| spot_017 | river | 9h8h7c6d5s | 100 | 950 | Straight on board; pot is a chop baseline, playing for kickers |
| spot_018 | river | KhKdKc2s4h | 100 | 950 | Quads possible, full houses common, kicker matters |
| spot_019 | flop | AhKd2c | 180 | 4910 | Deep-stack 3-bet pot (~500bb effective); high SPR stress |
| spot_020 | river | AhKd2cQc4d | 200 | 100 | SPR 0.5 all-in-jam shove-or-fold river |

Each row links to its corresponding `spot_NNN.json` file in this directory.

## How the test runner uses these

```
cargo test -p solver-cli --test fixtures_parse
```

Runs two tests:

1. `every_fixture_parses_and_validates` — loads each file, schema-checks
   it, parses each board card via `solver-eval::Card::parse`, and parses
   each range via `solver-nlhe::range::Range::parse`. This catches
   malformed JSON, unknown fields (via explicit whitelist), and bad
   range / card syntax.
2. `twenty_canonical_fixtures_exist` — asserts there are exactly 20
   fixtures with ids `spot_001`..`spot_020`. Guards against accidental
   deletion and against silent gaps (e.g. missing `spot_007`).

Both must pass for CI to green.

The Agent A14 runner (`solver-cli validate --spot <file>`) layers on top
of this: it re-parses each fixture, runs CFR+ to the iteration count
specified in the JSON, and diffs per-action frequency + EV against
TexasSolver's published output for the same spot. Diffs that exceed
`tolerances.action_freq_abs` / `tolerances.ev_bb_abs` fail the spot.

## Range-notation caveats

The parser supports `T9s+` (first-rank-iterates). For suited-aces /
suited-kings rolls where standard poker notation would use `A2s+` or
`K9s+` (second-rank-iterates), the fixtures in this directory enumerate
the combos explicitly (e.g. `A2s, A3s, A4s, A5s` instead of `A2s+`).
See [SCHEMA.md § Range notation caveats](SCHEMA.md#range-notation-caveats)
for the full explanation.

## Ranges used

All ranges are GTO-informed v0.1 approximations. They are **fixtures for
testing that the solver handles spots**, not for being optimal poker
strategy. Each fixture's `description` field calls out any notable
choices. The reference templates (BTN SRP open, BB defend, BTN 3-bet,
BB 3-bet) are in the task brief; the 3BP and river fixtures narrow them
further based on the action history assumed in that spot's `description`.
