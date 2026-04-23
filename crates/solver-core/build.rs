//! Build script for `solver-core`.
//!
//! Currently only used when the `metal` feature is enabled. Attempts to
//! compile the Metal Shading Language source at
//! `src/metal/shaders/regret_matching.metal` into a `regret_matching.metallib`
//! emitted into `OUT_DIR`, which the Rust code then embeds via
//! `include_bytes!`.
//!
//! # Graceful failure — this is load-bearing
//!
//! `xcrun -sdk macosx metal` availability is fragile. On fresh macOS
//! installs without the full Xcode "Metal Toolchain" component (which
//! Xcode now gates behind `xcodebuild -downloadComponent MetalToolchain`),
//! the compiler is missing. Henry's machine is currently in this state
//! — `xcrun metal --version` returns:
//!
//!     error: cannot execute tool 'metal' due to missing Metal Toolchain
//!
//! Rather than hard-failing `cargo build`, we:
//!
//! 1. Try to compile `.metal` → `.air` → `.metallib`.
//! 2. If any step fails, emit a `cargo:warning=` with the root cause,
//!    write an empty `regret_matching.metallib` into `OUT_DIR`, and set
//!    `cargo:rustc-cfg=no_metallib_available` so the Rust code knows to
//!    skip the embedded-metallib fast path.
//!
//! When `no_metallib_available` is set, the Rust code falls back to
//! compiling the shader at runtime via `MTLDevice::newLibraryWithSource`.
//! That path is slightly slower on first init (~10–30ms shader compile)
//! but produces identical GPU code and doesn't need a build-time
//! toolchain. It works on any Mac with the Metal framework, which is
//! every M-series Mac.
//!
//! If neither the build-time nor the runtime path works (e.g. no
//! Metal-capable device at all), `MetalContext::new()` returns an error
//! and the caller falls back to the SIMD/scalar implementations.
//!
//! When the `metal` feature is NOT enabled, this script is a no-op —
//! the whole build-time compile step only runs if Metal is wanted.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Tell cargo to rerun this script when the inputs change.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/metal/shaders/regret_matching.metal");
    // Also rerun when the feature flag toggles. Cargo handles this
    // implicitly via `CARGO_FEATURE_METAL`, but an explicit rerun hint
    // keeps the dependency graph obvious.
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_METAL");

    // Declare the cfgs we might emit so rustc doesn't warn about them.
    // (Required on stable 1.80+ per RFC 3383.)
    println!("cargo:rustc-check-cfg=cfg(no_metallib_available)");
    println!("cargo:rustc-check-cfg=cfg(has_metallib)");

    // Only compile shaders when the metal feature is on.
    if env::var("CARGO_FEATURE_METAL").is_err() {
        return;
    }

    // Metal shaders only make sense on macOS targets.
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        println!(
            "cargo:warning=solver-core build.rs: metal feature enabled but target_os={target_os} (not macos); skipping shader compile"
        );
        emit_empty_metallib();
        return;
    }

    let crate_dir = match env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(e) => {
            println!(
                "cargo:warning=solver-core build.rs: CARGO_MANIFEST_DIR unset ({e}); embedding empty metallib"
            );
            emit_empty_metallib();
            return;
        }
    };
    let out_dir = match env::var("OUT_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(e) => {
            println!(
                "cargo:warning=solver-core build.rs: OUT_DIR unset ({e}); embedding empty metallib"
            );
            emit_empty_metallib();
            return;
        }
    };

    let metal_src = crate_dir.join("src/metal/shaders/regret_matching.metal");
    if !metal_src.exists() {
        println!(
            "cargo:warning=solver-core build.rs: shader source missing at {}; embedding empty metallib",
            metal_src.display()
        );
        emit_empty_metallib();
        return;
    }

    let air_path = out_dir.join("regret_matching.air");
    let metallib_path = out_dir.join("regret_matching.metallib");

    // Step 1: .metal -> .air
    let air_result = Command::new("xcrun")
        .args(["-sdk", "macosx", "metal", "-c", "-o"])
        .arg(&air_path)
        .arg(&metal_src)
        .output();

    let air_output = match air_result {
        Ok(o) => o,
        Err(e) => {
            println!(
                "cargo:warning=solver-core build.rs: xcrun metal not available ({e}); falling back to runtime shader compile"
            );
            emit_empty_metallib();
            return;
        }
    };
    if !air_output.status.success() {
        let stderr = String::from_utf8_lossy(&air_output.stderr);
        // Shorten noisy multi-line errors to the first line in the
        // cargo warning — full output is preserved in the returned
        // Result if anyone debugs.
        let first = stderr.lines().next().unwrap_or("unknown error");
        println!(
            "cargo:warning=solver-core build.rs: xcrun metal -c failed ({}): {}; falling back to runtime shader compile",
            air_output.status, first
        );
        emit_empty_metallib();
        return;
    }

    // Step 2: .air -> .metallib
    let lib_result = Command::new("xcrun")
        .args(["-sdk", "macosx", "metallib", "-o"])
        .arg(&metallib_path)
        .arg(&air_path)
        .output();

    let lib_output = match lib_result {
        Ok(o) => o,
        Err(e) => {
            println!(
                "cargo:warning=solver-core build.rs: xcrun metallib not available ({e}); falling back to runtime shader compile"
            );
            emit_empty_metallib();
            return;
        }
    };
    if !lib_output.status.success() {
        let stderr = String::from_utf8_lossy(&lib_output.stderr);
        let first = stderr.lines().next().unwrap_or("unknown error");
        println!(
            "cargo:warning=solver-core build.rs: xcrun metallib failed ({}): {}; falling back to runtime shader compile",
            lib_output.status, first
        );
        emit_empty_metallib();
        return;
    }

    // Success — metallib was generated. Let the Rust code know it's
    // safe to include_bytes! the output.
    println!("cargo:rustc-cfg=has_metallib");
    println!(
        "cargo:warning=solver-core build.rs: compiled regret_matching.metallib at {}",
        metallib_path.display()
    );
}

/// Emit a zero-byte `regret_matching.metallib` into `OUT_DIR` and set
/// `cfg(no_metallib_available)` so the Rust code uses the runtime-source
/// fallback instead of the embedded-metallib fast path.
///
/// The empty file must exist because `include_bytes!` takes a literal
/// path and fails at compile time if the path is missing. We always
/// need the file to exist; the cfg flag tells the runtime code to
/// ignore its contents.
fn emit_empty_metallib() {
    println!("cargo:rustc-cfg=no_metallib_available");

    let out_dir = match env::var("OUT_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => return,
    };
    let metallib_path = out_dir.join("regret_matching.metallib");
    // Ignore write failures — if the filesystem is read-only we have
    // bigger problems than missing shader compilation. The Rust code
    // still guards the include_bytes! behind the cfg flag below.
    let _ = std::fs::write(&metallib_path, &[]);
}
