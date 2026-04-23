# Flop cache

Binary packed flop-subgame strategies keyed by
`(canonical_board, spr_bucket, pot_type, bet_tree_version)`. Loaded at
runtime by `solver_nlhe::flop_cache::FlopCache::load_from_file`.

## Shipped artifact: `flop-cache-v0.1.bin`

**v0.1 is a format-only placeholder.** The committed file contains 36
hand-constructed entries — the smallest dataset that proves the binary
format + loader round-trips end-to-end so downstream consumers (Poker
Panel) can be wired and exercised in tests ahead of real data.

It is **NOT** the output of a real CFR+ solve. Do not use its
strategies for play, training, or research.

### Contents

| Axis              | Values                                                                 |
| ----------------- | ---------------------------------------------------------------------- |
| Canonical boards  | `AhKd2c`, `QsJd2c`, `Th7c2d`, `JhTh9c`, `9h8c7d`, `QhJhTs`, `8h8c3d`, `AhAc5d`, `KhKdKc`, `AhKhQh`, `7s6s5s`, `ThJhKh` |
| SPR buckets       | `{4, 8, 15}`                                                           |
| Pot types         | `Srp` only                                                             |
| Bet-tree version  | `1` (matches `FlopCache::lookup` default)                              |
| Actions per entry | `2` (check, pot-bet)                                                   |
| Total entries     | 12 × 3 × 1 = **36**                                                    |

Strategies are uniform across the 1326 combo axis per entry — the split
between check and bet is biased by a simple board-texture classifier
(dry-high → check-heavy, wet-mid → bet-heavy, etc.) and nudged by SPR.
Exploitability is set to `0.5` on every entry as a loud "synthetic data"
marker; a real CFR+ solve reaches < 1 mbb/hand.

### Regenerating

```bash
cargo run --release -p solver-cli -- seed-cache \
    --output data/flop-cache/flop-cache-v0.1.bin
```

The subcommand performs a round-trip load-verify after writing and
fails loudly if the on-disk bytes don't parse back into the same entry
count. The output is deterministic — running it twice produces
byte-identical files.

## v1.0 plan: Colab precompute

Real cache data ships from the Day-5 Colab precompute job
(`colab/precompute_flops.md`):

1. Colab solves each `(board, spr, pot, bet_tree)` cell with the live
   CFR+ pipeline and writes one JSON per cell to Google Drive.
2. `scripts/pull-colab-cache.sh` syncs Drive locally and invokes
   `solver-cli pack-cache` to serialize the JSONs into a single binary
   at `data/flop-cache/flop-cache-v1.0.bin`.
3. `flop-cache-v0.1.bin` is superseded; consumers flip to the v1.0
   filename.

## .gitignore policy

`data/flop-cache/*.bin` is ignored by default — real packed caches are
hundreds of MB and don't belong in git. The v0.1 seed is the one
explicit exception, whitelisted by name:

```gitignore
data/flop-cache/*.bin
!data/flop-cache/flop-cache-v0.1.bin
```

When v1.0 ships, add its name too (or drop the exception entirely if
v1.0 lives outside the repo).
