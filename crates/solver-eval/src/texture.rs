//! Board-texture bucketing.
//!
//! Two boards are "the same texture" if they play similarly. The texture
//! bucket is a key for the flop cache when we don't want the full
//! (1,755 canonical flops) resolution — e.g., for warm-starting.
//!
//! Features that matter:
//! - paired / unpaired / trips
//! - suited / rainbow / monotone
//! - connectedness (straight-draw potential)
//! - high-card rank bucket

use crate::board::Board;

/// Compact texture identifier. Opaque; use equality and hashing only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct TextureBucket(pub u16);

/// Compute the texture bucket for a board.
// TODO (Day 5, agent A2): implement once cache format is designed.
pub fn texture_of(_board: &Board) -> TextureBucket {
    todo!()
}
