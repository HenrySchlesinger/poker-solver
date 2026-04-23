// swift-tools-version:5.9
//
// Package.swift — Swift Package Manager manifest for consuming the
// poker-solver FFI as a precompiled binary from a GitHub Release.
//
// Intended consumer: ~/Desktop/Poker Panel/ (and any future Swift client).
//
// Binary target distribution strategy:
//   - Our GitHub Release uploads a single universal-macOS tarball
//     produced by scripts/build-release.sh. The tarball contains
//     lib/libsolver_ffi.{a,dylib} plus include/solver.h.
//   - Swift Package Manager expects a .xcframework for remote binary
//     targets, which is a superset of what we ship today. For v0.1 we
//     document the manual integration path in docs/RELEASE_PROCESS.md
//     and ship this manifest as an *example consumer scaffold* that a
//     downstream project can copy, edit, and point at its own xcframework
//     build. We keep it compilable-as-a-standalone-package so
//     `swift package dump-package` against a copy succeeds.
//
// Post-v0.1 TODO: wrap the universal dylib in an xcframework inside
// build-release.sh and flip the `.binaryTarget(url:)` below to reference
// the real release URL + checksum.

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
        // Binary target placeholder. The URL points at the canonical v0.1.0
        // tarball shape produced by scripts/build-release.sh. The checksum
        // below is a placeholder; the real value is written into
        // target/release-bundle/solver-<VERSION>-macos-universal.tar.gz.sha256
        // after build-release.sh runs, and gets substituted in by
        // scripts/gh-release.sh's post-publish reminder.
        //
        // NOTE: SwiftPM requires .xcframework for remote binary targets.
        // For v0.1, consumers should integrate the .a/.dylib + header
        // manually per docs/RELEASE_PROCESS.md. This target is scaffolding
        // for the v0.2 xcframework shape.
        .binaryTarget(
            name: "PokerSolverBinary",
            url: "https://github.com/HenrySchlesinger/poker-solver/releases/download/v0.1.0/solver-v0.1.0-macos-universal.tar.gz",
            checksum: "TODO_CHECKSUM_AFTER_FIRST_RELEASE"
        ),
        .target(
            name: "PokerSolver",
            dependencies: ["PokerSolverBinary"],
            path: "Sources/PokerSolver"
        ),
    ]
)
