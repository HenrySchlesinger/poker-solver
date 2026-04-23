// swift-tools-version:5.9
//
// Package.swift — Swift Package Manager manifest for consuming the
// poker-solver FFI as a precompiled .xcframework from a GitHub Release.
//
// Intended consumer: ~/Desktop/Poker Panel/ (and any future Swift client).
//
// Binary target distribution strategy (v0.1 and forward):
//   - scripts/build-release.sh produces a universal .a/.dylib bundle.
//   - scripts/build-xcframework.sh wraps that into PokerSolver.xcframework
//     and tarballs it as PokerSolver-<VERSION>.xcframework.tar.gz.
//   - scripts/gh-release.sh attaches that tarball to the GitHub Release.
//   - This manifest's `.binaryTarget(url:)` points at that tarball.
//     SwiftPM fetches, verifies the sha256 checksum, and exposes the
//     universal static lib + headers as the module `PokerSolverBinary`.
//   - The `PokerSolver` module in Sources/ is a thin Swift wrapper that
//     re-exports the C symbols and adds a couple of Swifty conveniences.
//
// The URL and checksum below are set by a release step (documented in
// docs/RELEASE_PROCESS.md). Until the real v0.1.0 release is cut, the
// checksum reads "FILL_AFTER_RELEASE" as a sentinel so `swift build`
// fails fast with a useful error rather than trying to download a
// nonexistent asset. `swift package dump-package` succeeds either way
// (syntax is valid), which is what our CI checks.

import PackageDescription

let package = Package(
    name: "PokerSolver",
    platforms: [
        .macOS(.v13),
    ],
    products: [
        .library(
            name: "PokerSolver",
            targets: ["PokerSolver"]
        ),
    ],
    targets: [
        // The precompiled universal .xcframework produced by
        // scripts/build-xcframework.sh and attached to the GitHub Release.
        // SwiftPM downloads this lazily the first time `swift build` runs.
        // The URL must be a `.zip` — `.tar.gz` is not accepted by
        // `.binaryTarget(url:)` as of swift-tools-version 5.9.
        .binaryTarget(
            name: "PokerSolverBinary",
            url: "https://github.com/HenrySchlesinger/poker-solver/releases/download/v0.1.0/PokerSolver-v0.1.0.xcframework.zip",
            checksum: "FILL_AFTER_RELEASE"
        ),
        // Thin Swift wrapper. Re-exports PokerSolverBinary so consumers
        // only need `import PokerSolver`, and adds a small amount of
        // Swift sugar (status enum, version accessor) on top of the
        // raw C symbols.
        .target(
            name: "PokerSolver",
            dependencies: ["PokerSolverBinary"],
            path: "Sources/PokerSolver"
        ),
    ]
)
