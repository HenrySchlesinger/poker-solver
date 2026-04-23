# Precompute flop cache (Colab notebook plan)

Target: the big overnight job. Runs Day 5 onwards across many sessions.
Output: ~5000 JSON files in Drive, later packed into
`data/flop-cache/flop-cache-v0.1.bin`.

## Cell 1: setup (same as preflop)

```python
!curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
import os; os.environ["PATH"] = f"{os.path.expanduser('~')}/.cargo/bin:{os.environ['PATH']}"
!git clone https://github.com/henryschlesinger/poker-solver.git /content/poker-solver
%cd /content/poker-solver
!cargo build --release -p solver-cli

from google.colab import drive
drive.mount('/content/drive')
!mkdir -p /content/drive/MyDrive/poker-solver/flop-cache
```

## Cell 2: shard assignment

Multiple Colab sessions run in parallel. Each claims one shard.

```python
# SET THIS PER SESSION. Values 0..N_SHARDS-1.
SHARD_ID = 0
N_SHARDS = 4
```

## Cell 3: define the grid

```python
import itertools, hashlib, json
from pathlib import Path

# Canonical flops: ~1,755 suit-isomorphic distinct flops
# Load the enumeration from the solver-eval crate (generated on build).
with open("/content/poker-solver/data/iso-tables/canonical_flops.json") as f:
    CANONICAL_FLOPS = json.load(f)  # list of strings like "AhKhQh"

SPR_BUCKETS = [1, 2, 4, 8, 15, 30]
POT_TYPES = ["SRP", "3BP"]  # "4BP" added later if time
BET_TREES = ["default_3"]

grid = list(itertools.product(CANONICAL_FLOPS, SPR_BUCKETS, POT_TYPES, BET_TREES))

# Shard: keep only items whose hash % N == SHARD_ID
def in_shard(item):
    key = "_".join(str(x) for x in item)
    h = int(hashlib.sha1(key.encode()).hexdigest(), 16)
    return h % N_SHARDS == SHARD_ID

grid = [g for g in grid if in_shard(g)]
print(f"This shard ({SHARD_ID}/{N_SHARDS}): {len(grid)} spots")
```

## Cell 4: solve (with resume)

```python
import subprocess, os
from concurrent.futures import ProcessPoolExecutor

OUTPUT_DIR = "/content/drive/MyDrive/poker-solver/flop-cache"

def solve_one(params):
    board, spr, pot_type, bet_tree = params
    key = f"{board}_{spr}spr_{pot_type}_{bet_tree}"
    out = f"{OUTPUT_DIR}/{key}.json"

    if os.path.exists(out):
        return (key, "cached")

    try:
        result = subprocess.run([
            "./target/release/solver-cli", "precompute",
            "--board", board,
            "--spr", str(spr),
            "--pot-type", pot_type,
            "--bet-tree", bet_tree,
            "--iterations", "5000",
            "--output", out,
        ], check=True, capture_output=True, timeout=1200)
        return (key, "done")
    except subprocess.TimeoutExpired:
        return (key, "timeout")
    except subprocess.CalledProcessError as e:
        return (key, f"error: {e.returncode}")

with ProcessPoolExecutor(max_workers=os.cpu_count() or 2) as pool:
    done = 0
    for result in pool.map(solve_one, grid):
        done += 1
        if done % 10 == 0:
            print(f"[{done}/{len(grid)}] {result}")
```

## Cell 5: status

```python
!ls /content/drive/MyDrive/poker-solver/flop-cache | wc -l
!du -sh /content/drive/MyDrive/poker-solver/flop-cache
```

## Coordination across shards

Open 4 Colab tabs. In each, set `SHARD_ID` to 0, 1, 2, 3 before running
Cell 2. Each session processes its own slice, writes to the same Drive
folder. `solve_one` skips already-complete files, so if a session drops
partway through, restarting resumes cleanly.

## Expected runtime

- Per flop, SPR, pot-type, bet-tree: 2–10 minutes (avg ~5)
- Total grid: ~21,000 spots
- Per shard: ~5,250 spots × 5 min / 2 cores = ~218 hours per shard
- 4 shards in parallel: ~54 wall-clock hours

Call it two nights of multi-session work, or a week of nightly sessions.

## Expected output size

- Per JSON: ~50 KB (strategies + metadata)
- Total: ~1 GB raw JSON
- Packed binary: ~400 MB (via `solver-cli pack-cache`)

Ship a ~200 MB subset; host the rest on CDN for first-run download.
