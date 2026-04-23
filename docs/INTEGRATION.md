# Poker Panel integration spec

**Audience:** a future Swift engineer on Poker Panel (or Henry, with
this doc in one hand and Xcode in the other) who wants to consume
`poker-solver` without needing to know how CFR works.

**Goal of this doc:** stand alone. You should be able to integrate v0.1
from this file plus the release artifact alone, no need to read the
rest of the `poker-solver` repo. If something here contradicts the
code or the header, the code wins and this doc is a bug.

**Scope:** v0.1 contract only. If you're reading this after v0.2 shipped,
check `CHANGELOG.md` for breaking changes before trusting any claim
below.

---

## 1. What you're integrating

`poker-solver` is a Rust library that computes GTO strategies for
NLHE hand states. It has:

- A C header (`solver.h`) declaring ~5 `extern "C"` symbols.
- A static library (`libsolver_ffi.a`) and a dylib
  (`libsolver_ffi.dylib`) that export those symbols.
- A pile of shipped data files (preflop ranges, iso tables, a subset
  of the flop cache).

You link the library, include the header, and call `solver_solve`
whenever you have a hand state you want an overlay strategy for.
Everything runs on the user's Mac. No network, no cloud, no runtime
dependencies on Python or a Rust toolchain.

See `docs/ARCHITECTURE.md` for the full picture on our side; most of
this doc is "here is the Swift-shaped slice of that."

---

## 2. Obtain the release artifact

### 2a. From GitHub Releases (preferred)

1. Visit <https://github.com/henryschlesinger/poker-solver/releases>.
2. Pick the release you want to integrate against (e.g. `v0.1.0`).
3. Download:
   - `libsolver_ffi.a` — static library (use this for app bundling).
   - `libsolver_ffi.dylib` — dylib (only if you're dynamically loading).
   - `solver.h` — the generated C header.
   - `CHANGELOG.md` — read the `[0.1.0]` section before integrating.
4. Verify the SHA-256 of each artifact against the checksums in the
   release body. This is the supply-chain check — the release body is
   the source of truth for "the tag shipped this exact byte sequence."

### 2b. Build from source (for reproducibility)

If you need to verify the release was built honestly, or you're
running on a fork:

```bash
git clone https://github.com/henryschlesinger/poker-solver
cd poker-solver
git checkout v0.1.0
cargo build --release -p solver-ffi
```

After that:

- `target/release/libsolver_ffi.a` → copy into Xcode.
- `target/release/libsolver_ffi.dylib` → optional, for dlopen users.
- `crates/solver-ffi/include/solver.h` → copy into Xcode.

Requires a Rust toolchain matching `rust-toolchain.toml` (v0.1.0
pins 1.82.0). Don't build on Windows — we don't ship Windows and
haven't tested the generated header there.

---

## 3. Minimum platform

From `docs/REQUIREMENTS.md`:

- **macOS 13+** on Apple Silicon (M1, M2, M3, M4). Primary target.
- **macOS 13+** on Intel via Rosetta. Secondary, unoptimized. The
  Metal fast path (if enabled in a future version) won't kick in.
- **Not supported:** Windows, Linux, iOS. Do not attempt; we don't
  test there and will not accept issues.

Architecture-wise, the static lib is a universal binary for v0.1
(arm64 + x86_64). Check with `lipo -info libsolver_ffi.a` after
downloading if you want to confirm.

---

## 4. Add to a Swift Package Manager target

### 4a. As a binary target in Package.swift (preferred)

This is the shape we want Poker Panel using. v0.1.0 ships a real
`.xcframework.zip` asset on the GitHub Release, so you can wire it in
directly — no hand-wrapping needed.

The release attaches **`PokerSolver-v0.1.0.xcframework.zip`** alongside
the raw `.a`/`.dylib` tarball. Both ship; the `.xcframework.zip` is the
SwiftPM-facing one.

```swift
// Package.swift
let package = Package(
    name: "PokerPanel",
    platforms: [.macOS(.v13)],
    targets: [
        .binaryTarget(
            name: "PokerSolverBinary",
            url: "https://github.com/HenrySchlesinger/poker-solver/releases/download/v0.1.0/PokerSolver-v0.1.0.xcframework.zip",
            checksum: "<sha-256-from-release-body>"
        ),
        .target(
            name: "PokerPanelCore",
            dependencies: ["PokerSolverBinary"]
        ),
    ]
)
```

The checksum is the `swift package compute-checksum` value of the
downloaded zip — same as the sha256 shipped in
`PokerSolver-v0.1.0.xcframework.zip.sha256` on the release. Verify
locally before committing:

```bash
gh release download v0.1.0 -R HenrySchlesinger/poker-solver \
    --pattern 'PokerSolver-v0.1.0.xcframework.zip'
swift package compute-checksum PokerSolver-v0.1.0.xcframework.zip
# paste into checksum:
```

**What's inside the xcframework:** a single `macos-arm64_x86_64` slice
containing `libsolver_ffi.a` (universal static lib) plus
`Headers/solver.h` and `Headers/module.modulemap`. The modulemap
declares a module named `PokerSolverBinary`, so after wiring the
binary target you can `import PokerSolverBinary` from Swift and see
the `HandState`, `SolveResult`, `solver_*` symbols directly — no
bridging header required.

**Optional Swifty wrapper:** this repo ships a thin Swift module
(`crates/solver-ffi/Sources/PokerSolver/PokerSolver.swift`) that
re-exports `PokerSolverBinary` and adds a `PokerSolverStatus` enum
plus a `PokerSolver.version` accessor. To use it, add the `PokerSolver`
target from this repo to your Package.swift, or copy the ~30 lines of
Swift into your own module.

### 4b. Direct linkage in an Xcode target (no SPM, no xcframework)

If you're just dropping into the existing `Poker Panel.xcodeproj` and
don't want SwiftPM in the mix, use the raw `.a`/`.dylib` tarball:

1. `gh release download v0.1.0 -R HenrySchlesinger/poker-solver --pattern 'solver-v0.1.0-macos-universal.tar.gz*'`
2. Extract, then add `libsolver_ffi.a` to the `Poker Panel` target's
   **Frameworks, Libraries, and Embedded Content**.
3. Add `solver.h` to the project, and in the target's
   **Build Settings** set **Objective-C Bridging Header** to the path
   to `solver.h` (or `#include` it from an existing bridging header).
4. Under **Build Settings → Library Search Paths**, add the directory
   containing `libsolver_ffi.a`.
5. Build. If Swift can't see `HandState` or `solver_solve`, the
   bridging header isn't being picked up.

---

## 5. FFI contract

The authoritative source is `crates/solver-ffi/include/solver.h` in
the release. What follows is a plain-English summary; if there is
any disagreement between this section and the header, the header is
correct.

### 5a. Symbols

```c
SolverHandle* solver_new(void);
void          solver_free(SolverHandle* handle);

int32_t solver_solve(SolverHandle*    handle,
                     const HandState* input,
                     SolveResult*     output);

int32_t solver_lookup_cached(const HandState* input,
                             SolveResult*     output);

const char* solver_version(void);
```

### 5b. Types

**`HandState`** — what you pass in:

- `board[5]` + `board_len` — 0–5 cards, packed as `u8` per card.
- `hero_range[1326]`, `villain_range[1326]` — f32 weight per combo.
- `pot`, `effective_stack` — chips, `u32`.
- `to_act` — 0 for hero, 1 for villain.
- `bet_tree_version` — 0 for the v0.1 defaults.

**`SolveResult`** — what you read back:

- `solver_version` — `u32` matching the solver build.
- `action_count` — number of valid entries in `action_freq` and
  `action_ev` (up to 8).
- `action_freq[8]` — f32 frequencies, sum to 1.0.
- `action_ev[8]` — f32 EV per action, in big blinds.
- `hero_equity` — f32.
- `exploitability` — f32, lower = closer to Nash.
- `iterations`, `compute_ms` — `u32`.

See the `HandState` / `SolveResult` typedefs in `solver.h` for the
exact layout. **Do not reorder fields or change sizes on the Swift
side** — these structs are copied across the FFI boundary byte-for-
byte.

### 5c. Return codes (`SolverStatus`)

From `solver.h`:

|  Code | Name             | Meaning                                                      |
| ----: | ---------------- | ------------------------------------------------------------ |
|   `0` | `Ok`             | Success. `SolveResult` is valid.                             |
|   `1` | `CacheMiss`      | `solver_lookup_cached` only. Caller should fall through to `solver_solve`. |
|  `-1` | `InvalidInput`   | Null pointer, malformed `HandState`, impossible stack/pot.   |
|  `-2` | `InternalError`  | A Rust panic was caught at the FFI boundary. File a bug.     |
|  `-3` | `OutputTooSmall` | Reserved. Does not fire in v0.1 since we pass the struct by pointer. |

Treat any unknown code as a bug in the integration (newer solver,
older header). Log and fall back to the previous overlay state — do
**not** trust a `SolveResult` whose status wasn't `Ok`.

---

## 6. Call patterns

### 6a. Create a handle, reuse it

`SolverHandle` owns scratch memory (tens of MB for river, GB for
turn). Allocating per-call is prohibitive — create one handle per
solver thread and reuse it for the life of the broadcast.

```swift
final class SolverSession {
    private let handle: OpaquePointer

    init?() {
        guard let h = solver_new() else { return nil }
        self.handle = h
    }

    deinit { solver_free(handle) }

    func solve(_ state: inout HandState) -> SolveResult? {
        var result = SolveResult()
        let status = solver_solve(handle, &state, &result)
        guard status == Ok else {
            NSLog("[solver] status=\(status)")
            return nil
        }
        return result
    }
}
```

### 6b. Cache-first, solve-second

The flop-cache path is much faster than a live solve. Always try it
first:

```swift
func strategy(for state: inout HandState) -> SolveResult? {
    var result = SolveResult()
    let cached = solver_lookup_cached(&state, &result)
    if cached == Ok { return result }
    if cached != CacheMiss { return nil }        // real error
    // fall through to live solve
    return solve(&state)
}
```

### 6c. Version check at startup

Poker Panel reads `solver_version()` at launch and refuses to load
a mismatched build (`docs/ARCHITECTURE.md`). This catches the case
where someone drops in a newer `.a` without rebuilding the Swift
side — the struct layouts may have moved.

```swift
let ver = String(cString: solver_version())
guard ver.hasPrefix("0.1.") else {
    fatalError("solver version mismatch: got \(ver), want 0.1.x")
}
```

The release body lists the exact version string for that tag.

---

## 7. Threading model

From `docs/ARCHITECTURE.md` and `solver.h`:

- **Each `SolverHandle` is single-threaded.** Do not call
  `solver_solve` with the same handle concurrently from two threads.
  You'll corrupt scratch memory.
- **Concurrent solves = multiple handles.** Create a pool sized to
  your worst-case concurrent overlay count. One handle per thread is
  fine; pools are only needed if you genuinely fan out.
- **Inside a solve**, the library parallelizes across info sets with
  `rayon`. That's intra-solve parallelism — it does not require you
  to hand us multiple handles.
- **`solver_lookup_cached` is handle-free and thread-safe.** Call it
  from anywhere. It reads the cache under an internal read-only
  view; concurrent callers do not conflict.
- **`solver_version()` is thread-safe** and returns a static string
  you must not free.
- **No async.** CFR is CPU-bound. Don't wrap these calls in
  `Task.detached` expecting cooperative scheduling — they'll occupy
  a whole thread for the solve duration.

Rule of thumb for Poker Panel: one solver session per active table
being overlaid. A live broadcast is typically one table at a time,
so one handle is usually enough.

---

## 8. Error handling

- **Never ignore the return code.** Every `solver_solve` call must
  check the status before reading `SolveResult`.
- **Treat `InternalError` (-2) as a bug to file.** That means we
  caught a panic in the Rust side — not your fault, and the
  `SolveResult` is undefined.
- **`InvalidInput` (-1) is your bug.** Most commonly: ranges that
  don't sum to anything, negative pot, `to_act` outside {0, 1},
  board cards outside 0..52.
- **`CacheMiss` (+1) is expected** from `solver_lookup_cached` —
  it's the normal "need to solve live" signal. Don't alarm on it.

Suggested overlay UX: if `solver_solve` fails with any non-`Ok`
code, **keep the previous overlay visible rather than rendering
garbage**. Show a tiny indicator ("solver: stale") if you want
operators to know.

---

## 9. Verifying the integration works

Three levels of confidence, increasing:

### 9a. The smoke test (1 minute)

```swift
let v = String(cString: solver_version())
print("solver version:", v)
```

Should print the release tag (e.g. `"0.1.0"`). If this crashes or
prints garbage, the linker isn't finding `libsolver_ffi.a`.

### 9b. The ABI round-trip (5 minutes)

Run the Swift harness we ship with the library:

```bash
cd crates/solver-ffi/examples/swift-harness/
# Follow README.md in that directory.
swiftc main.swift \
    -import-objc-header ../../include/solver.h \
    -L ../../../../target/release \
    -lsolver_ffi \
    -o swift-harness
./swift-harness
```

Expected output in that directory's `README.md`. If every symbol
prints a status code from the `SolverStatus` enum (not `Ok` yet —
v0.1 stubs often return `InternalError` or `CacheMiss` deliberately)
and nothing crashes, the ABI is loadable.

### 9c. The correctness round-trip (15 minutes)

Pick a fixture from `fixtures/` (spot 001 is the dry-board baseline).
Feed its `HandState` into `solver_solve` from your Swift code. Read
the `SolveResult`. Compare to the expected output in the fixture
file (generated by `solver-cli validate` against TexasSolver).

If the numbers agree within 5% per-action frequency, the integration
is real. If not, check:

- Are you sending the ranges in the same combo order solver-eval
  uses? See `solver-eval::combo::combo_index`.
- Is `bet_tree_version` = 0 (v0.1 default tree)?
- Is `to_act` pointing at the right player?

---

## 10. What this spec does NOT cover

- **How to solve multi-way pots.** Not in v0.1. Heads-up only.
- **How to do ICM-adjusted EVs.** Not in v0.1. Cash-style EV only.
- **PLO.** Not in v0.1. NLHE only.
- **Exploit / node-locking APIs.** Not shipping. GTO only.
- **A Swift-native wrapper.** Not in the repo. If you want a Swifty
  API on top of these C calls, write it in Poker Panel; we don't own
  it here.
- **Telemetry.** The solver doesn't phone home. If you want usage
  metrics, collect them in Poker Panel.

---

## 11. Upgrading to a future version

When v0.2 (or v0.1.1) drops:

1. Read `CHANGELOG.md` for the new tag. Breaking FFI changes are
   called out in **bold** in the release notes.
2. If `solver_version()` changed its major/minor, expect the struct
   layouts may have moved. Rebuild Poker Panel against the new
   header, don't just drop in the new `.a`.
3. Re-run the Swift harness (section 9b) against the new library to
   confirm the ABI loads cleanly.
4. Re-run the fixture round-trip (section 9c) to confirm strategies
   still agree with TexasSolver within the documented tolerance.

Downgrading is also fine — v0.1.0 is a forever-frozen tag.

---

## 12. Filing bugs

Bugs go on GitHub: <https://github.com/henryschlesinger/poker-solver/issues>.
Use the `bug_report.md` template. The most useful info is:

- Solver tag / `solver_version()` output.
- `HandState` that triggered the bug (board, ranges, pot, stacks,
  to_act). Feel free to anonymize the ranges.
- Return code + full `SolveResult` contents, if any.
- Poker Panel version.

We're not promising response SLAs on a v0.1 indie release — flag
anything critical directly to Henry.
