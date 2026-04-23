//! Card / board isomorphism.
//!
//! Canonical-renaming of suits so that strategically-equivalent boards
//! map to the same key. Reduces the flop-cache size by ~12× on average,
//! more on paired/flushy boards.
//!
//! Scheme: walk the board (and optionally the hero/villain hands)
//! in deal order, assigning suit labels 0, 1, 2, 3 to each new suit
//! as it appears. The canonical board is the result of that relabeling.

use crate::board::Board;

/// Return the canonicalized form of `board`.
// TODO (Day 1, agent A4): implement.
pub fn canonical_board(_board: &Board) -> Board {
    todo!()
}

/// Full canonicalization including hero + villain hole cards. Used for
/// cache lookup: two spots with the same canonical representation have
/// identical strategies.
// TODO (Day 2, agent A4): implement.
pub fn canonical_spot(
    _board: &Board,
    _hero_combo_idx: u16,
    _villain_combo_idx: u16,
) -> (Board, u16, u16) {
    todo!()
}
