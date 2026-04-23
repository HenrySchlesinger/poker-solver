# Release process

This is the ship runbook for cutting a tagged release of `poker-solver`
that Poker Panel (or any other Swift consumer) can pin to via Swift
Package Manager or a manual `.a` / `.dylib` drop-in.

The pipeline is two scripts:

1. [`scripts/build-release.sh`](../scripts/build-release.sh) — produces a
   universal macOS tarball (`arm64` + `x86_64`) under
   `target/release-bundle/`.
2. [`scripts/gh-release.sh`](../scripts/gh-release.sh) — uploads that
   tarball + its sha256 sidecar + the C header to a GitHub Release.

If all you want is a clean mental model: **tag, build, release, verify,
pin.** The rest of this doc is the "what can go wrong" and "how to
rollback" coverage.

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

### 3. Dry-run build (optional but recommended)

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

The script prints the full contents listing and the outer tarball
sha256 at the end — you'll want the sha256 for the `Package.swift`
update in step 6.

Sanity-check the universal lib yourself:

```bash
lipo -info target/release-bundle/solver-$VERSION/lib/libsolver_ffi.a
# expected: "Architectures in the fat file: ... are: x86_64 arm64"
```

### 4. Publish the GitHub Release

```bash
bash scripts/gh-release.sh "$VERSION"
```

The script:

1. Refuses to run if `gh` is unauthenticated or the tag doesn't exist.
2. Attaches three assets to the release:
   - `solver-$VERSION-macos-universal.tar.gz` — the bundle.
   - `solver-$VERSION-macos-universal.tar.gz.sha256` — the outer sha.
   - `solver.h` — the C header, so consumers without SwiftPM can just
     curl it.
3. Uses `docs/RELEASE_NOTES_$VERSION.md` if it exists, else
   `docs/RELEASE_NOTES.md`, else a synthesized one-liner noting the sha.

### 5. Verify the published release

Visit `https://github.com/HenrySchlesinger/poker-solver/releases/tag/$VERSION`
in a browser. Download the tarball, then:

```bash
# Re-check the sha published in the release matches what we built:
cat target/release-bundle/solver-$VERSION-macos-universal.tar.gz.sha256
# ... and compare to the sha attached to the release.

# Extract into a scratch dir and confirm the contents:
tmpdir=$(mktemp -d)
tar xzf "$HOME/Downloads/solver-$VERSION-macos-universal.tar.gz" -C "$tmpdir"
ls "$tmpdir/solver-$VERSION"
# Expect: lib/ include/ VERSION CHECKSUMS.sha256
( cd "$tmpdir/solver-$VERSION" && shasum -a 256 -c CHECKSUMS.sha256 )
# All files should report `OK`.
```

### 6. Update `Package.swift` with the real checksum

The [`crates/solver-ffi/Package.swift`](../crates/solver-ffi/Package.swift)
manifest ships with `checksum: "TODO_CHECKSUM_AFTER_FIRST_RELEASE"`.
After the release is live, replace it with the tarball's sha256 and
bump the URL to the new version:

```bash
NEW_SHA=$(awk '{print $1}' target/release-bundle/solver-$VERSION-macos-universal.tar.gz.sha256)
# Edit crates/solver-ffi/Package.swift manually — one URL line, one
# checksum line — then:
git add crates/solver-ffi/Package.swift
git commit -m "release: Package.swift checksum for $VERSION"
git push origin main
```

Note: SwiftPM requires an `.xcframework` for `.binaryTarget(url:)`.
For v0.1 the `Package.swift` is scaffolding — Poker Panel integrates the
`.a` / `.dylib` manually per the next section. We'll ship a real
xcframework wrapper inside `build-release.sh` for v0.2.

### 7. Notify consumers

For v0.1, "consumers" = **Henry**, integrating into
`~/Desktop/Poker Panel/`. Post a quick note in that repo's worklog /
your todo list:

> poker-solver $VERSION shipped. SHA256: `<sha>`. Pull via
> `gh release download $VERSION -R HenrySchlesinger/poker-solver`.

---

## Manual consumer integration (v0.1 path)

SwiftPM's remote binary target support is xcframework-only, so for
v0.1 Poker Panel links the static lib directly. The steps:

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
