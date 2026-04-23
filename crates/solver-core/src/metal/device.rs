//! Metal device handle and dispatch entry point.
//!
//! See `mod.rs` for the high-level overview. This file contains:
//!
//! - [`MetalContext`] — owns the device, queue, pipeline states, and
//!   reusable scratch buffers.
//! - [`regret_match_metal`] — public dispatch entry point.
//! - [`MetalError`] — the error type.

use std::cell::RefCell;

use metal::{
    CompileOptions, ComputePipelineState, Device, Function, Library, MTLResourceOptions, MTLSize,
    NSUInteger,
};
use objc::rc::autoreleasepool;

/// Errors surfaced by the Metal backend.
///
/// All of these are "fall back to scalar/SIMD" conditions from the
/// caller's perspective — none are programmer errors in solver code.
/// They indicate a runtime issue with the Metal framework (missing
/// device, compilation failure on exotic hardware, etc.) that the
/// caller should handle gracefully.
#[derive(Debug, thiserror::Error)]
pub enum MetalError {
    /// No Metal-capable device available on this machine. In practice
    /// this should never happen on Apple Silicon — every M-series Mac
    /// has an integrated GPU with Metal support. Can happen in CI
    /// containers running without GPU passthrough.
    #[error("no Metal-capable device found")]
    NoDevice,

    /// The embedded `.metallib` was empty or malformed. Typically
    /// means the build-time `xcrun metal` compile failed and the
    /// runtime-source fallback path was not attempted (bug).
    #[error("metal library loading failed: {0}")]
    LibraryLoad(String),

    /// Runtime shader compilation via `newLibraryWithSource` failed.
    /// The payload carries Metal's error message. This is the worst
    /// case — means our shader source has a syntax error that slipped
    /// past whatever testing produced the committed source. The
    /// equivalence test catches this immediately.
    #[error("metal shader compilation failed: {0}")]
    ShaderCompile(String),

    /// The named kernel function is missing from the compiled library.
    /// Same as `ShaderCompile` — indicates a mismatch between the
    /// expected kernel names and what's in the library. In normal
    /// operation this can't happen because we compile from sources
    /// that live in this repo; it would fire only if someone edited
    /// the `.metal` file in an incompatible way.
    #[error("metal kernel function not found: {0}")]
    MissingKernel(String),

    /// Creating a compute pipeline state from a loaded function failed.
    /// Indicates the kernel exists but the hardware refused to compile
    /// it — e.g. it uses features not supported by the current GPU.
    /// Should not happen on Apple Silicon for our kernels.
    #[error("metal pipeline state creation failed: {0}")]
    PipelineCreation(String),
}

/// Persistent Metal state owned across many regret-match calls.
///
/// Creation cost is ~10–30ms (shader compile + pipeline creation).
/// After that, each `regret_match_metal` dispatch is a handful of
/// buffer updates + encoder calls + a GPU wait. The context holds a
/// single persistent command queue (safe to reuse across command
/// buffers) plus a pool of scratch buffers sized to the largest seen
/// input.
///
/// # Thread safety
///
/// `MetalContext` is **not** `Send` or `Sync`. Metal command queues
/// should be owned by the thread that creates and uses them. For the
/// solver's use case this is fine — each `SolverHandle` owns its own
/// context, and rayon workers each get their own `SolverHandle`.
pub struct MetalContext {
    #[allow(dead_code)]
    device: Device,

    /// Single persistent command queue. Metal allows many command
    /// buffers to be created against the same queue; reusing the queue
    /// across calls is the documented pattern and avoids queue-create
    /// overhead on each dispatch.
    command_queue: metal::CommandQueue,

    /// Compute pipeline for `regret_matching_sum_kernel`.
    sum_pipeline: ComputePipelineState,
    /// Compute pipeline for `regret_matching_normalize_kernel`.
    normalize_pipeline: ComputePipelineState,

    /// Scratch buffer pool grows lazily to the largest seen input size.
    /// `RefCell` because `regret_match_metal` takes `&self` — the
    /// context is borrowed immutably but the buffer pool is mutated.
    buffer_pool: RefCell<BufferPool>,
}

/// Lazily-grown pool of shared-storage Metal buffers. We keep three
/// persistent buffers reused across calls:
///
/// - `regrets_buf`: input regrets, length >= n * sizeof(f32)
/// - `out_buf`: output strategy, length >= n * sizeof(f32)
/// - `sum_buf`: a single-f32 scratch for the atomic accumulator
///
/// They're allocated with `StorageModeShared` so the CPU writes
/// directly into the same memory the GPU reads. On Apple Silicon's
/// unified memory this is effectively zero-copy.
struct BufferPool {
    regrets_buf: Option<metal::Buffer>,
    out_buf: Option<metal::Buffer>,
    /// Single-f32 atomic-add accumulator. Allocated once, reset per
    /// call (we do the reset via CPU write since the buffer is
    /// shared storage).
    sum_buf: metal::Buffer,
    capacity: usize,
}

impl BufferPool {
    fn new(device: &Device) -> Self {
        // Single f32 sum buffer, never grows.
        let sum_buf = device.new_buffer(
            std::mem::size_of::<f32>() as u64,
            MTLResourceOptions::StorageModeShared,
        );
        Self {
            regrets_buf: None,
            out_buf: None,
            sum_buf,
            capacity: 0,
        }
    }

    /// Ensure the in/out buffers hold at least `n` f32s. Grows
    /// geometrically so repeated calls at the same size hit a stable
    /// allocation after the first.
    fn ensure_capacity(&mut self, device: &Device, n: usize) {
        if self.capacity >= n && self.regrets_buf.is_some() && self.out_buf.is_some() {
            return;
        }
        // Power-of-two growth. Avoids repeated alloc on increasing-n
        // call sequences.
        let new_cap = n.next_power_of_two().max(1326);
        let bytes = (new_cap * std::mem::size_of::<f32>()) as u64;
        self.regrets_buf = Some(device.new_buffer(bytes, MTLResourceOptions::StorageModeShared));
        self.out_buf = Some(device.new_buffer(bytes, MTLResourceOptions::StorageModeShared));
        self.capacity = new_cap;
    }
}

impl MetalContext {
    /// Initialize a new Metal context. Expensive (~10–30ms) — call
    /// once per solver handle, reuse across calls.
    ///
    /// Returns `Err` if the machine has no Metal device, if the
    /// embedded shader library fails to load, or if the pipeline
    /// states can't be built. In all error cases the caller should
    /// fall back to the SIMD or scalar regret-matching path.
    pub fn new() -> Result<Self, MetalError> {
        autoreleasepool(|| Self::new_impl())
    }

    fn new_impl() -> Result<Self, MetalError> {
        let device = Device::system_default().ok_or(MetalError::NoDevice)?;
        let command_queue = device.new_command_queue();
        let library = load_library(&device)?;

        let sum_pipeline = build_pipeline(&device, &library, "regret_matching_sum_kernel")?;
        let normalize_pipeline =
            build_pipeline(&device, &library, "regret_matching_normalize_kernel")?;

        let buffer_pool = RefCell::new(BufferPool::new(&device));

        Ok(Self {
            device,
            command_queue,
            sum_pipeline,
            normalize_pipeline,
            buffer_pool,
        })
    }
}

/// Load the Metal library, preferring the build-time-compiled
/// `.metallib` fast path when it's available, falling back to runtime
/// compilation from embedded source otherwise.
///
/// Both paths produce functionally identical `Library` handles. The
/// difference is purely startup speed.
fn load_library(device: &Device) -> Result<Library, MetalError> {
    // Embedded metallib bytes. If the build-time compile failed, this
    // is a zero-byte file and `cfg(no_metallib_available)` is set, so
    // we skip the metallib path entirely.
    //
    // `include_bytes!` requires a literal path. We use
    // `concat!(env!("OUT_DIR"), ...)` to reference the file the build
    // script wrote. Cargo sets OUT_DIR before compilation.
    #[cfg(not(no_metallib_available))]
    {
        const METALLIB_BYTES: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/regret_matching.metallib"));
        if !METALLIB_BYTES.is_empty() {
            return device
                .new_library_with_data(METALLIB_BYTES)
                .map_err(MetalError::LibraryLoad);
        }
        // Empty metallib — fall through to runtime compile.
    }

    // Runtime source-compile fallback. `CompileOptions::new()` uses
    // sensible defaults: Metal language version = matching the target
    // OS, fast_math_enabled = true (which is fine — regret matching
    // doesn't rely on strict IEEE behaviour).
    let options = CompileOptions::new();
    device
        .new_library_with_source(crate::metal::SHADER_SOURCE, &options)
        .map_err(MetalError::ShaderCompile)
}

/// Build a compute pipeline for a named kernel in a library.
///
/// This is where the hardware-level shader compilation actually
/// happens on Apple Silicon (the Metal library is an intermediate
/// format; the GPU driver compiles it to Apple GPU ISA on first
/// pipeline creation). Cost is ~1–5ms per pipeline.
fn build_pipeline(
    device: &Device,
    library: &Library,
    kernel_name: &str,
) -> Result<ComputePipelineState, MetalError> {
    let function: Function = library
        .get_function(kernel_name, None)
        .map_err(|_| MetalError::MissingKernel(kernel_name.to_string()))?;
    device
        .new_compute_pipeline_state_with_function(&function)
        .map_err(MetalError::PipelineCreation)
}

/// Run regret matching on the GPU.
///
/// Semantics match [`crate::matching::regret_match`] within a 1e-4
/// tolerance (see `tests/metal_equivalence.rs` for the property
/// sweep). Specifically:
///
/// - If `sum_i max(regrets[i], 0) > 0`: `out[i] = max(regrets[i], 0) / sum`
/// - Otherwise: `out[i] = 1 / regrets.len()`
///
/// NaN regrets are treated as `<= 0` (matching the scalar path).
///
/// # Panics
///
/// Panics on `regrets.len() != out.len()` and on empty inputs. Callers
/// should dispatch tiny inputs (n < ~100) to the scalar or SIMD path
/// directly — GPU dispatch overhead at that scale is net negative.
///
/// # Errors
///
/// Returns `MetalError` only on Metal framework failures (should be
/// rare in practice on Apple Silicon). On error, `out` is left in an
/// undefined state; the caller should use the SIMD/scalar fallback.
pub fn regret_match_metal(
    ctx: &MetalContext,
    regrets: &[f32],
    out: &mut [f32],
) -> Result<(), MetalError> {
    assert_eq!(
        regrets.len(),
        out.len(),
        "regret_match_metal: regrets and out must have the same length"
    );
    assert!(
        !regrets.is_empty(),
        "regret_match_metal: cannot operate on an empty action set"
    );

    let n = regrets.len();

    autoreleasepool(|| -> Result<(), MetalError> {
        let mut pool = ctx.buffer_pool.borrow_mut();
        pool.ensure_capacity(&ctx.device, n);

        // We just ensured the buffers exist; the unwraps are infallible.
        let regrets_buf = pool
            .regrets_buf
            .as_ref()
            .expect("regrets_buf must be allocated after ensure_capacity");
        let out_buf = pool
            .out_buf
            .as_ref()
            .expect("out_buf must be allocated after ensure_capacity");
        let sum_buf = &pool.sum_buf;

        // --- Stage the input ---------------------------------------------
        //
        // Shared-storage buffers expose their backing memory via
        // `.contents()`, so we can `memcpy` directly. This is zero-copy
        // on the GPU side (unified memory) but still costs a CPU write
        // on the host side — unavoidable given the input lives in
        // caller-owned memory.
        unsafe {
            let dst = regrets_buf.contents() as *mut f32;
            std::ptr::copy_nonoverlapping(regrets.as_ptr(), dst, n);
            // Reset the sum accumulator to zero. The atomic add in the
            // sum kernel accumulates from this starting point.
            let sum_dst = sum_buf.contents() as *mut f32;
            *sum_dst = 0.0f32;
        }

        // --- Build the command buffer ------------------------------------
        let command_buffer = ctx.command_queue.new_command_buffer();
        let encoder = command_buffer.new_compute_command_encoder();

        // Threadgroup width. 32 matches Apple Silicon's simdgroup
        // size, so each threadgroup is exactly one simdgroup. This
        // makes the `simd_sum` reduction in the sum kernel map cleanly
        // to a single hardware reduce instruction per threadgroup.
        let threadgroup_size = MTLSize::new(32, 1, 1);
        // ceil(n / 32) threadgroups. At n=1326, that's 42 groups.
        let threadgroup_count = MTLSize::new(
            ((n as NSUInteger) + threadgroup_size.width - 1) / threadgroup_size.width,
            1,
            1,
        );

        let length_u32: u32 = n as u32;

        // --- Dispatch 1: sum-of-positives --------------------------------
        encoder.set_compute_pipeline_state(&ctx.sum_pipeline);
        encoder.set_buffer(0, Some(regrets_buf), 0);
        encoder.set_buffer(1, Some(out_buf), 0);
        encoder.set_buffer(2, Some(sum_buf), 0);
        encoder.set_bytes(
            3,
            std::mem::size_of::<u32>() as u64,
            &length_u32 as *const u32 as *const std::ffi::c_void,
        );
        encoder.dispatch_thread_groups(threadgroup_count, threadgroup_size);

        // --- Dispatch 2: normalize ---------------------------------------
        //
        // Same encoder, same command buffer. The Metal runtime
        // schedules the two dispatches with the implicit dependency
        // order (second reads what first wrote), so no explicit
        // barrier is needed between them.
        encoder.set_compute_pipeline_state(&ctx.normalize_pipeline);
        encoder.set_buffer(0, Some(out_buf), 0);
        encoder.set_buffer(1, Some(sum_buf), 0);
        encoder.set_bytes(
            2,
            std::mem::size_of::<u32>() as u64,
            &length_u32 as *const u32 as *const std::ffi::c_void,
        );
        encoder.dispatch_thread_groups(threadgroup_count, threadgroup_size);

        encoder.end_encoding();
        command_buffer.commit();
        command_buffer.wait_until_completed();

        // --- Read the output ---------------------------------------------
        unsafe {
            let src = out_buf.contents() as *const f32;
            std::ptr::copy_nonoverlapping(src, out.as_mut_ptr(), n);
        }

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    //! Smoke tests. The heavy lifting lives in the integration test
    //! at `tests/metal_equivalence.rs`, which runs a 10k-trial random
    //! property sweep. These tests exist to catch dumb bugs at
    //! `cargo test -p solver-core --features metal --lib` so the full
    //! integration suite doesn't need to run every iteration.

    use super::*;
    use crate::matching::regret_match;

    fn try_ctx() -> Option<MetalContext> {
        MetalContext::new().ok()
    }

    #[test]
    fn single_spot_matches_scalar() {
        let Some(ctx) = try_ctx() else {
            eprintln!("skipping: no Metal device");
            return;
        };
        let regrets = [1.0f32, -2.0, 3.0, 0.0, -1.0, 2.0, -3.0, 4.0, 5.0];
        let mut out_metal = vec![0.0f32; regrets.len()];
        let mut out_scalar = vec![0.0f32; regrets.len()];
        regret_match_metal(&ctx, &regrets, &mut out_metal).unwrap();
        regret_match(&regrets, &mut out_scalar);
        for (i, (&m, &s)) in out_metal.iter().zip(out_scalar.iter()).enumerate() {
            assert!((m - s).abs() < 1e-4, "idx {i}: metal {m} vs scalar {s}");
        }
    }

    #[test]
    fn river_size_smoke() {
        let Some(ctx) = try_ctx() else {
            eprintln!("skipping: no Metal device");
            return;
        };
        // Deterministic fill — every other lane alternates positive
        // and negative. Exercises the normal (non-uniform-fallback)
        // branch at river scale.
        let regrets: Vec<f32> = (0..1326)
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect();
        let mut out_metal = vec![0.0f32; 1326];
        let mut out_scalar = vec![0.0f32; 1326];
        regret_match_metal(&ctx, &regrets, &mut out_metal).unwrap();
        regret_match(&regrets, &mut out_scalar);
        let sum: f32 = out_metal.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-3,
            "strategy should sum to 1, got {sum}"
        );
        for (i, (&m, &s)) in out_metal.iter().zip(out_scalar.iter()).enumerate() {
            assert!((m - s).abs() < 1e-4, "idx {i}: metal {m} vs scalar {s}");
        }
    }
}
