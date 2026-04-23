//! Metal compute backend for the river Vector CFR inner loop.
//!
//! Per [`docs/HARDWARE.md`](../../../../../docs/HARDWARE.md) and
//! [`docs/LIMITING_FACTOR.md`](../../../../../docs/LIMITING_FACTOR.md), the
//! river regret-matching kernel at N=1326 is the hottest loop in the
//! whole solver. CPU SIMD (`wide::f32x8` via `matching_simd`) gets us
//! to ~1µs-per-call at N=1326 on an M1 Pro. Metal compute on the same
//! hardware expects **3–10× on top of that** because Apple Silicon's
//! unified memory gives us a zero-copy CPU↔GPU boundary.
//!
//! # When this module is compiled
//!
//! Only when the `metal` feature is enabled *and* the target is macOS.
//! The Rust-level `#[cfg(all(feature = "metal", target_os = "macos"))]`
//! gate is applied at the *re-export* site in `lib.rs`, not here — this
//! module is always compiled when the feature is on, and the macOS
//! gating happens via `[target.'cfg(target_os = "macos")'.dependencies]`
//! in `Cargo.toml`. On a Linux build with `--features metal`, the
//! `metal` and `objc2` crates are absent, so this module fails to
//! compile; `lib.rs` is responsible for not pulling it in on Linux.
//!
//! # Shader loading strategy
//!
//! The build script (`build.rs`) tries to compile
//! `shaders/regret_matching.metal` into a `.metallib` at build time
//! using `xcrun -sdk macosx metal`. If that succeeds, we
//! `include_bytes!` the compiled library (fast init, ~1ms). If the
//! build-time toolchain is missing (e.g. Henry's machine currently
//! lacks the downloadable Metal Toolchain component), `build.rs`
//! writes an empty file and sets `cfg(no_metallib_available)`; in that
//! case we fall back to compiling the shader from source at runtime
//! via `MTLDevice::newLibraryWithSource` (slower init, ~10–30ms, but
//! produces identical GPU code). The source is unconditionally embedded
//! via `include_str!` so the fallback path always has the source to
//! hand to Metal.
//!
//! This two-path strategy is why the task brief specified "do not
//! break the build under any circumstances" — the runtime-source path
//! works even on dev machines where the build-time toolchain is not
//! installed.
//!
//! # API
//!
//! - [`MetalContext`] — owns the Metal device, command queue, pipeline
//!   states, and scratch buffers. Create once, reuse across many
//!   regret-match calls. Not `Send` or `Sync` (Metal command queues
//!   require the same autoreleasepool scope for the thread that owns
//!   them).
//! - [`regret_match_metal`] — the dispatch entry point. Takes an
//!   input slice, writes to an output slice. Same semantics as
//!   [`crate::matching::regret_match`] within a 1e-4 tolerance.
//! - [`MetalError`] — opaque error wrapping Metal framework
//!   initialization failures. Callers should handle this by falling
//!   back to the SIMD/scalar path.

mod device;

pub use device::{regret_match_metal, MetalContext, MetalError};

/// The Metal Shading Language source for the regret-matching kernels.
///
/// Always embedded via `include_str!`, regardless of whether the
/// build-time shader compile succeeded. Used by the runtime fallback
/// path in [`device::MetalContext::new`] when
/// `cfg(no_metallib_available)` is set.
pub(crate) const SHADER_SOURCE: &str = include_str!("shaders/regret_matching.metal");
