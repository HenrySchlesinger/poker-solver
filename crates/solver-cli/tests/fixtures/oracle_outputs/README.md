# Oracle outputs for differential testing

This directory ships **reference outputs** for the canonical spots in
`crates/solver-cli/tests/fixtures/spot_*.json`. Each fixture has up to
three files here:

| Suffix                      | What it is                                                      | Who produced it             |
|-----------------------------|-----------------------------------------------------------------|-----------------------------|
| `spot_NNN.tsconfig`         | TexasSolver config translated from the fixture JSON             | `solver-cli translate-fixture` |
| `spot_NNN.texassolver.json` | TexasSolver's full strategy tree dump (the **oracle**)          | `./bin/texassolver`         |
| `spot_NNN.ts-log.txt`       | TexasSolver's stdout log (iter count + exploitability log lines) | `./bin/texassolver`         |
| `spot_NNN.our.json`         | Our `solver-cli solve` output (aggregate action freqs + EVs)    | `./target/release/solver-cli solve` |

## Why ship these as test data

The auto-diff harness (`crates/solver-cli/tests/texassolver_diff.rs`,
A14/A47) needs something to compare against. TexasSolver is expensive to
run (requires cmake, libomp, an OpenMP-enabled C++ build). Shipping its
outputs alongside the fixtures lets:

1. The diff harness run in CI **without** building TexasSolver on every
   runner.
2. Future agents audit a regression without needing to re-run the oracle.
3. A diff against a known-good baseline to catch drift in our own solver.

## How these were generated

One-shot end-to-end per fixture, using the verified flow from A50:

```bash
cd ~/Desktop/poker-solver
mkdir -p target/tsconfig
F=crates/solver-cli/tests/fixtures/spot_016.json
./target/release/solver-cli translate-fixture \
    --input "$F" \
    --output target/tsconfig/spot_016.tsconfig \
    --dump-path target/tsconfig/spot_016.result.json
./bin/texassolver --resource_dir bin/resources -i target/tsconfig/spot_016.tsconfig
./target/release/solver-cli solve \
    --board "$(jq -r .input.board $F)" \
    --hero-range "$(jq -r .input.hero_range $F)" \
    --villain-range "$(jq -r .input.villain_range $F)" \
    --pot "$(jq -r .input.pot $F)" \
    --stack "$(jq -r .input.effective_stack $F)" \
    --iterations "$(jq -r .iterations $F)"
```

## Current coverage (2026-04-23)

Five river fixtures validated end-to-end. A14/A50 ran `spot_015`; A63
extended to `spot_016`, `spot_017`, `spot_018`, `spot_020`.

| Fixture    | Street | Board               | TS iters / time | Our compute_ms |
|------------|--------|---------------------|-----------------|----------------|
| `spot_015` | river  | `AhKd2cQc4d` dry    | 41 / 11 ms      | 7 354          |
| `spot_016` | river  | `JhTh9c8h7c` wet    | 171 / 116 ms    | 79 517         |
| `spot_017` | river  | `9h8h7c6d5s` board-straight | 151 / 147 ms | 208 100 |
| `spot_018` | river  | `KhKdKc2s4h` quads  | 271 / 133 ms    | 31 537         |
| `spot_020` | river  | `AhKd2cQc4d` SPR 0.5 | 21 / ~0 ms     | 4 029          |

TexasSolver converged to the fixture's configured `set_accuracy 0.3`
threshold in every case well under the 1000-iter cap. See
`docs/DIFFERENTIAL_TESTING.md` § Status for the full status including
asymmetries that still block the frequency diff.

## Known asymmetries (not yet reconciled by the diff harness)

See the **Asymmetry: our JSON vs TexasSolver JSON** section in
`docs/DIFFERENTIAL_TESTING.md`. Short version:

1. **Granularity.** TexasSolver dump is a full per-node-per-combo-per-action
   strategy tree; our output is aggregate-per-root-action.
2. **Bet-size naming.** We emit `"bet_33"`; TexasSolver emits
   `"BET 19.800000"` (chip count).
3. **EV.** We include `ev_per_action` in JSON; TexasSolver prints
   `player N exploitability M` only in the log.

These files are the *inputs* to a future comparator. The comparator is
A47's follow-up job.
