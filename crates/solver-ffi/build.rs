//! Build script for `solver-ffi`.
//!
//! Regenerates `crates/solver-ffi/include/solver.h` from `src/lib.rs`
//! on every build, using the config in `cbindgen.toml`.
//!
//! Philosophy: **header generation must not be a hard build dependency.**
//! If cbindgen fails for any reason (missing in a minimal CI environment,
//! parse error during a work-in-progress edit, IO error on a locked-down
//! filesystem), we emit a `cargo:warning=` and let the Rust lib itself
//! continue to compile. A stale header on disk is recoverable; a repo
//! that won't `cargo build` at all is not.

use std::env;
use std::path::PathBuf;

fn main() {
    // Rerun only when the things that can change the generated header
    // actually change. cbindgen does not need to rerun for every edit
    // to an unrelated file in the workspace.
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=build.rs");
    // Also rerun if the output header is missing (e.g. a contributor
    // deleted `include/solver.h` without doing a full `cargo clean`).
    // `rerun-if-changed` on a non-existent path fires on every build,
    // which is exactly what we want as the "please regenerate" trigger.
    println!("cargo:rerun-if-changed=include/solver.h");

    let crate_dir = match env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(e) => {
            println!(
                "cargo:warning=solver-ffi build.rs: CARGO_MANIFEST_DIR unset ({e}); skipping header generation"
            );
            return;
        }
    };

    let include_dir = crate_dir.join("include");
    if let Err(e) = std::fs::create_dir_all(&include_dir) {
        println!(
            "cargo:warning=solver-ffi build.rs: could not create {}: {e}; skipping header generation",
            include_dir.display()
        );
        return;
    }
    let header_path = include_dir.join("solver.h");

    // cbindgen's builder API returns a Result at every step. We log and
    // skip on any failure rather than aborting the whole build. The
    // generated header is checked into the repo, so downstream consumers
    // are never left header-less — worst case they get a stale one that
    // was last regenerated successfully.
    let config_path = crate_dir.join("cbindgen.toml");
    let config = match cbindgen::Config::from_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            println!(
                "cargo:warning=solver-ffi build.rs: could not read {}: {e}; skipping header generation",
                config_path.display()
            );
            return;
        }
    };

    let bindings = match cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    {
        Ok(b) => b,
        Err(e) => {
            println!(
                "cargo:warning=solver-ffi build.rs: cbindgen failed to parse crate: {e}; skipping header generation"
            );
            return;
        }
    };

    // `write_to_file` returns `true` if the file was (re)written, `false`
    // if the on-disk contents already matched. Both are success — we just
    // avoid bumping the mtime needlessly, which would otherwise retrigger
    // every downstream consumer that depends on `solver.h`.
    let _wrote = bindings.write_to_file(&header_path);
}
