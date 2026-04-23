//! Combo indexing: mapping between `(Card, Card)` pairs and 0..1326.
//!
//! The canonical ordering is: for cards `a < b` (by u8 value),
//! `combo_index(a, b) = a * 51 - (a * (a+1) / 2) + b - a - 1` (or
//! equivalent). This gives 1326 distinct values in the order expected by
//! `solver-nlhe::Range::weights`.

// TODO (Day 1, agent A1): document exact formula, write the two functions
// and property-test that they round-trip.
//
// pub const NUM_COMBOS: usize = 1326;
//
// pub fn combo_index(a: Card, b: Card) -> usize
// pub fn index_to_combo(idx: usize) -> (Card, Card)
