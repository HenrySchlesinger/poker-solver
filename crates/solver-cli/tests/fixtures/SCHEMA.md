# Fixture JSON schema

Each `spot_NNN.json` in this directory describes one canonical NLHE spot
the solver must handle correctly. The runner at
`crates/solver-cli/tests/fixtures_parse.rs` loads every file and checks
that it conforms to this schema. The Agent A14 differential runner
(`solver-cli validate --spot <file>`) consumes the same schema.

## Top-level fields

| Field | Type | Required | Description |
|---|---|---|---|
| `id` | string | yes | Stable fixture id, matching the filename stem (e.g. `"spot_001"`). Used as the primary key in test reports and by validation tooling. |
| `name` | string | yes | Short human-readable name, 1 line. |
| `description` | string | yes | 1–3 sentence description of the spot's strategic significance (dry flop, wet flop, paired, 3BP, etc.) — document any notation choices here (e.g. why we expanded `A2s+` manually). |
| `street` | string | yes | One of `"flop"`, `"turn"`, `"river"`. (Preflop is a static lookup; preflop spots don't live here in v0.1.) |
| `input` | object | yes | See [input](#input-fields). |
| `iterations` | integer | yes | How many CFR+ iterations the solver should run for this fixture. 1000 is the default for river, 500 for turn. |
| `tolerances` | object | yes | See [tolerances](#tolerances-fields). |
| `expected_reference` | string | yes | Which external solver we compare against. Currently always `"texassolver"`. |
| `expected_notes` | string | yes | Free-form notes about the golden output. Populated by the differential runner once a reference solve exists; may be `"populated by differential run"` as a placeholder. |

## input fields

| Field | Type | Required | Description |
|---|---|---|---|
| `board` | string | yes | Concatenated card string. 6 chars for flop (`"AhKd2c"`), 8 for turn (`"AhKd2cQc"`), 10 for river (`"AhKd2cQc4d"`). Parsed 2 chars at a time; see `solver-eval::card::Card::parse`. |
| `hero_range` | string | yes | Range string in `solver-nlhe::range::Range::parse` notation. See [Range notation caveats](#range-notation-caveats) below. |
| `villain_range` | string | yes | Range string, same notation as `hero_range`. |
| `pot` | integer | yes | Pot size in chips at the start of this street's action. Convention: 1 BB = 10 chips throughout these fixtures (matches TexasSolver's 5/10 default). So `pot: 60` = 6bb, `pot: 200` = 20bb. |
| `effective_stack` | integer | yes | Chips behind for each player. |
| `to_act` | string | yes | One of `"hero"`, `"villain"`. Who has the option on this street. |
| `bet_tree` | string | yes | Name of the bet-tree preset. Currently always `"default_v0_1"` — see `solver-nlhe::bet_tree::BetTree::default_v0_1`. |

## tolerances fields

| Field | Type | Required | Description |
|---|---|---|---|
| `action_freq_abs` | number | yes | Max absolute delta (in probability, 0..1) between our per-action frequency and the reference's, for any action at any info set. `0.05` = 5 percentage points. |
| `ev_bb_abs` | number | yes | Max absolute delta, in big blinds, between our per-action EV and the reference's. `0.10` = 0.1 bb. |

## Range notation caveats

The parser in `solver-nlhe::range::Range::parse` supports:

- `AA`, `KK` — pair
- `AKs`, `AKo`, `AK` — suited / offsuit / any
- `T9s+` — second rank fixed, first rank iterates up (so this means
  `T9s, J9s, Q9s, K9s, A9s`)
- `22+`, `JJ-`, `88-TT` — pair ranges
- `:weight` suffix — `AA:0.5` means weight 0.5 on every AA combo

**Intentional caveat:** `A2s+` in standard poker notation means
`A2s, A3s, A4s, A5s, …` (second rank iterates). The parser implements the
`X Y s+` rule as "first rank iterates"; since A is already the top rank,
`A2s+` would parse to only `A2s`. In these fixtures we write such ranges
out explicitly — e.g. `A2s, A3s, A4s, A5s` — rather than relying on the
notation. Each fixture's `description` field calls this out when it
matters.

## Full example

```json
{
  "id": "spot_001",
  "name": "Dry AK-high flop, BB vs BTN SRP",
  "description": "Straightforward single-raised pot, dry board, classic c-bet study spot. Narrow ranges (broadway-heavy) to make the output easy to eyeball against TexasSolver.",
  "street": "flop",
  "input": {
    "board": "AhKd2c",
    "hero_range": "AA, KK, AKs, AKo, QQ, JJ",
    "villain_range": "77, 88, 99, TT, AQs, AQo, AJs, KQs",
    "pot": 60,
    "effective_stack": 970,
    "to_act": "hero",
    "bet_tree": "default_v0_1"
  },
  "iterations": 1000,
  "tolerances": {
    "action_freq_abs": 0.05,
    "ev_bb_abs": 0.10
  },
  "expected_reference": "texassolver",
  "expected_notes": "Populated by differential run; this is the golden slot."
}
```

## Parse-test contract

`crates/solver-cli/tests/fixtures_parse.rs` asserts:

1. Every `spot_NNN.json` file parses as JSON.
2. Every file matches the `Fixture` struct shape (all required fields present, correct types).
3. `id` matches the filename stem (guard against copy-paste mistakes).
4. `street` is one of `"flop"`, `"turn"`, `"river"`.
5. `to_act` is one of `"hero"`, `"villain"`.
6. `board` has the correct char length for the street: 6 for flop, 8 for turn, 10 for river.
7. Every board card parses as a valid `solver-eval::card::Card`.
8. `hero_range` and `villain_range` parse via `solver-nlhe::range::Range::parse` without error.
9. `pot > 0` and `effective_stack > 0`.
10. `iterations > 0`.

The parse test is **only** schema validation — it does not call the solver.
The actual convergence comparison (against TexasSolver) is Agent A14's
runner, not this test.

## File naming

`spot_NNN.json` where `NNN` is a zero-padded 3-digit integer. `id` inside
the file must match `spot_NNN`. See the 20-spot table in `README.md` for
the full list.
