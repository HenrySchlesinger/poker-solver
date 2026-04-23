# Convergence validation (Colab notebook plan)

Target: nightly validation that our solver matches TexasSolver on
canonical spots. Catches regressions the unit tests miss.

Runs Day 6 onwards. Output: a JSON summary in Drive that Henry can skim.

## Cell 1: setup (same pattern)

```python
!curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
import os; os.environ["PATH"] = f"{os.path.expanduser('~')}/.cargo/bin:{os.environ['PATH']}"
!git clone https://github.com/henryschlesinger/poker-solver.git /content/poker-solver
%cd /content/poker-solver
!cargo build --release --workspace
```

## Cell 2: install TexasSolver (console build)

```bash
%%bash
# TexasSolver console build for reference outputs. Licensed separately;
# do not ship this binary.
apt-get install -y cmake g++
git clone https://github.com/bupticybee/TexasSolver.git /tmp/texassolver
cd /tmp/texassolver
mkdir build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release -DCONSOLE=ON
make -j$(nproc)
cp console_solver /usr/local/bin/texassolver
```

## Cell 3: load fixtures

```python
import json
from pathlib import Path

FIXTURES = list(Path("/content/poker-solver/crates/solver-cli/tests/fixtures").glob("*.json"))
print(f"{len(FIXTURES)} canonical spots to validate")
```

## Cell 4: run both solvers, diff

```python
import subprocess, json

def run_ours(fixture):
    out = subprocess.run(
        ["./target/release/solver-cli", "solve", "--input", str(fixture), "--json"],
        capture_output=True, text=True, check=True
    )
    return json.loads(out.stdout)

def run_theirs(fixture):
    # Convert our fixture format to TexasSolver config.
    # See solver-cli/tests/translate_fixture.py
    out = subprocess.run(
        ["texassolver", "--config", str(fixture.with_suffix(".tsconfig"))],
        capture_output=True, text=True, check=True
    )
    return json.loads(out.stdout)

def diff(ours, theirs):
    # Compare action_frequencies, ev_per_action within tolerance.
    freq_delta = max(
        abs(ours["action_freq"][a] - theirs["action_freq"][a])
        for a in ours["action_freq"]
    )
    ev_delta = max(
        abs(ours["action_ev"][a] - theirs["action_ev"][a])
        for a in ours["action_ev"]
    )
    return {"freq_delta": freq_delta, "ev_delta": ev_delta,
            "pass": freq_delta < 0.05 and ev_delta < 0.1}

results = {}
for fixture in FIXTURES:
    ours = run_ours(fixture)
    theirs = run_theirs(fixture)
    results[fixture.name] = diff(ours, theirs)
    print(fixture.name, results[fixture.name])
```

## Cell 5: summarize

```python
import json, datetime

summary = {
    "date": datetime.datetime.utcnow().isoformat(),
    "git_sha": subprocess.run(["git", "rev-parse", "HEAD"],
                              capture_output=True, text=True,
                              cwd="/content/poker-solver").stdout.strip(),
    "total_spots": len(results),
    "passed": sum(1 for r in results.values() if r["pass"]),
    "failed": sum(1 for r in results.values() if not r["pass"]),
    "details": results,
}

with open("/content/drive/MyDrive/poker-solver/convergence-latest.json", "w") as f:
    json.dump(summary, f, indent=2)

print(f"PASS: {summary['passed']}/{summary['total_spots']}")
if summary["failed"]:
    for name, r in results.items():
        if not r["pass"]:
            print(f"  FAIL {name}: {r}")
```

## Expected runtime

~1 hour for 20 spots. Grows linearly with fixture count.

## When this catches bugs

- CFR+ regret update math error → freq_delta huge
- Bet-tree discretization mismatch → specific actions missing
- Card isomorphism bug → specific boards fail, others pass
- Iteration count too low → marginal freq_delta, EV mostly OK

Each failure mode has a signature in the output JSON.
