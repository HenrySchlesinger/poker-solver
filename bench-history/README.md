# bench-history/

Dated snapshots of `cargo bench -p solver-core` output on Henry's M-series
MacBook. One JSON file per baseline run, named
`YYYY-MM-DD_HHMMSS_<sha7>.json`.

## Why this directory exists

Criterion stores its own baselines under `target/criterion/<bench>/<name>/`,
but those are per-workspace, not checked in, and get wiped by `cargo clean`.
When an optimization lands on `main` we lose the "what it was before"
reference unless we committed the numbers somewhere. This directory is
that somewhere.

These files are the **official record** that goes with any performance
claim in a commit message or PR description. If you cite a µs/ns number,
the closest JSON in here should back it up.

## File format

Each file is a flat JSON document:

```json
{
  "commit": "<full sha>",
  "commit_short": "<sha7>",
  "date_utc": "<iso8601>",
  "hardware": "M-series Mac (Henry's laptop)",
  "agent": "<agent id that captured the run, e.g. A55>",
  "rust_toolchain": "<rustc version>",
  "profile": "bench (optimized + debuginfo)",
  "notes": "<anything worth flagging — contention, anomalies, skips>",
  "benches": {
    "<group/param>": {
      "mean_ns": <criterion mean in nanoseconds>,
      "unit": "ns",
      "ci_low_ns":  <criterion lower bound>,
      "ci_high_ns": <criterion upper bound>,
      "mean_display": "<optional pretty string e.g. '1.7365 µs'>"
    }
  }
}
```

`mean_ns` is always nanoseconds so you can sort / diff numerically across
files without parsing `µs` / `ms` suffixes. `mean_display` preserves the
unit criterion actually printed.

## What gets captured

The canonical baseline set:

- `regret_matching_scalar/{3, 8, 26, 169, 1326}` — scalar inner-loop
  microbench at every N that shows up in practice.
- `cfr_plus_kuhn/{10, 100, 1000}` — full CFR+ solve on Kuhn Poker.
- `cfr_plus_kuhn_single_iteration` — one `iterate_from` call for
  inner-loop reasoning.

Not captured (on purpose, as of 2026-04-23):

- `benches/river.rs` — Kuhn-wrapped placeholder, redundant with
  `cfr_plus_kuhn/1000`. Will be captured once `solver-nlhe::NlheSubgame`
  lands and `SOLVER_RUN_RIVER_BENCH=1` unlocks the real river spots.
- `benches/simd_matching.rs`, `benches/metal_matching.rs`,
  `benches/flat_vs_hashmap.rs` — optimization-path benches that move
  independently of the scalar baseline. Captured only when the
  corresponding optimization lands on `main` and we want a dated record.

## How to add a new snapshot

```bash
cd ~/Desktop/poker-solver

# Make sure nothing else is churning the CPU / allocator.
pgrep -l cargo

# One bench invocation per file, sequential — don't parallelize.
cargo bench -p solver-core --bench regret_matching 2>&1 | tee /tmp/bench_regret.txt
cargo bench -p solver-core --bench cfr_kuhn        2>&1 | tee /tmp/bench_kuhn.txt

# Grab the criterion mean from each "time: [low mean high]" line, build
# the JSON by hand or with a tiny script, name it:
#   bench-history/$(date -u +"%Y-%m-%d_%H%M%S")_$(git rev-parse --short=7 HEAD).json
```

Guidelines:

1. **One clean run, not three averaged**. Criterion already averages 100
   samples per bench. If the run got contended (concurrent `cargo test`,
   fan spinning up, Xcode indexing) note it in `"notes"` rather than
   silently retrying.
2. **Keep it small**. Flat structure, no nesting. This directory should
   stay diffable 5 years from now.
3. **Don't delete old files**. Even a bad run with a known artifact is
   useful context for the next optimizer.
4. **Don't check in `target/criterion/`**. The HTML reports are huge and
   regenerated every run. The JSON here is the durable record.

## Seeing the trajectory

A quick "how has N=1326 moved over time" is:

```bash
jq -r '.date_utc + " " + (.benches["regret_matching_scalar/1326"].mean_ns|tostring) + " ns (" + .commit_short + ")"' \
   bench-history/*.json | sort
```

The doc-level baseline table in `docs/BENCHMARKS.md` reflects the most
recent durable numbers, but the JSON files are the append-only source of
truth.
