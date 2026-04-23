//! NEON intrinsic implementation of the showdown matmul row × vector.
//!
//! Gated on `target_arch = "aarch64"`. The fallback `wide`-based path
//! in `subgame_vector.rs` stays intact for x86 and scalar targets.
//!
//! # Why we hand-roll NEON here
//!
//! The row of the showdown matrix is `[i8; 1326]`. The `wide`-based
//! pre-NEON path (see `subgame_vector::showdown_matmul_rows`) widens
//! `i8 → f32` with a scalar lambda expression that the compiler
//! auto-vectorizes inconsistently — on aarch64 clang emits per-lane
//! unpacks instead of the single-instruction `SXTL2/SXTL` widening
//! chains that NEON provides. The gap between "auto-vectorized wide"
//! and "intrinsic-written NEON" is 2-3x on this kernel, which is what
//! we need to close the remaining 28% gap to the 300 ms @ 1000 iters
//! target on `river_canonical_spot`.
//!
//! # Two kernels
//!
//! * [`showdown_row_dot_neon`] — plain dot product `Σ_j sign[j] * w[j]`.
//!   Used as the correctness reference for the equivalence test and
//!   as a building block; a plain dot is sufficient when the matmul
//!   is rewritten to use signed coefficients.
//! * [`showdown_row_pos_neg_neon`] — the real kernel the existing
//!   showdown matmul needs. Returns `(pos, neg)` where
//!   `pos = Σ_j max(sign[j] * w[j], 0)` and
//!   `neg = Σ_j max(-sign[j] * w[j], 0)`.
//!   This matches `showdown_matmul_rows`'s per-row inner loop.

#![cfg(target_arch = "aarch64")]

use std::arch::aarch64::*;

use solver_eval::combo::NUM_COMBOS;

/// Compute `Σ_j sign_row[j] as f32 * weights[j]` using NEON intrinsics.
///
/// 16 lanes at a time: i8x16 load → s16x8 × 2 → s32x4 × 4 → f32x4 × 4.
///
/// Safe to call because the loads/stores use pointer arithmetic
/// bounded by the [`NUM_COMBOS`]-length guarantee. The tail (last
/// `NUM_COMBOS % 16` elements) falls back to scalar.
#[target_feature(enable = "neon")]
pub unsafe fn showdown_row_dot_neon(
    sign_row: &[i8; NUM_COMBOS],
    weights: &[f32; NUM_COMBOS],
) -> f32 {
    let mut acc = vdupq_n_f32(0.0); // f32x4 accumulator
    let mut i = 0usize;

    while i + 16 <= NUM_COMBOS {
        // Load 16 i8s
        let sign_i8 = vld1q_s8(sign_row.as_ptr().add(i));
        // Widen low 8 bytes to i16x8, then to i32x4 (low) and i32x4 (high)
        let lo_i16 = vmovl_s8(vget_low_s8(sign_i8));
        let hi_i16 = vmovl_s8(vget_high_s8(sign_i8));
        let lo_lo_i32 = vmovl_s16(vget_low_s16(lo_i16));
        let lo_hi_i32 = vmovl_s16(vget_high_s16(lo_i16));
        let hi_lo_i32 = vmovl_s16(vget_low_s16(hi_i16));
        let hi_hi_i32 = vmovl_s16(vget_high_s16(hi_i16));
        // Convert to f32
        let s0 = vcvtq_f32_s32(lo_lo_i32);
        let s1 = vcvtq_f32_s32(lo_hi_i32);
        let s2 = vcvtq_f32_s32(hi_lo_i32);
        let s3 = vcvtq_f32_s32(hi_hi_i32);
        // Load 4 f32x4 weight vectors
        let w0 = vld1q_f32(weights.as_ptr().add(i));
        let w1 = vld1q_f32(weights.as_ptr().add(i + 4));
        let w2 = vld1q_f32(weights.as_ptr().add(i + 8));
        let w3 = vld1q_f32(weights.as_ptr().add(i + 12));
        // Fused multiply-add: acc += s * w
        acc = vfmaq_f32(acc, s0, w0);
        acc = vfmaq_f32(acc, s1, w1);
        acc = vfmaq_f32(acc, s2, w2);
        acc = vfmaq_f32(acc, s3, w3);
        i += 16;
    }

    // Horizontal sum of the accumulator
    let mut sum = vaddvq_f32(acc);

    // Scalar tail for the last (NUM_COMBOS - 16*k) elements — up to 15
    while i < NUM_COMBOS {
        sum += sign_row[i] as f32 * weights[i];
        i += 1;
    }

    sum
}

/// Compute `(pos, neg)` where
///   `pos = Σ_j max(sign_row[j] as f32 * weights[j], 0)` and
///   `neg = Σ_j max(-sign_row[j] as f32 * weights[j], 0)`.
///
/// This is the inner loop of the row-major showdown matmul
/// (`showdown_matmul_rows` in `subgame_vector.rs`): because +1 and -1
/// sign entries get multiplied by *different* coefficients
/// (`win_coeff` for wins, `lose_coeff` for losses), the accumulator
/// must split at the per-lane sign. NEON `vmaxq_f32(x, 0)` and
/// `vmaxq_f32(-x, 0)` handle the split branch-free.
///
/// 16 lanes at a time: same i8→f32 widening as
/// [`showdown_row_dot_neon`], then `vfmaq_f32`-equivalent into pos/neg
/// accumulators via `vmaxq_f32` on the signed product.
#[target_feature(enable = "neon")]
pub unsafe fn showdown_row_pos_neg_neon(
    sign_row: &[i8; NUM_COMBOS],
    weights: &[f32; NUM_COMBOS],
) -> (f32, f32) {
    let zero = vdupq_n_f32(0.0);
    let mut pos_acc = vdupq_n_f32(0.0);
    let mut neg_acc = vdupq_n_f32(0.0);
    let mut i = 0usize;

    while i + 16 <= NUM_COMBOS {
        // Load + widen 16 i8 signs → 4 × f32x4
        let sign_i8 = vld1q_s8(sign_row.as_ptr().add(i));
        let lo_i16 = vmovl_s8(vget_low_s8(sign_i8));
        let hi_i16 = vmovl_s8(vget_high_s8(sign_i8));
        let s0 = vcvtq_f32_s32(vmovl_s16(vget_low_s16(lo_i16)));
        let s1 = vcvtq_f32_s32(vmovl_s16(vget_high_s16(lo_i16)));
        let s2 = vcvtq_f32_s32(vmovl_s16(vget_low_s16(hi_i16)));
        let s3 = vcvtq_f32_s32(vmovl_s16(vget_high_s16(hi_i16)));

        let w0 = vld1q_f32(weights.as_ptr().add(i));
        let w1 = vld1q_f32(weights.as_ptr().add(i + 4));
        let w2 = vld1q_f32(weights.as_ptr().add(i + 8));
        let w3 = vld1q_f32(weights.as_ptr().add(i + 12));

        // Signed products
        let rs0 = vmulq_f32(s0, w0);
        let rs1 = vmulq_f32(s1, w1);
        let rs2 = vmulq_f32(s2, w2);
        let rs3 = vmulq_f32(s3, w3);

        // pos_acc += max(rs, 0)
        pos_acc = vaddq_f32(pos_acc, vmaxq_f32(rs0, zero));
        pos_acc = vaddq_f32(pos_acc, vmaxq_f32(rs1, zero));
        pos_acc = vaddq_f32(pos_acc, vmaxq_f32(rs2, zero));
        pos_acc = vaddq_f32(pos_acc, vmaxq_f32(rs3, zero));

        // neg_acc += max(-rs, 0)
        neg_acc = vaddq_f32(neg_acc, vmaxq_f32(vnegq_f32(rs0), zero));
        neg_acc = vaddq_f32(neg_acc, vmaxq_f32(vnegq_f32(rs1), zero));
        neg_acc = vaddq_f32(neg_acc, vmaxq_f32(vnegq_f32(rs2), zero));
        neg_acc = vaddq_f32(neg_acc, vmaxq_f32(vnegq_f32(rs3), zero));

        i += 16;
    }

    let mut pos = vaddvq_f32(pos_acc);
    let mut neg = vaddvq_f32(neg_acc);

    while i < NUM_COMBOS {
        let rs = sign_row[i] as f32 * weights[i];
        if rs > 0.0 {
            pos += rs;
        } else if rs < 0.0 {
            neg += -rs;
        }
        i += 1;
    }

    (pos, neg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};
    use rand_xoshiro::Xoshiro256PlusPlus;

    fn scalar_dot(sign: &[i8; NUM_COMBOS], w: &[f32; NUM_COMBOS]) -> f32 {
        let mut sum = 0.0f32;
        for i in 0..NUM_COMBOS {
            sum += sign[i] as f32 * w[i];
        }
        sum
    }

    fn scalar_pos_neg(sign: &[i8; NUM_COMBOS], w: &[f32; NUM_COMBOS]) -> (f32, f32) {
        let mut pos = 0.0f32;
        let mut neg = 0.0f32;
        for i in 0..NUM_COMBOS {
            let rs = sign[i] as f32 * w[i];
            if rs > 0.0 {
                pos += rs;
            } else if rs < 0.0 {
                neg += -rs;
            }
        }
        (pos, neg)
    }

    fn make_inputs(
        rng: &mut Xoshiro256PlusPlus,
    ) -> (Box<[i8; NUM_COMBOS]>, Box<[f32; NUM_COMBOS]>) {
        let mut sign = Box::new([0i8; NUM_COMBOS]);
        let mut w = Box::new([0.0f32; NUM_COMBOS]);
        for i in 0..NUM_COMBOS {
            sign[i] = rng.gen_range(-1..=1);
            w[i] = rng.gen_range(-1.0..1.0);
        }
        (sign, w)
    }

    #[test]
    fn neon_dot_matches_scalar_on_random_inputs() {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(42);

        for trial in 0..100 {
            let (sign, w) = make_inputs(&mut rng);
            let neon_result = unsafe { showdown_row_dot_neon(&sign, &w) };
            let scalar_result = scalar_dot(&sign, &w);
            // Accumulation order differs between NEON (4 parallel lanes +
            // horizontal add + FMA) and scalar (left-to-right). With
            // weights in [-1, 1] and 1326 lanes, cumulative rounding
            // drift stays well under 1e-3.
            assert!(
                (neon_result - scalar_result).abs() < 1e-3,
                "trial {trial}: neon={neon_result}, scalar={scalar_result}"
            );
        }
    }

    #[test]
    fn neon_pos_neg_matches_scalar_on_random_inputs() {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(7);

        for trial in 0..100 {
            let (sign, w) = make_inputs(&mut rng);
            let (np, nn) = unsafe { showdown_row_pos_neg_neon(&sign, &w) };
            let (sp, sn) = scalar_pos_neg(&sign, &w);
            assert!(
                (np - sp).abs() < 1e-3 && (nn - sn).abs() < 1e-3,
                "trial {trial}: neon=({np}, {nn}), scalar=({sp}, {sn})"
            );
        }
    }

    /// Sanity check that the tail loop fires for inputs with all-zero
    /// sign but a tail mismatch — the "all 16-chunk" fast path must
    /// not drop the remainder elements.
    #[test]
    fn neon_handles_tail_exactly() {
        // Put a marker in the final tail elements so the full-chunk
        // path cannot cover them: NUM_COMBOS = 1326 = 82 * 16 + 14, so
        // indices 1312..1326 are the scalar tail.
        let mut sign = Box::new([0i8; NUM_COMBOS]);
        let mut w = Box::new([0.0f32; NUM_COMBOS]);
        for i in (NUM_COMBOS - 14)..NUM_COMBOS {
            sign[i] = 1;
            w[i] = 2.0;
        }
        let dot = unsafe { showdown_row_dot_neon(&sign, &w) };
        assert!((dot - 28.0).abs() < 1e-6, "expected 14*2=28, got {dot}");

        let (pos, neg) = unsafe { showdown_row_pos_neg_neon(&sign, &w) };
        assert!(
            (pos - 28.0).abs() < 1e-6 && neg == 0.0,
            "expected (28, 0), got ({pos}, {neg})"
        );
    }
}
