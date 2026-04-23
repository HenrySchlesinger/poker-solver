// Regret-matching compute shaders for the river Vector CFR inner loop.
//
// Given an array `regrets[length]`, produce `out[length]` such that
//   if sum_i max(regrets[i], 0) > 0:
//     out[i] = max(regrets[i], 0) / sum_i max(regrets[i], 0)
//   else:
//     out[i] = 1.0 / length
//
// This matches the semantics of `regret_match` in src/matching.rs and
// `regret_match_simd` in src/matching_simd.rs. The Metal path must
// match scalar/SIMD within 1e-4 per the equivalence test.
//
// Dispatch model
// --------------
// We use two kernels in the same command buffer:
//
//   1. regret_matching_sum_kernel   — computes sum_positive and
//                                     writes max(r, 0) into `out` as
//                                     scratch. `sum` buffer holds a
//                                     single f32 accumulator.
//   2. regret_matching_normalize_kernel — reads `sum`, divides `out[i]`
//                                         by it (or fills uniform if
//                                         sum <= 0).
//
// Both dispatch one thread per element. At length=1326 with
// threadgroup_size=32, that's ceil(1326/32) = 42 threadgroups per
// dispatch. The two dispatches are queued on the same command buffer
// so the GPU pipelines them without host involvement.
//
// Why a threadgroup reduction instead of a raw atomic per thread:
// At length 1326, per-thread atomic_fetch_add is ~1326 contended ops
// per regret_match. With a simdgroup (32-thread) reduce + one atomic
// per simdgroup, we go down to ~42 atomics total. Lower contention,
// same result to the last ulp (we still use the same summation order
// modulo simdgroup-local fma, which is within the 1e-4 equivalence
// tolerance).
//
// Atomic f32 is available on all M-series devices via
// atomic_fetch_add_explicit on `atomic<float>` (Metal 3.0+). Apple
// Silicon is the only target; no need for the u32-CAS workaround.

#include <metal_stdlib>
#include <metal_atomic>
#include <metal_simdgroup>

using namespace metal;

// Sum-of-positives kernel.
//
// Each thread reads one regret value, clamps it to >= 0, writes the
// clamped value into `out` (used as scratch for the second pass), and
// participates in a simdgroup reduction so that one thread per
// simdgroup performs the atomic-add into the global sum.
kernel void regret_matching_sum_kernel(
    device const float *regrets    [[ buffer(0) ]],
    device float       *out        [[ buffer(1) ]],
    device atomic_float *sum       [[ buffer(2) ]],
    constant uint      &length     [[ buffer(3) ]],
    uint tid                       [[ thread_position_in_grid ]],
    uint simd_lane_id              [[ thread_index_in_simdgroup ]]
) {
    float contribution = 0.0f;
    if (tid < length) {
        float r = regrets[tid];
        // NaN > 0 is false in IEEE, matching the scalar path which
        // tests `r > 0.0` (NaN contributes 0). `r > 0` here reproduces
        // that behaviour bit-for-bit.
        float positive = (r > 0.0f) ? r : 0.0f;
        out[tid] = positive;
        contribution = positive;
    }

    // Simdgroup-wide reduction: every lane in this simdgroup contributes
    // to `lane_sum`. simd_sum is a broadcast reduction — every lane in
    // the simdgroup gets the same result.
    float lane_sum = simd_sum(contribution);

    // Exactly one thread per simdgroup posts the partial to the global
    // accumulator. 1326 threads / 32 per simdgroup = ~42 atomic ops
    // total, acceptable contention.
    if (simd_lane_id == 0) {
        atomic_fetch_add_explicit(sum, lane_sum, memory_order_relaxed);
    }
}

// Normalize kernel.
//
// After the first dispatch completes, `out` contains the element-wise
// positive part of `regrets`, and `sum` contains the total sum of
// positives. Each thread normalizes one element:
//   - If sum > 0: out[i] = out[i] / sum
//   - Otherwise:  out[i] = 1.0 / length  (uniform fallback)
//
// This exactly matches the scalar path's two-branch structure.
kernel void regret_matching_normalize_kernel(
    device float              *out     [[ buffer(0) ]],
    device const atomic_float *sum     [[ buffer(1) ]],
    constant uint             &length  [[ buffer(2) ]],
    uint tid                           [[ thread_position_in_grid ]]
) {
    if (tid >= length) {
        return;
    }

    float total = atomic_load_explicit(sum, memory_order_relaxed);
    if (total > 0.0f) {
        float inv = 1.0f / total;
        out[tid] = out[tid] * inv;
    } else {
        out[tid] = 1.0f / float(length);
    }
}
