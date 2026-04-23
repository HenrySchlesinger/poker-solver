//! Range: a 1326-wide weight vector over hole-card combos.
//!
//! Implementation: `[f32; 1326]` with a canonical ordering.
//! Combo index = combo encoding defined in `solver-eval::combo`.
//!
//! The parser accepts standard notation:
//!   - "AA"       — pocket aces (all 6 combos, weight 1.0)
//!   - "AKs"      — A-K suited (4 combos)
//!   - "AKo"      — A-K offsuit (12 combos)
//!   - "AK"       — A-K any (16 combos)
//!   - "T9s+"     — T9s, J9s, Q9s, K9s, A9s
//!   - "22+"      — all pocket pairs
//!   - "88-TT"    — pocket pairs 88 through TT
//!
//! Comma-separated: "AA, KK, AKs, T9s+".

#[allow(dead_code)]
const NUM_COMBOS: usize = 1326;

/// 1326-wide weight vector over NLHE hole-card combos.
///
/// Index 0..1326 maps to specific combos (see `solver-eval::combo` for the
/// encoding). Weight is in [0, 1]: 1.0 means "always this hand," 0.5
/// means "half the time," 0 means "never."
// TODO (Day 1, agent A3): implement.
#[derive(Clone)]
pub struct Range {
    /// Weights per combo.
    pub weights: Box<[f32; NUM_COMBOS]>,
}

impl Range {
    /// Uniform range (all 1326 combos at weight 1.0).
    // TODO: implement.
    pub fn full() -> Self {
        Self { weights: Box::new([1.0; NUM_COMBOS]) }
    }

    /// Empty range (all weights 0).
    pub fn empty() -> Self {
        Self { weights: Box::new([0.0; NUM_COMBOS]) }
    }

    /// Parse a range from standard notation.
    ///
    /// Returns an error on malformed input.
    // TODO (Day 1, agent A3): implement.
    pub fn parse(_s: &str) -> Result<Self, RangeParseError> {
        todo!()
    }

    /// Total weight (number of combos × per-combo weight).
    pub fn total_weight(&self) -> f32 {
        self.weights.iter().sum()
    }
}

/// Errors from `Range::parse`.
#[derive(Debug, thiserror::Error)]
pub enum RangeParseError {
    /// Unknown token in the range string.
    #[error("unknown token: {0}")]
    UnknownToken(String),
    /// Malformed pair specifier.
    #[error("bad pair: {0}")]
    BadPair(String),
}
