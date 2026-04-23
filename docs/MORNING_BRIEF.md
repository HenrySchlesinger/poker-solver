# Morning brief — 2026-04-23

Audit agent A61 on HEAD `96db52e`. Sprint day 2 of 7. Target tag:
2026-04-29.

## What landed overnight

- **A58 `5629935`** — fixed the 30 GB river-CFR+ OOM; stack>0 now
  terminates on all-in. River solves are bounded again.
- **A59 `8ee6d1b` + `7a462c3` + `684e4a6`** — `solver_solve` wired
  end-to-end through the FFI boundary. Clippy + safety docs clean.
- **A47 `3328a99`** — `solver-cli solve` runs end-to-end on `stack=0`
  spots; paired `#[ignore]`s for the three `river_canonical` tests
  that still hit CFR+ walk OOM on non-zero stacks (`8e26e00`).
- **A60 `96db52e`** — xcframework dress rehearsal `v0.1.0-test2` (13
  MB zip); live `solver_solve` call verified from a SwiftPM consumer.
- **A51 `f250279` + `7e587c7` + `b8b480a`** — Metal benched and
  benched out. SIMD beats Metal **580×** at N=1326 (192.7 ns vs 112 µs).
  Metal module retained behind `--features metal`, SIMD ships.
- **A50** — TexasSolver differential oracle verified on spot_015
  (0.07% exploitability at 1000 iters in ~15 ms; 214 KB dump).
- **A49 `4a092f4`** — `flop-cache-v0.1.bin` (374 KB, 36 entries)
  shipped as format-only placeholder with version-gated loader.
- **A55/A56/A52/A53/A48** — bench baseline snapshot (`d1ea968`),
  CHANGELOG + release notes polish (`08979d9`, `49955e6`), release
  dry-run (`19336ad`), pre-commit hook (`1b100a8`), Colab .ipynbs
  (`0165d9b`).

## What's ship-ready

- Three FFI paths are green: Rust smoke test, Swift harness,
  SwiftPM xcframework consumer (A60).
- `libsolver_ffi.dylib` is 472 KB (well under the 10 MB gate).
- Kuhn Poker convergence test still canaries CFR+ (exploitability
  ~0.00486 @ 1000 iters).
- `regret_matching_inner` gate met: 192.7 ns at N=1326 vs < 1 µs
  target.
- 20-spot fixture battery pinned under `crates/solver-cli/tests/fixtures/`.

## What still blocks v0.1 tag

Five real blockers, not counting the 20+ "run ship.sh and capture
numbers on tag day" gates:

1. **`solver_version()` still returns `"0.1.0-dev"`.** Flip to
   `"0.1.0"` on tag commit (`crates/solver-ffi/src/lib.rs:222`).
2. **Modified `solver.h` is uncommitted** — the A59 header update
   from `684e4a6` needs to land before the tag.
3. **River / turn / equity / cache benches are not wired.** Baseline
   table rows are empty; `benches/river.rs` still runs Kuhn.
4. **TexasSolver diff has only covered 1 of 20 spots** (spot_015).
   The `validate` subcommand has not been run across the battery.
5. **Preflop ranges `.bin` has not been generated.** Loader exists,
   `data/preflop-ranges/` is `.gitkeep`-only.

Plus: 3 `river_canonical` tests are `#[ignore]`'d; stack=0 is the
only fully trusted live path.

## Suggested next move

Do **not** cut the tag today. Priority order for the next 48 hours:

1. Commit the `solver.h` diff + flip `solver_version()` to `"0.1.0"`.
2. Wire `river_canonical_spot` to the real NLHE subgame in
   `benches/river.rs`; re-run the bench baseline.
3. Run the full 20-spot TexasSolver diff (drop `#[ignore]` on
   `texassolver_diff` with `./bin/texassolver` present).
4. Generate and commit `preflop-v0.1.bin` from the preflop Colab.
5. Then run `scripts/ship.sh`, capture the numbers into the bench
   table, tag `v0.1.0`, push, publish release.

Realistic tag date on current trajectory: **2026-04-27 or 2026-04-28**,
one or two days inside the Day-7 target. Tag today would ship real
gaps in perf evidence and correctness coverage.

**Audit result: 14 boxes ticked, 37 open.**
