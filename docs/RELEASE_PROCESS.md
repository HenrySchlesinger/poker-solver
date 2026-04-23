# Release process

This is the ship runbook for cutting a tagged release of `poker-solver`
that Poker Panel (or any other Swift consumer) can pin to via Swift
Package Manager or a manual `.a` / `.dylib` drop-in.

The pipeline is three scripts:

1. [`scripts/build-release.sh`](../scripts/build-release.sh) — produces a
   universal macOS tarball (`arm64` + `x86_64`) containing the raw
   `.a` / `.dylib` / header under `target/release-bundle/`.
2. [`scripts/build-xcframework.sh`](../scripts/build-xcframework.sh) —
   wraps that bundle into `PokerSolver.xcframework` and zips it as
   `PokerSolver-<VERSION>.xcframework.zip` for SwiftPM consumption.
3. [`scripts/gh-release.sh`](../scripts/gh-release.sh) — uploads both
   archives + their sha256 sidecars + the C header to a GitHub Release.

If all you want is a clean mental model: **tag, build, wrap, release,
verify, pin.** The rest of this doc is the "what can go wrong" and
"how to rollback" coverage.

---

## One-time prerequisites

These only need to happen once per machine.

### Toolchains

```bash
# Rust toolchain is pinned by rust-toolchain.toml (1.85.0 at time of
# writing). rustup honors it automatically.
rustup show

# Both macOS targets are required for the universal binary.
rustup target add aarch64-apple-darwin
rustup target add x86_64-apple-darwin
```

Verify:

```bash
rustup target list --installed | grep apple-darwin
# expected output:
#   aarch64-apple-darwin
#   x86_64-apple-darwin
```

### System tools

Shipped with Xcode command line tools (`xcode-select --install` if
missing):

- `lipo` — glues the per-arch libraries into a fat binary.
- `shasum` — sha256 checksums.
- `tar` — packaging.

Plus the GitHub CLI for the publish step:

```bash
brew install gh
gh auth login            # pick github.com, HTTPS, authenticate
gh auth status           # confirm scope includes `repo` (needed for releases)
```

### Cargo config (usually automatic)

Nothing beyond the pinned toolchain. If you see linker errors on
cross-compile, make sure you're not overriding `CC`/`LD` in your shell
profile — the stock Apple linker handles both targets.

---

## Day-to-day release flow

The flow assumes you're on `main`, the tree is clean, and the tag
commit is the HEAD commit. If you're somewhere else, stop and fix that
first.

### 1. Verify the code is shippable

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test  --workspace --release
cargo bench --workspace --no-run    # benches at least compile
```

All four must be green. See [`docs/SHIP_V0_1.md`](./SHIP_V0_1.md) for
the full ship-gate checklist — this doc is the mechanical part of that
checklist.

### 2. Create the tag

```bash
VERSION=v0.1.0                         # substitute the version you're cutting
git tag -a "$VERSION" -m "poker-solver $VERSION"
git push origin "$VERSION"
```

Tag names are `vMAJOR.MINOR.PATCH`. Prereleases: `v0.1.0-rc1`,
`v0.1.0-test`, etc. — the scripts treat them identically.

### 3. Build the raw bundle

```bash
bash scripts/build-release.sh "$VERSION"
```

The script:

1. Confirms both rustup targets are installed.
2. Builds `solver-ffi` twice — once per arch — with `cargo build --release`.
3. `lipo`s the resulting `.a` and `.dylib` into universal binaries.
4. Copies `crates/solver-ffi/include/solver.h` into the bundle.
5. Writes a `VERSION` file with git SHA + build timestamp + target list.
6. Writes `CHECKSUMS.sha256` inside the bundle.
7. Tarballs the bundle as
   `target/release-bundle/solver-$VERSION-macos-universal.tar.gz` and
   emits `<tarball>.sha256` next to it.

Sanity-check the universal lib yourself:

```bash
lipo -info target/release-bundle/solver-$VERSION/lib/libsolver_ffi.a
# expected: "Architectures in the fat file: ... are: x86_64 arm64"
```

### 4. Wrap it in an xcframework

```bash
bash scripts/build-xcframework.sh "$VERSION"
```

The script:

1. Fails fast if step 3's bundle isn't there or isn't universal.
2. Stages `solver.h` alongside a generated `module.modulemap` that
   declares `module PokerSolverBinary { header "solver.h"; export * }`.
   This is what lets Swift consumers do `import PokerSolverBinary`
   without a bridging header.
3. Runs `xcodebuild -create-xcframework -library <static lib> -headers
   <staged dir> -output target/release-bundle/PokerSolver.xcframework`.
4. Verifies the resulting `Info.plist` references the `macos-arm64_x86_64`
   slice — otherwise the xcframework is broken for Rosetta users.
5. Zips the xcframework with `ditto -c -k --keepParent` (Apple's
   notarization-compatible zip; SwiftPM accepts both `ditto` and `zip`
   outputs, but `ditto` preserves symlinks correctly if we ever add
   any).
6. Writes `PokerSolver-$VERSION.xcframework.zip` + `.sha256` next to it.
7. If `swift` is in `PATH`, also computes the canonical
   `swift package compute-checksum` value (empirically identical to
   sha256-of-file for zips, but reported separately for clarity).

**Why a zip, not a tar.gz:** `swift package dump-package` rejects
`.binaryTarget(url:)` values ending in anything other than
`artifactbundleindex` or `zip` with "unsupported extension". That's a
SwiftPM requirement going back to Swift 5.3.

**Why a modulemap:** without `module.modulemap` in the Headers folder,
Swift can't surface the C types as a module — `import PokerSolverBinary`
fails with "no such module". The script generates a minimal modulemap
that exports everything declared in `solver.h`.

Sanity-check:

```bash
ls target/release-bundle/PokerSolver.xcframework/
# expected: Info.plist  macos-arm64_x86_64
ls target/release-bundle/PokerSolver.xcframework/macos-arm64_x86_64/
# expected: Headers  libsolver_ffi.a
ls target/release-bundle/PokerSolver.xcframework/macos-arm64_x86_64/Headers/
# expected: module.modulemap  solver.h
```

### 5. Publish the GitHub Release

```bash
bash scripts/gh-release.sh "$VERSION"
```

The script:

1. Refuses to run if `gh` is unauthenticated, the tag doesn't exist,
   the raw tarball is missing, or the xcframework zip is missing.
2. Attaches five assets to the release (in this order):
   - `solver-$VERSION-macos-universal.tar.gz` — raw .a/.dylib bundle.
   - `solver-$VERSION-macos-universal.tar.gz.sha256` — its sha.
   - `PokerSolver-$VERSION.xcframework.zip` — SPM binaryTarget zip.
   - `PokerSolver-$VERSION.xcframework.zip.sha256` — the sha that
     goes into Package.swift `checksum:`.
   - `solver.h` — the C header, for consumers without SwiftPM.
3. Uses `docs/RELEASE_NOTES_$VERSION.md` if it exists, else
   `docs/RELEASE_NOTES.md`, else a synthesized note listing both shas.

### 6. Verify the published release

Visit `https://github.com/HenrySchlesinger/poker-solver/releases/tag/$VERSION`
in a browser. Confirm all five assets are attached, then:

```bash
# Compare published shas to what we built:
cat target/release-bundle/solver-$VERSION-macos-universal.tar.gz.sha256
cat target/release-bundle/PokerSolver-$VERSION.xcframework.zip.sha256

# Download the xcframework zip and verify it builds cleanly in a
# scratch SPM consumer:
tmpdir=$(mktemp -d)
cd "$tmpdir"
swift package init --type executable
# ... then edit Package.swift to add the binaryTarget block from
# docs/INTEGRATION.md section 4a, pointing at the release URL +
# newly-published sha. `swift build` should succeed.
```

### 7. Update `Package.swift` with the real checksum

The [`crates/solver-ffi/Package.swift`](../crates/solver-ffi/Package.swift)
manifest ships with `checksum: "FILL_AFTER_RELEASE"`. After the release
is live, replace it with the xcframework zip's sha256 and bump the URL
to the new version:

```bash
NEW_SHA=$(awk '{print $1}' target/release-bundle/PokerSolver-$VERSION.xcframework.zip.sha256)
# Edit crates/solver-ffi/Package.swift manually — one URL line, one
# checksum line — then:
git add crates/solver-ffi/Package.swift
git commit -m "release: Package.swift checksum for $VERSION"
git push origin main
```

Sanity: after the edit, `cd crates/solver-ffi && swift package dump-package`
should still succeed. If it fails with "unsupported extension", the URL
was edited to a non-zip value — revert.

### Dry-run results (v0.1.0-test)

Captured from a local run of the full pipeline against the v0.1.0-test
bundle:

| Asset | Size | SHA-256 |
| --- | --- | --- |
| `solver-v0.1.0-test-macos-universal.tar.gz` | ~11 MiB | `2d9696f66c2c1b67a114fe107ab61d19a633743e3a488f5a7b3ab679b560051c` |
| `PokerSolver-v0.1.0-test.xcframework.zip` | ~11 MiB | `190808d81ccaa14159c30175ffc3830b784dbdabfbf9398cded40dd783d8f00e` |

- `Info.plist` shape: `LibraryIdentifier = macos-arm64_x86_64`,
  `SupportedArchitectures = [arm64, x86_64]`, `SupportedPlatform = macos`.
- SwiftPM test-consumer package at `/tmp/test-consumer` built cleanly
  against the local xcframework, linked, and ran: `solver_version()`
  printed `0.1.0-wip`.
- SwiftPM test-wrapper package at `/tmp/test-wrapper` additionally
  compiled the `PokerSolver` Swift module (the thin wrapper in
  `crates/solver-ffi/Sources/PokerSolver/`) on top of the xcframework
  and ran: `PokerSolver.version` returned `0.1.0-wip`,
  `PokerSolverStatus.{ok, cacheMiss, invalidInput}` rawValues matched
  the enum in `solver.h`.

### Dry-run results (v0.1.0-dryrun, 2026-04-23, A52)

Re-verification run of the full pipeline after A28/A39 landed the
release + xcframework scripts. Ran both scripts end-to-end on an
arm64 Mac (`Darwin arm64`), verified artifacts, and exercised the
xcframework from a temporary SwiftPM consumer.

**Artifact sizes (verified):**

| Asset | Exact bytes | Human |
| --- | --- | --- |
| `lib/libsolver_ffi.a` (universal) | 31,692,552 | ~30.2 MiB |
| `lib/libsolver_ffi.dylib` (universal) | 33,360 | ~33 KiB |
| `solver-v0.1.0-dryrun-macos-universal.tar.gz` | 11,545,365 | ~11 MiB |
| `PokerSolver-v0.1.0-dryrun.xcframework.zip` | 11,541,970 | ~11 MiB |

**SHA-256 (verified):**

| Asset | SHA-256 |
| --- | --- |
| `solver-v0.1.0-dryrun-macos-universal.tar.gz` | `7e02ce4a6f8d618b68a7fc06f1a98d9188859d2aa2ea859578d7adf3c1f9ba33` |
| `PokerSolver-v0.1.0-dryrun.xcframework.zip` | `465d8265b6971b92eece8b767403fb11b01cf7b547d8ca5be0dff04f3cfd2fcd` |
| `lib/libsolver_ffi.a` (in-bundle) | `4058f89d2c2f0cfb9ccea74cabd84234c1ff1dac11f5e7f7c3bdb11d84f6501c` |
| `lib/libsolver_ffi.dylib` (in-bundle) | `24d999c0aaff0961c767a6faa97eb7f4b8f2fde6aad310bd9db2ea4ff402ab4e` |
| `include/solver.h` (in-bundle) | `426b3784839b39d0eb59224c304d82edd60d3d091741b4de1a0038e418053da8` |

`swift package compute-checksum` on the xcframework zip returned
exactly the file sha256 — these match for SPM-consumed `.zip`
archives as expected. That's the value that goes into Package.swift
`checksum:` when cutting the real release.

**Structural validation:**

- `lipo -info` on both the static and dynamic libraries reports
  `x86_64 arm64` — universal binary confirmed on both.
- `PokerSolver.xcframework/Info.plist` references the
  `macos-arm64_x86_64` slice, matching the expected layout.
- xcframework tree:
  - `Info.plist`
  - `macos-arm64_x86_64/libsolver_ffi.a`
  - `macos-arm64_x86_64/Headers/module.modulemap`
  - `macos-arm64_x86_64/Headers/solver.h`

**Test-consumer SPM build:**

Created a scratch SwiftPM executable under
`/tmp/poker-solver-test-consumer/` with a single
`.binaryTarget(path: "PokerSolver.xcframework")` pointing at the
local xcframework dir, a one-line `main.swift` calling
`@_silgen_name("solver_version")`, and ran `swift build`. Result:

- `swift build` completed in ~4 seconds (`Build complete! (3.92s)`).
- `.build/debug/TestConsumer` printed `0.1.0-wip` — the FFI symbol
  resolved against the universal `libsolver_ffi.a` embedded in the
  xcframework.

**Cross-compile gotchas actually hit:**

None on this run. `cargo build --release --target
{aarch64,x86_64}-apple-darwin -p solver-ffi` both succeeded without
any custom `CC`/`LD` overrides. The `zstd-sys`, `safe_arch`, and
`wide` crates all cross-compiled cleanly from an arm64 host to the
x86_64 slice. No stale `CARGO_BUILD_TARGET` in the shell, no
`PATH`-order surprises from brewed lld.

**Script status:**

- `scripts/build-release.sh` — worked first run, no edits needed.
- `scripts/build-xcframework.sh` — worked first run, no edits needed.
- `scripts/gh-release.sh` — not exercised here (dry run, no tag
  pushed). Status unchanged from A28/A39.

### 7. Notify consumers

For v0.1, "consumers" = **Henry**, integrating into
`~/Desktop/Poker Panel/`. Post a quick note in that repo's worklog /
your todo list:

> poker-solver $VERSION shipped. SHA256: `<sha>`. Pull via
> `gh release download $VERSION -R HenrySchlesinger/poker-solver`.

---

## Consumer integration paths

### SPM binaryTarget (preferred, requires xcframework)

The v0.1 release ships a proper `.xcframework.zip`, so consumers can
wire it in via `Package.swift`:

```swift
.binaryTarget(
    name: "PokerSolverBinary",
    url: "https://github.com/HenrySchlesinger/poker-solver/releases/download/v0.1.0/PokerSolver-v0.1.0.xcframework.zip",
    checksum: "<sha from PokerSolver-v0.1.0.xcframework.zip.sha256>"
),
```

See `docs/INTEGRATION.md` section 4a for the full example including
the optional Swift wrapper module.

### Manual `.a` drop-in (for Xcode-without-SPM setups)

If you want to skip SwiftPM, the raw `.a`/`.dylib` tarball is still
attached to the release:

```bash
# Somewhere under ~/Desktop/Poker Panel/vendor/poker-solver/:
gh release download v0.1.0 -R HenrySchlesinger/poker-solver \
    --pattern 'solver-v0.1.0-macos-universal.tar.gz*'

shasum -a 256 -c solver-v0.1.0-macos-universal.tar.gz.sha256
tar xzf solver-v0.1.0-macos-universal.tar.gz
```

Then in Xcode:

1. Drag `solver-v0.1.0/lib/libsolver_ffi.a` into the Poker Panel target's
   "Frameworks, Libraries, and Embedded Content".
2. Add a bridging header that does `#include "solver.h"` and point Xcode
   at `solver-v0.1.0/include/` as a header search path.
3. Rebuild Poker Panel. The `solver_*` functions are now callable from
   Swift.

See `crates/solver-ffi/examples/swift-harness/` in this repo for the
minimal swiftc invocation pattern.

---

## Rollback

You cannot un-ship a tag that users have already pulled, but you can:

### Pull the release assets

If a release is broken (bad build, wrong arch, regression) and nobody
has pinned to it yet:

```bash
VERSION=v0.1.0
gh release delete "$VERSION" --yes       # removes the Release + its assets
git tag -d "$VERSION"                    # delete local tag
git push --delete origin "$VERSION"      # delete remote tag
```

Then fix the underlying issue and re-tag as the **next** patch version
(`v0.1.1`). Do not re-use `v0.1.0` — even if you deleted the tag, some
consumers may have cached it.

### Yank without deleting (safer)

If the release has been out for more than a few hours, prefer yanking
(marking as pre-release so `gh release download --latest` skips it) over
deletion:

```bash
gh release edit "$VERSION" --prerelease
```

Post a follow-up release with the fix and notify consumers explicitly.

---

## Troubleshooting

### `error: component 'rust-std' for target 'x86_64-apple-darwin' is not available`

You're on a non-standard rustup channel (nightly with missing targets,
usually). Either `rustup default stable` or install the component
explicitly:

```bash
rustup component add rust-std --target x86_64-apple-darwin
```

### `lipo: ... have the same architectures (arm64) and can't be in the same fat output file`

You built the same arch twice. Make sure `$CARGO_BUILD_TARGET` isn't set
in your environment — `build-release.sh` passes `--target` explicitly,
but a globally set `CARGO_BUILD_TARGET` overrides it.

```bash
unset CARGO_BUILD_TARGET
bash scripts/build-release.sh "$VERSION"
```

### `gh: unauthenticated`

```bash
gh auth login           # follow prompts
gh auth status          # confirm
```

### The tarball is larger than 20 MiB

That's a sign something unintended got packaged. `build-release.sh`
only copies the lib, the header, the VERSION file, and CHECKSUMS. If
the tarball is huge, inspect:

```bash
tar tzf target/release-bundle/solver-$VERSION-macos-universal.tar.gz
```

Common culprits: stale debug symbols (check `debug = false` in
`[profile.release]`), an accidentally-committed test fixture under
`crates/solver-ffi/`, or a rogue `OUT_DIR`.

### `cargo build` succeeds but links against the host's libSystem

Cross-compiling arm64 on x86 (or vice versa) works fine with stock
Rust + Apple toolchains because both archs share the same system libs.
If a Mac-specific dependency hardcodes `cc` flags, it may break — at
which point you isolate the dep and either vendor or gate it behind
`#[cfg(target_arch)]`.

---

## See also

- [`docs/SHIP_V0_1.md`](./SHIP_V0_1.md) — the full ship checklist.
- [`docs/ARCHITECTURE.md`](./ARCHITECTURE.md) — crate layout and FFI contract.
- [`crates/solver-ffi/examples/swift-harness/`](../crates/solver-ffi/examples/swift-harness/) — minimal
  Swift-from-shell example that exercises the FFI symbols.
