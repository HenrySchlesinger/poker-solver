//! SIMD-vectorized regret matching — the river hot-path optimization.
//!
//! **Stub commit.** This first commit exists only to let
//! `tests/simd_equivalence.rs` compile and run against the scalar
//! reference. The body here just forwards to
//! [`crate::matching::regret_match`]. The follow-up commit replaces this
//! body with a real `wide::f32x8` implementation and the test sticks
//! around unchanged — which is the whole point of the test-first order:
//! prove the test would catch a regression before introducing the thing
//! it's meant to police.
//!
//! Why `wide` and not `std::simd`: `std::simd` (the `portable_simd`
//! feature, tracking issue #86656) is still nightly-only on the pinned
//! 1.75+ stable toolchain this workspace uses. Henry's machine runs
//! stable; no nightly. `wide = "0.7"` gives us a portable 8-wide
//! `f32x8` type that maps to NEON on Apple Silicon and to AVX2 on x86
//! under `#[cfg(target_feature = "avx")]`, with a safe scalar fallback
//! elsewhere. That's everything this crate needs.
//!
//! See also: `docs/LIMITING_FACTOR.md` for why the river inner loop is
//! THE critical path, and `benches/simd_matching.rs` for the measured
//! speedup numbers.

use crate::matching::regret_match;

/// SIMD f32 regret matching. Equivalent to [`crate::matching::regret_match`]
/// within f32 rounding. Requires `regrets.len() == out.len()`; both must be
/// non-empty. No length restriction (handles tails scalar).
///
/// # Current status
///
/// Temporarily forwards to the scalar path so the equivalence test can be
/// committed and exercised before the real SIMD body lands in the next
/// commit. The public signature and semantics are final.
pub fn regret_match_simd(regrets: &[f32], out: &mut [f32]) {
    regret_match(regrets, out);
}
