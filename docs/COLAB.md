# Colab: the offline precompute pipeline

## What Colab is for

**Offline precompute, not runtime.** Colab runs overnight (or across
multiple daytime sessions) to generate data files that ship with Poker
Panel. The solver itself never talks to Colab, never calls out to cloud
compute at runtime, never requires an internet connection to work.

## What runs on Colab

1. **Flop cache population.** The big one. Solve thousands of flop
   subgames offline, pack results into a binary lookup file. Each flop
   takes 2–10 minutes of single-core compute; we run many in parallel.
2. **Preflop range generation.** One-time job. Solve standard preflop
   scenarios at various stack depths (10bb, 15bb, 20bb, 25bb, ... 200bb,
   500bb) and positions. Output: ~100 MB static range database.
3. **Convergence validation.** Run our solver and TexasSolver on the same
   canonical spots, diff the outputs, flag any regressions. This is a
   nightly CI-like job.
4. **Benchmark tracking** (optional). Run the criterion suite on a fixed
   Colab VM for stable-hardware baselines across Rust versions.

## What does NOT run on Colab

- The live solver (obviously). Runs on the user's Mac via FFI.
- Any training, ML, or neural networks. We're doing classical CFR, not
  neural CFR.
- Anything that reads or writes user data.

## Setup

Colab notebooks live in `colab/` and are checked into git as Markdown
plans. Convert each to an `.ipynb` at runtime (or use `jupytext`).

### One-time setup

```python
# Cell 1: install Rust on the Colab VM
!curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
!source $HOME/.cargo/env

# Cell 2: clone the repo (read-only; precompute doesn't commit back)
!git clone https://github.com/henryschlesinger/poker-solver.git  # TBD repo
%cd poker-solver

# Cell 3: build the solver in release mode
!cargo build --release -p solver-cli

# Cell 4: mount Google Drive for output persistence
from google.colab import drive
drive.mount('/content/drive')
!mkdir -p /content/drive/MyDrive/poker-solver/flop-cache
```

### Recurring precompute (the overnight job)

```python
# Cell 5: generate the grid of flops to solve
import itertools, json
from pathlib import Path

BOARD_TEXTURES = [...]  # enumerated in solver-eval::texture
SPR_BUCKETS    = [1, 3, 6, 10, 20, 50]
POT_TYPES      = ["SRP", "3BP", "4BP"]  # single-raised, 3-bet, 4-bet
BET_TREES      = ["default_3", "default_5"]

grid = list(itertools.product(
    BOARD_TEXTURES, SPR_BUCKETS, POT_TYPES, BET_TREES
))

# Cell 6: parallel solve via multiprocessing
import subprocess, os
from concurrent.futures import ProcessPoolExecutor

def solve_one(params):
    board, spr, pot_type, bet_tree = params
    out = f"/content/drive/MyDrive/poker-solver/flop-cache/" \
          f"{board}_{spr}_{pot_type}_{bet_tree}.bin"
    if Path(out).exists():
        return (params, "cached")
    subprocess.run([
        "./target/release/solver-cli", "precompute",
        "--board", board,
        "--spr", str(spr),
        "--pot-type", pot_type,
        "--bet-tree", bet_tree,
        "--iterations", "5000",
        "--output", out,
    ], check=True)
    return (params, "done")

with ProcessPoolExecutor(max_workers=os.cpu_count()) as pool:
    for result in pool.map(solve_one, grid):
        print(result)
```

### Download and pack

Back on Henry's Mac:

```bash
# Pull from Drive (rclone or manual download)
rclone copy gdrive:poker-solver/flop-cache/ ~/Desktop/poker-solver/data/flop-cache/

# Pack into single binary for shipping
cargo run -p solver-cli --release -- pack-cache \
    --input data/flop-cache \
    --output data/flop-cache/flop-cache-v0.1.bin
```

## Parallelism strategy

Colab free tier gives ~2 CPU cores per session. To go fast we use
**multiple free-tier sessions in parallel**:

- Open 4 browser tabs, each runs a Colab free-tier session
- Each session processes a non-overlapping slice of the grid (use a hash
  modulo N assignment to partition)
- All sessions write to the same Drive folder
- Dedupe on the Mac side when packing

With 4 sessions × 2 cores each = 8 parallel solves. A grid of 5000 flop
spots at 5 min each = 5000 × 5 / 8 = ~52 hours of wall clock. Two nights
on free tier.

**Free tier only.** Henry's rule: no paid services. If a session hits the
12-hour cap mid-batch, the resumable logic (skip already-complete output
files) picks up on a fresh session. If free tier is ever insufficient,
fallback is an overnight run on Henry's Mac — same Rust binary either
way. No paid upgrade.

## When to run what

| Job | Cadence | Duration |
|---|---|---|
| First flop-cache population | Starts Day 5, runs ~2 weeks | 50–100 hours total |
| Preflop range generation | One-time Day 5 overnight | ~8 hours |
| Convergence validation | Nightly, starting Day 6 | ~1 hour |
| Benchmark regression check | Per-PR, starting post-v0.1 | ~10 min |

## Output format

All Colab jobs emit JSON for debugging and binary for shipping:

```json
{
  "board": "canonical:AhKhQh",
  "spr_bucket": 6,
  "pot_type": "SRP",
  "bet_tree": "default_3",
  "solver_version": "0.1.0",
  "iterations": 5000,
  "exploitability": 0.0034,
  "strategies": {
    "root_check": {
      "check":   [0.73, 0.68, ...],  // 1326 combo-wise frequencies
      "bet_33":  [0.15, 0.21, ...],
      "bet_66":  [0.09, 0.08, ...],
      "bet_pot": [0.03, 0.03, ...]
    },
    ...
  }
}
```

Binary format is the same data packed with `bytemuck`. See
`crates/solver-nlhe/src/cache.rs`.

## Costs

| Item | Cost |
|---|---|
| Colab free tier | $0 |
| Colab Pro (optional, for throughput on overnight jobs) | $10/mo |
| Google Drive storage | Free up to 15 GB; we'll use < 5 GB |

**Default: free tier.** Colab Pro is OK if Henry genuinely needs the
throughput — the rule is "don't pay for stuff our product REPLACES"
(solvers, GTO APIs), not "never spend a dollar on compute." Pick free
tier first and only upgrade if free runs are blocking progress.

## What to NOT do

- Do not call out to Colab or any cloud service from the runtime solver.
  This is a hard architectural rule — violating it breaks the "local only"
  design contract.
- Do not store any user data on Colab. Precompute jobs are solving
  *hypothetical* canonical poker spots, not real users' hand histories.
- Do not leave precompute jobs running on Henry's personal Google account
  after the sprint. Either set up a dedicated Google account for the
  project or time-box the jobs carefully.
