# Differential testing vs TexasSolver

We validate our solver against
[TexasSolver](https://github.com/bupticybee/TexasSolver) by solving the
same canonical spots through both solvers and diffing the outputs. If
our solver and TexasSolver disagree by more than a small tolerance, that
is either a bug in our CFR+ implementation, a bet-tree mismatch, or a
range-parser divergence — all three are things we want to catch early.

This doc covers:

- [Status (what is verified, 2026-04-23)](#status-what-is-verified-2026-04-23)
- [Why TexasSolver](#why-texassolver)
- [Acceptance tolerances](#acceptance-tolerances)
- [Workflow](#workflow)
- [The file layout](#the-file-layout)
- [License — why this is legal](#license--why-this-is-legal)
- [Nightly validation (Day 7+)](#nightly-validation-day-7)
- [Known limitations](#known-limitations)

## Status (what is verified, 2026-04-23)

Agent A50 end-to-end verification on Henry's Mac (Apple Silicon,
macOS 26.2, `arm64`):

- **TexasSolver binary built and runs.** `./bin/texassolver` is an
  `arm64` Mach-O compiled via `scripts/install-texassolver.sh`. The
  vendored source pin is `bupticybee/TexasSolver` commit
  `8d7c4bf` (2021-11-29) on the `console` branch.
- **Runtime usage**. The binary takes:

  ```bash
  ./bin/texassolver --resource_dir bin/resources \
                    --input_file <path/to/*.tsconfig>
  ```

  The `--resource_dir` flag is **required** unless `cwd` is the
  directory that contains `resources/`. When missing, TexasSolver
  defaults to `./resources` and segfaults the moment the first
  iteration touches the hand-evaluator tables. (We've been bitten
  by this — it reports "Iter: 0" and then dies with SIGSEGV =
  exit 139.) The diff harness should always pass
  `--resource_dir bin/resources` explicitly.
- **Six fixtures successfully translate + solve** (A50 seeded `spot_015`
  + `spot_001`; A63 extended to `spot_016`–`spot_018` and `spot_020`
  on 2026-04-23):

  | Fixture     | Street | Translate | TexasSolver solve                                                       | TS dump size     | Our compute_ms | Auto-diff ready? |
  |-------------|--------|-----------|-------------------------------------------------------------------------|------------------|----------------|------------------|
  | `spot_015`  | river  | OK        | OK, converges to 0.25% exploitability in ~11 ms (41 iter, acc=0.3)      | 214 KB           | 7 354          | No (combo rollup + bet-size map) |
  | `spot_016`  | river  | OK        | OK, converges to 0.27% exploitability in ~116 ms (171 iter, acc=0.3)    | 1.0 MB           | 79 517         | No (combo rollup + bet-size map) |
  | `spot_017`  | river  | OK        | OK, converges to 0.29% exploitability in ~147 ms (151 iter, acc=0.3)    | 1.7 MB           | 208 100        | No (combo rollup + bet-size map) |
  | `spot_018`  | river  | OK        | OK, converges to 0.28% exploitability in ~133 ms (271 iter, acc=0.3)    | 667 KB           | 31 537         | No (combo rollup + bet-size map) |
  | `spot_020`  | river  | OK        | OK, converges to 0.15% exploitability in <1 ms (21 iter, acc=0.3)       | 5.9 KB           | 4 029          | No (combo rollup + bet-size map) |
  | `spot_001`  | flop   | OK        | OK, runs to completion; 20-iter smoke at 110 s wall (deep stack → deep tree), 86% exploitability after 20 iters. 1000-iter depth needs more compute than Henry's Mac supplies in a test window. | 24 MB (20 iter)  | n/a (not captured) | No (flop solve time prohibitive on laptop) |

  The five river fixtures all hit the `set_accuracy 0.3` convergence
  threshold in well under the 1000-iteration cap — TexasSolver river
  solves are fast on a laptop. `spot_001` is the lone flop spot and
  needs Colab-scale compute for full 1000-iter parity.

  Translated configs and the full result JSONs land under
  `target/tsconfig/` (gitignored, regenerated per run). The committed
  oracle outputs for the river fixtures live at
  `crates/solver-cli/tests/fixtures/oracle_outputs/` — see that
  directory's `README.md` for file layout.
- **Our-side outputs captured for all five river fixtures** under
  `crates/solver-cli/tests/fixtures/oracle_outputs/spot_NNN.our.json`
  (~700 B–1.0 KB each: aggregate action frequencies + EVs, which is
  the summary format `solver-cli solve` emits today).
- **Auto-diff status**: zero fixtures green yet, because all of them are
  blocked on the same three asymmetries (rollup, bet-size name map,
  log-line EV parse) below. Five river fixtures have the *data* in
  place; the blocker is the comparator, not the oracle.

### Translation-format quirks (learned from TexasSolver source)

Two parser quirks took a few iterations to track down. Both are
documented in detail in `crates/solver-cli/src/translate.rs`:

1. **Two-token cap per line.**
   `vendor/TexasSolver/src/tools/CommandLineTool.cpp::processCommand`
   splits each line on a single space and rejects anything with more
   than two tokens with `command not valid: ...`.

   Consequences for the translator:
   - **No comment lines.** An emitted `# auto-generated by ...`
     header crashes the binary. Fixture identity is now preserved
     only via the output filename (`spot_001.tsconfig`) and the
     dumped result JSON.
   - **No whitespace in range strings.** `"AA, KK, AKs"` has three
     tokens → crash. The translator strips every whitespace char
     before emitting ranges (TexasSolver's own sample input uses the
     `"AA,KK,AKs"` no-space form).
2. **Length-2-or-3 tokens only in ranges.**
   `vendor/TexasSolver/src/tools/PrivateRangeConverter.cpp::
   rangeStr2Cards` only understands tokens of length 2 (`"AA"`,
   `"AK"`) or 3 (`"AKs"`, `"AKo"`). It throws
   `range str ... len not valid` on every compact form our fixtures
   use. The translator now expands these to explicit comma-lists
   before emitting:

   | Our token | Expanded                                                |
   |-----------|---------------------------------------------------------|
   | `77-TT`   | `77,88,99,TT`                                           |
   | `22+`     | `22,33,...,AA`                                          |
   | `JJ-`     | `22,33,...,JJ`                                          |
   | `T9s+`    | `T9s,J9s,Q9s,K9s,A9s`                                   |
   | `KTo+`    | `KTo,ATo` (skips `TTo` which would be nonsensical)      |

   `:weight` suffixes pass through to every sub-token of an
   expansion (e.g. `77-99:0.5` → `77:0.5,88:0.5,99:0.5`).

### Asymmetry: our JSON vs TexasSolver JSON

The two output formats are NOT structurally comparable yet. The
diff harness owned by A14 / A47 still needs to reconcile them.

| | `solver-cli solve` (our side)                   | TexasSolver `dump_result`             |
|-|--|--|
| Granularity | one *aggregate* frequency & EV per root action        | full strategy tree per node, per hand, per action |
| Size (spot_015, river) | 882 B                                        | 214 KB                                |
| EV per combo | no (ev_per_action is aggregate only)                  | no (only log-line exploitability)    |
| Root actions | identified by name (`"bet_66"`, `"allin"`)           | identified by chip count (`"BET 66.000000"`) |
| Combo format | suit-pair (`"AhKh"`)                                 | suit-pair with color letters (`"AhKh"`) — same, modulo casing |

**What's needed for A47 to close the loop:**

1. **Output-shape reducer.** TexasSolver's per-hand-per-action
   strategy → a single `{action: frequency}` map, averaging over
   all combos in the root range (weight-weighted once fractional
   weights exist). The `texassolver_diff.rs` runner has a `rollup`
   function stub for this — see the "Combo rollup" note under
   [Known limitations](#known-limitations).
2. **Bet-size naming.** Our root-action map keys on "bet_33"
   (percent-of-pot); TexasSolver keys on "BET 19.800000" (chip
   count, float-formatted). The comparator needs to map one to the
   other via the pot size.
3. **Oracle log parsing for EV.** TexasSolver prints
   `player 0 exploitability <x>` but does not emit per-action EV in
   its JSON dump. Until we add log-line parsing, only frequency
   gets a two-sided diff. Flagged by
   [Known limitations > EV comparison is one-sided](#known-limitations).

## Why TexasSolver

TexasSolver is an open-source, free heads-up Hold'em solver that has
been publicly validated against PioSolver (the commercial reference) and
tracks it to within 0.3% on the benchmark flop spots. That makes it a
good oracle for us: we are NOT shipping TexasSolver, we are NOT modifying
its code, we are NOT calling into its library — we are running it as a
command-line binary and comparing the JSON output to our own.

Two alternatives considered:
- **PioSolver:** commercial, paid, Windows-only. Would require a Windows
  test VM. Rejected for sprint velocity.
- **MonkerSolver / Jesolver:** commercial, paid. Same reasons.
- **Writing our own "reference"** CFR+ in Python: defeats the purpose
  (oracles must be independently-authored).

## Acceptance tolerances

A fixture passes the differential test if, for every action at the root
info set:

| Metric | Tolerance | Rationale |
|---|---|---|
| Per-action frequency | ≤ 5 percentage points (absolute) | Empirically, CFR+ converges to within 2-3% at ~1000 iterations; 5% leaves headroom for sampling noise and iteration-count differences. |
| Per-action EV | ≤ 0.1 bb | Within the noise band of rounding + float ops. Tighter would flag legitimate iteration differences; looser would miss a real bug. |

These are the defaults. Each fixture carries its own `tolerances` block
(see `crates/solver-cli/tests/fixtures/SCHEMA.md`) so a particularly
stable spot can tighten them, and a particularly MCCFR-noisy turn spot
can loosen.

The 5%/0.1bb numbers are also what the
[sprint roadmap](ROADMAP.md#day-6--2026-04-27-sunday) codifies as the v0.1 ship gate.

## Workflow

### One-time setup

```bash
# On Henry's Mac:
brew install cmake libomp
./scripts/install-texassolver.sh
./bin/texassolver --help   # smoke check; prints "Usage: ..."
```

This clones <https://github.com/bupticybee/TexasSolver>, builds
`console_solver`, and drops the binary at `./bin/texassolver` alongside
its `resources/` runtime directory. The TexasSolver source tree lives
at `vendor/TexasSolver/` and is **not** committed (see `.gitignore`).

A manual end-to-end sanity check without the diff harness:

```bash
cargo build --release -p solver-cli
mkdir -p target/tsconfig
./target/release/solver-cli translate-fixture \
    --input crates/solver-cli/tests/fixtures/spot_015.json \
    --output target/tsconfig/spot_015.tsconfig \
    --dump-path target/tsconfig/spot_015.result.json
./bin/texassolver --resource_dir bin/resources \
                  --input_file target/tsconfig/spot_015.tsconfig
ls -l target/tsconfig/spot_015.result.json
```

spot_015 is a river spot and solves in tens of ms. Flop spots take
minutes (deep stack → deep tree).

### Running one spot

```bash
cargo test -p solver-cli --test texassolver_diff -- --ignored
```

The `--ignored` flag is required: the differential test is marked
`#[ignore]` so CI does not try to build TexasSolver (which would require
`libomp` and cmake on every CI runner — defeats the point of cheap CI).

The `-- --nocapture` flag prints both solvers' progress to stdout if
you want to watch the solve happen.

### Debugging a divergence

1. Read the failure output; it names the fixture and the offending
   action (e.g. `"bet_66": frequency delta 0.12 > tolerance 0.05`).
2. The intermediate files are preserved:
   - `<CARGO_TARGET_TMPDIR>/texassolver_diff_*/spot_NNN/input.tsconfig`
     — the exact TexasSolver config we fed in.
   - `<CARGO_TARGET_TMPDIR>/texassolver_diff_*/spot_NNN/ts_out.json`
     — TexasSolver's full strategy JSON.
3. Re-run TexasSolver manually with that config and poke at its output:
   ```bash
   cd bin && ./texassolver -i /path/to/spot_NNN/input.tsconfig \
       --resource_dir ./resources
   ```
4. Compare to our solver's JSON:
   ```bash
   cargo run -p solver-cli -- solve \
       --board AhKd2c \
       --hero-range "AA, KK, AKs" \
       --villain-range "QQ, AQs" \
       --pot 60 --stack 970 --iterations 1000 \
       --bet-tree default_v0_1
   ```
5. The usual culprits:
   - **Bet-tree mismatch.** Our `default_v0_1` preset emits a specific
     bet-size list per street; `scripts/translate_fixture.py`
     `BET_TREE_PRESETS` is the source of truth for what TexasSolver
     sees. If our solver's `BetTree::default_v0_1` has been edited, the
     translator's table needs to move with it.
   - **Range parser divergence.** TexasSolver and our
     `solver-nlhe::range::Range::parse` both claim PokerStove/Monker
     syntax; `A2s+` in particular (see SCHEMA.md § Range notation
     caveats) is handled differently. Fixtures enumerate the combos
     explicitly to dodge this.
   - **Iteration count.** TexasSolver has both `set_accuracy` (an
     exploitability target) and `set_max_iteration` (a hard cap). We
     set `accuracy = 0.3` (loose) so the iteration cap dominates. If
     TexasSolver still converges faster than our solver, it will stop
     early and report a strategy that hasn't been refined to the same
     point — flags as a small but consistent delta. Tighten
     `set_accuracy` on the oracle side (override via fixture) to force
     iteration parity.
   - **IP/OOP mapping.** The translator defaults `hero_is_ip=true`.
     If the spot is actually BB vs BTN with hero=BB, set
     `input.hero_position = "oop"` in the fixture.

## The file layout

| Path | Owner | Notes |
|---|---|---|
| `scripts/install-texassolver.sh` | A14 | macOS build. Clones + compiles + stages to `bin/`. |
| `scripts/install-texassolver-colab.sh` | A14 | Ubuntu/Debian variant for the nightly Colab validator. |
| `crates/solver-cli/src/translate.rs` | A14/A50 | Fixture JSON → TexasSolver `.tsconfig`. Rust module under `solver-cli translate-fixture`. Includes the whitespace / dash-range / `+` expansion logic that TexasSolver requires (see quirks section above). |
| `crates/solver-cli/tests/texassolver_diff.rs` | A14 | The diff runner. `#[ignore]`d. |
| `crates/solver-cli/tests/fixtures/` | A15 | The 20 canonical spots. Schema in `SCHEMA.md` (A15's file). |
| `bin/texassolver` | built, not committed | Produced by `install-texassolver.sh`. |
| `bin/resources/` | built, not committed | TexasSolver runtime tables. |
| `vendor/TexasSolver/` | cloned, not committed | Upstream source tree. |

## License — why this is legal

TexasSolver is licensed under **GNU AGPL v3**. AGPL is copyleft:
distributing modified versions, or running a modified version as a
network service, obligates you to release your full source under AGPL.

**We do none of those things:**

1. **We do not distribute TexasSolver.** `bin/texassolver` is produced
   by a build script on Henry's Mac (and on the Colab VM); it never
   ships in a Poker Panel release. Users never see it.
2. **We do not modify TexasSolver.** `install-texassolver.sh` clones the
   upstream source verbatim and builds it verbatim. Zero patches.
3. **We do not link against TexasSolver.** Our Rust code never imports,
   `extern "C"`-s, `dlopen`s, or otherwise links a TexasSolver object
   file. We shell out to the binary via `std::process::Command` and
   parse its stdout/output JSON. That is an *independent process
   boundary*, not a link.
4. **We do not run it as a public network service.** The AGPL §13
   network-use clause is triggered by serving modified versions over
   the network; we do neither.

In AGPL terms the relationship is: TexasSolver is a **separate program
on the same machine** that we run through a CLI and parse the output
of. That is the same relationship that `cargo test` has to every other
tool on the system (e.g. `gcc`, `make`, the Linux kernel) — none of
which impose their license on the test code.

This is the "aggregate on the same medium" carve-out the GPL FAQ
discusses:

> Mere aggregation of two programs means putting them side by side on
> the same CD-ROM or hard disk. We use this term in the case where they
> are separate programs, not parts of a single program. In this case,
> if one of the programs is covered by the GPL, it has no effect on the
> other program.
>
> — <https://www.gnu.org/licenses/gpl-faq.html#MereAggregation>

Additionally, TexasSolver's author has publicly granted an explicit
commercial carve-out: per the project README's Q&A,

> Q: Can I integrate it to my software?
> A: If you integrate the release package (binary) into your software,
> Yes, you can do that.

We are not integrating the binary into Poker Panel. But the author's
stated position is even more permissive than our actual use, so the
carve-out is belt-and-suspenders here.

### Future-agent rules

A future agent reading this doc might be tempted to:

- **Vendor TexasSolver source into this repo.** **Don't.** That would
  turn "clones an external AGPL project" into "distributes an AGPL
  project," which is a license-compliance problem we'd have to solve
  at ship time.
- **Link against `libTexasSolver.a`.** **Don't.** That is covered
  linking under AGPL §5, and our Rust crate would become an AGPL work.
  If we ever need tighter integration than shell-out, we write our own
  best-response algorithm — TexasSolver is only an oracle for testing.
- **Ship `bin/texassolver` to end users.** **Don't.** Even if the
  author's carve-out technically allows it, distributing an AGPL binary
  with Poker Panel would require us to ship its source too (§6) or
  point users at it, which is operational overhead we don't want.
- **Port chunks of TexasSolver's C++ into our Rust.** **Don't copy or
  translate TexasSolver code.** Re-implement from the papers in
  [docs/ALGORITHMS.md](ALGORITHMS.md) instead.

If any of the above looks attractive, stop and ask Henry.

## Nightly validation (Day 7+)

The Day 7 task kicks off a nightly Colab job that:

1. Pulls the latest `main` of this repo on the Colab VM.
2. Runs `scripts/install-texassolver-colab.sh` (idempotent; only
   rebuilds if the binary is missing).
3. Runs `cargo build --release -p solver-cli`.
4. Runs `cargo test -p solver-cli --test texassolver_diff -- --ignored`.
5. Posts results as a daily summary (JSON, not email — Henry hates email).

See [COLAB.md](COLAB.md) for the notebook layout and credentials story.

## Known limitations

- **EV comparison is one-sided today.** Our solver emits
  `ev_per_action` in its JSON; TexasSolver's strategy dump does not —
  the EV numbers only show up in its log file (`player 0 exploitability
  N%` lines). Parsing the log is future work. Until then, the
  differential test only enforces the **frequency** tolerance when both
  sides are populated, and silently accepts missing EVs on the
  TexasSolver side. Flagged in the `diff_spot` function in
  `texassolver_diff.rs` with a comment.
- **The runner is slow.** Each TexasSolver solve is tens of seconds of
  real CPU time. The 20-fixture run takes ~10-20 minutes, dominated by
  TexasSolver's flop solves. Acceptable for nightly; not acceptable for
  per-commit CI (hence `#[ignore]`).
- **Combo rollup is uniform-averaged.** When we summarize a
  TexasSolver strategy to a single root action frequency, we take a
  plain mean over combos in the range. The "right" rollup is a
  combo-weight-weighted mean (a combo with weight 0.5 counts half).
  Today our fixtures use all-weight-1 ranges, so mean == weighted
  mean. If future fixtures introduce fractional weights, update the
  rollup code (noted in `run_texassolver` comments).
- **Threading is non-deterministic.** TexasSolver's OpenMP
  parallelization introduces small run-to-run variation. With 1000+
  iterations this is sub-percent; within tolerance. For
  determinism-sensitive debugging, set `TS_THREADS=1` in the
  environment before running the test.
