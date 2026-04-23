# Precompute preflop ranges (Colab notebook plan)

Target: one-time job, runs overnight Day 5. Output: `data/preflop-ranges/preflop-v0.1.bin`.

## Cell 1: setup

```python
!curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
import os; os.environ["PATH"] = f"{os.path.expanduser('~')}/.cargo/bin:{os.environ['PATH']}"
!rustc --version
```

## Cell 2: clone & build

```python
!git clone https://github.com/henryschlesinger/poker-solver.git /content/poker-solver
%cd /content/poker-solver
!cargo build --release -p solver-cli
```

## Cell 3: Drive mount

```python
from google.colab import drive
drive.mount('/content/drive')
!mkdir -p /content/drive/MyDrive/poker-solver/preflop-ranges
```

## Cell 4: define the grid

```python
import itertools, json
from pathlib import Path

# Positions (heads-up for v0.1)
POSITIONS = ["BTN_vs_BB", "BB_vs_BTN"]

# Stack depths in big blinds — cover the range tournaments actually use
STACK_DEPTHS = [10, 15, 20, 25, 30, 40, 50, 75, 100, 150, 200, 300, 500]

# Pot types
POT_TYPES = ["SRP", "3BP", "4BP", "5BP"]

grid = list(itertools.product(POSITIONS, STACK_DEPTHS, POT_TYPES))
print(f"{len(grid)} preflop spots to solve")
```

## Cell 5: parallel solve

```python
import subprocess, os
from concurrent.futures import ProcessPoolExecutor

OUTPUT_DIR = "/content/drive/MyDrive/poker-solver/preflop-ranges"

def solve_one(params):
    position, stack, pot_type = params
    key = f"{position}_{stack}bb_{pot_type}"
    out = f"{OUTPUT_DIR}/{key}.json"
    if os.path.exists(out):
        return (key, "cached")
    try:
        subprocess.run([
            "./target/release/solver-cli", "precompute",
            "--position", position,
            "--stack-bb", str(stack),
            "--pot-type", pot_type,
            "--iterations", "10000",
            "--output", out,
        ], check=True, timeout=3600)
        return (key, "done")
    except subprocess.TimeoutExpired:
        return (key, "timeout")

N_WORKERS = os.cpu_count() or 2
with ProcessPoolExecutor(max_workers=N_WORKERS) as pool:
    for result in pool.map(solve_one, grid):
        print(result)
```

## Cell 6: pack to binary

```python
!./target/release/solver-cli pack-preflop \
    --input /content/drive/MyDrive/poker-solver/preflop-ranges \
    --output /content/drive/MyDrive/poker-solver/preflop-v0.1.bin

!ls -lh /content/drive/MyDrive/poker-solver/preflop-v0.1.bin
```

## Download locally

```bash
# On Henry's Mac:
rclone copy gdrive:poker-solver/preflop-v0.1.bin \
    ~/Desktop/poker-solver/data/preflop-ranges/
```

## Expected runtime

~8 hours on a free-tier Colab session. ~2 hours on Pro.

## Expected output size

~100 MB. Shipped with the app.
