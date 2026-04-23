# v0.1.0 tag runbook — copy-paste

Action-oriented commands for cutting the `v0.1.0` tag. Assumes Henry
has read [MORNING_BRIEF.md](MORNING_BRIEF.md) and decided to ship.

For the "why" / "is it ready" questions, see:
- [SHIP_V0_1.md](SHIP_V0_1.md) — per-box gate audit
- [MORNING_BRIEF.md](MORNING_BRIEF.md) — overnight summary
- [CHANGELOG.md](../CHANGELOG.md) — everything that shipped

This doc is just the sequence.

## 1. Pre-flight (30 sec)

```bash
cd ~/Desktop/poker-solver
git status --short                # expect: clean
git pull --ff-only origin main    # expect: already up to date
bash scripts/e2e.sh 2>&1 | tail -5
# expect: "e2e.sh: ALL PATHS GREEN"
```

If e2e is red, STOP and triage. Don't tag a broken tree.

## 2. Flip `solver_version` to "0.1.0" (A61 blocker #1)

```bash
# Edit crates/solver-ffi/src/lib.rs near `fn solver_version()`
# Change the byte literal from "0.1.0-dev\0" to "0.1.0\0"
# Also check grep for any other "0.1.0-dev" references
rg '0\.1\.0-dev' --glob '*.rs' --glob '*.md'
```

Commit:

```bash
git add crates/solver-ffi/src/lib.rs
git commit -m "release: bump solver_version to 0.1.0 for tag"
git push origin main
```

Re-run e2e to confirm nothing broke:

```bash
bash scripts/e2e.sh 2>&1 | grep "solver_version"
# expect: "solver_version": "0.1.0"
```

## 3. Verify `scripts/ship.sh` passes (5-10 min)

```bash
bash scripts/ship.sh 2>&1 | tail -20
# expect: exit 0, all gates pass
```

This runs fmt + clippy + workspace tests + bench-compile + FFI artifact
smoke. It's the deterministic ship gate.

## 4. Cut the tag (10 sec)

```bash
git tag -a v0.1.0 -m "poker-solver v0.1.0

First release of the local NLHE GTO solver. See CHANGELOG.md.

End-to-end verified: CLI, FFI, Swift (xcframework via SwiftPM).
Known gaps:
 - Exploitability reported as null/NaN (sentinel for v0.2 fix)
 - River perf is ~4.35x over target at 1000 iters (Vector CFR in v0.2)
 - Flop cache is a 36-entry placeholder (real data from Colab)"
git push origin v0.1.0
```

## 5. Build universal artifacts (~2 min)

```bash
rustup target add x86_64-apple-darwin aarch64-apple-darwin 2>/dev/null
bash scripts/build-release.sh v0.1.0
bash scripts/build-xcframework.sh v0.1.0
ls -lh target/release-bundle/ | grep v0.1.0
# expect both:
#   solver-v0.1.0-macos-universal.tar.gz (~14 MiB)
#   PokerSolver-v0.1.0.xcframework.zip   (~14 MiB)
shasum -a 256 target/release-bundle/PokerSolver-v0.1.0.xcframework.zip
# keep the SHA — you'll paste it into Package.swift below
```

## 6. Update Package.swift with the real SHA (30 sec)

```bash
# Edit crates/solver-ffi/Package.swift
# Replace `checksum: "FILL_AFTER_RELEASE"` with the sha256 from step 5.
# Also update the URL: it should point at
# https://github.com/HenrySchlesinger/poker-solver/releases/download/v0.1.0/PokerSolver-v0.1.0.xcframework.zip
git add crates/solver-ffi/Package.swift
git commit -m "release: pin v0.1.0 xcframework sha256 in Package.swift"
git push origin main
```

## 7. Publish the GitHub Release (30 sec)

```bash
bash scripts/gh-release.sh v0.1.0
# This attaches:
#   solver-v0.1.0-macos-universal.tar.gz  + .sha256
#   PokerSolver-v0.1.0.xcframework.zip    + .sha256
#   crates/solver-ffi/include/solver.h
# Release notes: docs/RELEASE_NOTES_v0.1.md
```

## 8. Verify the release page (manual)

Open https://github.com/HenrySchlesinger/poker-solver/releases/tag/v0.1.0
and confirm:
- All artifacts attached
- Release notes rendered
- Shasum files present

## 9. Smoke-test SwiftPM consumer against the public release (1 min)

```bash
mkdir -p /tmp/poker-solver-public-test && cd /tmp/poker-solver-public-test
cat > Package.swift <<'EOF'
// swift-tools-version:5.9
import PackageDescription
let package = Package(
    name: "PublicTest",
    platforms: [.macOS(.v13)],
    targets: [
        .binaryTarget(
            name: "PokerSolverBinary",
            url: "https://github.com/HenrySchlesinger/poker-solver/releases/download/v0.1.0/PokerSolver-v0.1.0.xcframework.zip",
            checksum: "PASTE_THE_SHA_FROM_STEP_5"
        ),
        .executableTarget(name: "PublicTest", dependencies: ["PokerSolverBinary"]),
    ]
)
EOF

mkdir -p Sources/PublicTest
cat > Sources/PublicTest/main.swift <<'EOF'
@_silgen_name("solver_version") func solver_version() -> UnsafePointer<CChar>
print(String(cString: solver_version()))
EOF

swift build 2>&1 | tail -5
.build/debug/PublicTest
# expect: "0.1.0"
cd ~/Desktop/poker-solver && rm -rf /tmp/poker-solver-public-test
```

## 10. Bump dev version back (post-tag housekeeping)

```bash
# Edit crates/solver-ffi/src/lib.rs, flip "0.1.0\0" → "0.1.1-dev\0"
# Prevents post-tag builds reporting 0.1.0 when they're actually ahead of the tag.
git add crates/solver-ffi/src/lib.rs
git commit -m "chore: bump solver_version to 0.1.1-dev post v0.1.0 tag"
git push origin main
```

## 11. Announce

- Commit message in Poker Panel saying "Add poker-solver v0.1.0 integration"
- ...and you're live.

## Rollback (if needed)

See [RELEASE_PROCESS.md#rollback](RELEASE_PROCESS.md) for the full
delete-tag + pre-release flow. The TL;DR:

```bash
git tag -d v0.1.0
git push origin :refs/tags/v0.1.0
gh release delete v0.1.0 --yes
```

Fix the issue, re-run from step 3.
