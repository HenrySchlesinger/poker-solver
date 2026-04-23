# Link Check (A69, 2026-04-23)

Final pre-ship hygiene pass across `docs/`, `README.md`, `CHANGELOG.md`,
and `CLAUDE.md`. Grep-only — no builds, no cargo.

## Summary

- Markdown files in repo (excl. `target/`, `.git/`, `vendor/`): 39
- Markdown files containing inter-doc links: 13
- Inter-doc markdown links found (relative `.md` paths): 90
- Broken (file missing): 0
- Broken (anchor missing): 1 (fixed — see below)
- Commit SHAs cited in CHANGELOG / MORNING_BRIEF / SHIP_V0_1: 27 unique
- Missing commit SHAs: 0

## Files scanned for links

All 13 files below were grep-scanned against the pattern
`\[([^\]]+)\]\(([^)]+\.md[^)]*)\)`:

- `README.md`
- `CLAUDE.md`
- `benches/README.md`
- `colab/README.md`
- `docs/ALGORITHMS.md`
- `docs/DIFFERENTIAL_TESTING.md`
- `docs/GETTING_STARTED.md`
- `docs/GLOSSARY.md`
- `docs/POKER.md`
- `docs/RELEASE_NOTES_v0.1.md`
- `docs/RELEASE_PROCESS.md`
- `docs/ROADMAP_V0_2.md`
- `crates/solver-cli/tests/fixtures/README.md`

`CHANGELOG.md` contains no inter-doc `.md` links (only a SHA reference).

## Broken links (fixed)

**`README.md:144`** — target was
`docs/RELEASE_PROCESS.md#manual-consumer-integration-v01-path`.
No heading in `RELEASE_PROCESS.md` produces that slug. The surrounding
README prose is the "manual-integration path (static lib + header, no
SwiftPM binary target yet) — Xcode drop-in steps" block, which is the
same content as `### Manual `.a` drop-in (for Xcode-without-SPM setups)`
in `RELEASE_PROCESS.md` (slug `manual-a-drop-in-for-xcode-without-spm-setups`).

Fix applied: anchor changed to `#manual-a-drop-in-for-xcode-without-spm-setups`.

## Broken links (flagged, not fixed)

None.

## Commit SHAs cited (all present in history)

Verified via `git cat-file -e <sha>`. All 27 resolve:

CHANGELOG.md: `3328a99`

MORNING_BRIEF.md: `0165d9b`, `08979d9`, `19336ad`, `1b100a8`, `3328a99`,
`49955e6`, `4a092f4`, `5629935`, `684e4a6`, `7a462c3`, `7e587c7`,
`8e26e00`, `8ee6d1b`, `96db52e`, `b8b480a`, `c05d926`, `d1ea968`,
`f250279`

SHIP_V0_1.md: `027906a`, `08979d9`, `19336ad`, `19a6967`, `2ad0a5e`,
`49955e6`, `4a092f4`, `5629935`, `684e4a6`, `7a462c3`, `7bc1f5e`,
`7e587c7`, `8e26e00`, `8ee6d1b`, `939f6b4`, `96db52e`, `a14c074`,
`d1ea968`, `d6b939e`, `da73746`, `de5e159`

No missing SHAs. (`8e26e00` appears alongside the "ignored CFR+ OOM
tests" note — still resolves as a commit.)

## Anchors verified (in-doc `#heading` links)

Cross-referenced every `#anchor` against target-file headings and
their GitHub-style slugs (lowercase, spaces → `-`, punctuation except
`-` stripped, em-dash " — " → `--`):

- `docs/POKER.md` anchors: `#three-worked-example-hands`,
  `#action-strings`, `#example-3-bluff-on-a-paired-river`,
  `#bet-sizing`, `#position-who-acts-when`, `#range-syntax`,
  `#example-2-turn-all-in-with-a-draw`, `#spr-stack-to-pot-ratio`,
  `#a-small-ascii-game-tree` — all present.
- `docs/ALGORITHMS.md`: `#discounted-cfr--discounted-cfr-post-v01`,
  `#vector-cfr--the-river-hot-path`, `#validation` — all present.
- `docs/ARCHITECTURE.md`: `#the-ffi-contract-solver-ffi`,
  `#why-opaque-handles-not-free-functions` — both present.
- `docs/REQUIREMENTS.md`: `#functional`, `#quality-targets` — both present.
- `docs/LIMITING_FACTOR.md`: `#2-simd-inner-loop-day-3` — present.
- `docs/ROADMAP.md`: `#day-6--2026-04-27-sunday` — present.
- `docs/DIFFERENTIAL_TESTING.md` (self-anchors):
  `#status-what-is-verified-2026-04-23`, `#why-texassolver`,
  `#acceptance-tolerances`, `#workflow`, `#the-file-layout`,
  `#license--why-this-is-legal`, `#nightly-validation-day-7`,
  `#known-limitations` — all present.
- `docs/GLOSSARY.md` (self-anchors): `#poker-terminology`,
  `#solver--algorithm-terminology`, `#implementation--rust-terminology`,
  `#product--business-terminology` — all present.
- `crates/solver-cli/tests/fixtures/SCHEMA.md` (self-anchors):
  `#input-fields`, `#tolerances-fields`, `#range-notation-caveats` —
  all present.

## Notes

- External URLs (`https://...`) not validated — no network at test time.
  The http links in `README.md` badges and in `RELEASE_PROCESS.md`
  example code blocks are out of scope for this pass.
- Agent-ID references (A47, A51, A55, …) are informal and intentionally
  not validated.
- `docs/ROADMAP_V0_2.md:31` contains `[LIMITING_FACTOR.md §5](LIMITING_FACTOR.md)`
  — the link target omits the `#...` anchor but the file exists and
  section 5 (`### 5. Vector CFR reformulation`) is real. Cosmetic, not
  broken. Left alone.
- `docs/ROADMAP_V0_2.md:64` contains `[SHIP_V0_1.md Data section](SHIP_V0_1.md)`
  — same shape: valid file link, imprecise display text. Left alone.
