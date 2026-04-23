//! NEON intrinsic implementation of the showdown matmul row × vector.
//!
//! Gated on `target_arch = "aarch64"`. The fallback `wide`-based path
//! in `subgame_vector.rs` stays intact for x86 and scalar targets.

#![cfg(target_arch = "aarch64")]

use std::arch::aarch64::*;

// Implementation follows in a subsequent commit.
