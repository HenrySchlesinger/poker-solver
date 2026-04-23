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
//!   - "T9s+"     — T9s, J9s, Q9s, K9s, A9s (second card fixed, first
//!     card varies up to Ace). `QTs+` = QTs, KTs, ATs.
//!   - "22+"      — all pocket pairs 22 through AA
//!   - "88-TT"    — pocket pairs 88 through TT (inclusive)
//!   - "JJ-"      — all pocket pairs ≤ JJ
//!   - "AA:0.5"   — optional weight suffix applies to every combo in the
//!     token (not a random subset).
//!
//! Comma-separated: "AA, KK, AKs, T9s+".
//!
//! Later tokens overwrite earlier ones for the same combo (last write wins).
//! This matters for e.g. `"AK, AKs:0.5"` — the AKs combos end up at 0.5.

use std::num::ParseFloatError;

/// Number of unique NLHE hole-card combos (C(52, 2)).
const NUM_COMBOS: usize = 1326;

/// 1326-wide weight vector over NLHE hole-card combos.
///
/// Index 0..1326 maps to specific combos (see `solver-eval::combo` for the
/// encoding). Weight is in [0, 1]: 1.0 means "always this hand," 0.5
/// means "half the time," 0 means "never."
#[derive(Clone)]
pub struct Range {
    /// Weights per combo.
    pub weights: Box<[f32; NUM_COMBOS]>,
}

impl Range {
    /// Uniform range (all 1326 combos at weight 1.0).
    pub fn full() -> Self {
        Self {
            weights: Box::new([1.0; NUM_COMBOS]),
        }
    }

    /// Empty range (all weights 0).
    pub fn empty() -> Self {
        Self {
            weights: Box::new([0.0; NUM_COMBOS]),
        }
    }

    /// Parse a range from standard notation.
    ///
    /// Returns an error on malformed input. See module docs for the
    /// supported grammar. Whitespace around tokens is allowed; tokens are
    /// separated by commas.
    pub fn parse(s: &str) -> Result<Self, RangeParseError> {
        let mut r = Self::empty();
        for raw in s.split(',') {
            let token = raw.trim();
            if token.is_empty() {
                continue;
            }
            apply_token(&mut r.weights, token)?;
        }
        Ok(r)
    }

    /// Total weight summed across all 1326 combos.
    pub fn total_weight(&self) -> f32 {
        self.weights.iter().sum()
    }
}

/// Errors from `Range::parse`.
#[derive(Debug, thiserror::Error)]
pub enum RangeParseError {
    /// Unknown or malformed token in the range string.
    #[error("unknown token: {0}")]
    UnknownToken(String),
    /// Malformed pair specifier (e.g., "A2-KK" mixes pair and non-pair).
    #[error("bad pair: {0}")]
    BadPair(String),
    /// Malformed weight suffix (after the ':').
    #[error("bad weight in token {token:?}: {source}")]
    BadWeight {
        /// The offending token.
        token: String,
        /// The underlying float-parse error.
        #[source]
        source: ParseFloatError,
    },
}

// ---------------------------------------------------------------------------
// Token application
// ---------------------------------------------------------------------------

/// Apply a single (already-trimmed, non-empty) token to the weights array.
fn apply_token(weights: &mut [f32; NUM_COMBOS], raw: &str) -> Result<(), RangeParseError> {
    // Split off optional weight suffix ":<float>".
    let (body, weight) = match raw.rsplit_once(':') {
        Some((b, w)) => {
            let parsed: f32 = w
                .trim()
                .parse()
                .map_err(|source| RangeParseError::BadWeight {
                    token: raw.to_string(),
                    source,
                })?;
            (b.trim(), parsed)
        }
        None => (raw, 1.0),
    };
    if body.is_empty() {
        return Err(RangeParseError::UnknownToken(raw.to_string()));
    }

    // Classify: is this a pair token (both ranks equal) or a two-rank token?
    let (first_rank, second_rank) =
        read_two_ranks(body).ok_or_else(|| RangeParseError::UnknownToken(raw.to_string()))?;

    if first_rank == second_rank {
        apply_pair_token(weights, first_rank, body, weight, raw)
    } else {
        apply_two_rank_token(weights, first_rank, second_rank, body, weight, raw)
    }
}

/// Read the two leading rank letters from a token body. Returns
/// `(first, second)` as rank values in 0..13 (Two=0, Ace=12), or `None`
/// if the token doesn't start with two valid rank letters.
fn read_two_ranks(s: &str) -> Option<(u8, u8)> {
    let bytes = s.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let a = rank_from_char(bytes[0] as char)?;
    let b = rank_from_char(bytes[1] as char)?;
    Some((a, b))
}

/// Map a rank character ('2'..'9', 'T', 'J', 'Q', 'K', 'A') to 0..13.
/// Case-insensitive for letter ranks.
fn rank_from_char(c: char) -> Option<u8> {
    match c {
        '2' => Some(0),
        '3' => Some(1),
        '4' => Some(2),
        '5' => Some(3),
        '6' => Some(4),
        '7' => Some(5),
        '8' => Some(6),
        '9' => Some(7),
        'T' | 't' => Some(8),
        'J' | 'j' => Some(9),
        'Q' | 'q' => Some(10),
        'K' | 'k' => Some(11),
        'A' | 'a' => Some(12),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Pair tokens: "AA", "22+", "JJ-", "88-TT"
// ---------------------------------------------------------------------------

/// Handle tokens where the two leading ranks are equal (i.e. pocket pairs).
fn apply_pair_token(
    weights: &mut [f32; NUM_COMBOS],
    rank: u8,
    body: &str,
    weight: f32,
    raw: &str,
) -> Result<(), RangeParseError> {
    // Body starts with "RR"; inspect what comes after.
    let tail = &body[2..];
    match tail {
        "" => {
            // "AA" — single pocket pair.
            set_pair(weights, rank, weight);
        }
        "+" => {
            // "22+" — all pairs ≥ rank.
            for r in rank..=12 {
                set_pair(weights, r, weight);
            }
        }
        "-" => {
            // "JJ-" — all pairs ≤ rank.
            for r in 0..=rank {
                set_pair(weights, r, weight);
            }
        }
        _ if tail.starts_with('-') => {
            // "88-TT" — inclusive pair range.
            let upper = &tail[1..];
            let (hi_a, hi_b) =
                read_two_ranks(upper).ok_or_else(|| RangeParseError::BadPair(raw.to_string()))?;
            if hi_a != hi_b || upper.len() != 2 {
                return Err(RangeParseError::BadPair(raw.to_string()));
            }
            let (lo, hi) = if rank <= hi_a {
                (rank, hi_a)
            } else {
                (hi_a, rank)
            };
            for r in lo..=hi {
                set_pair(weights, r, weight);
            }
        }
        _ => {
            return Err(RangeParseError::UnknownToken(raw.to_string()));
        }
    }
    Ok(())
}

/// Set all 6 combos of pocket pair `rank` to `weight`.
fn set_pair(weights: &mut [f32; NUM_COMBOS], rank: u8, weight: f32) {
    // A pair has 6 combos: the 4-choose-2 suit pairs.
    for s1 in 0u8..4 {
        for s2 in (s1 + 1)..4 {
            let a = card_u8(rank, s1);
            let b = card_u8(rank, s2);
            weights[combo_index(a, b)] = weight;
        }
    }
}

// ---------------------------------------------------------------------------
// Two-rank tokens: "AKs", "AKo", "AK", "T9s+", "QTs+", "AT-KT" (future)
// ---------------------------------------------------------------------------

/// Suitedness constraint for a two-rank token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Suitedness {
    Suited,
    Offsuit,
    Any,
}

/// Handle tokens where the two leading ranks differ (i.e., non-pair hands).
///
/// `first` must be the higher rank and `second` the lower rank for the
/// standard-notation ordering. We reject the reverse (e.g. "2A") because
/// that's non-idiomatic and often the sign of a typo.
fn apply_two_rank_token(
    weights: &mut [f32; NUM_COMBOS],
    first: u8,
    second: u8,
    body: &str,
    weight: f32,
    raw: &str,
) -> Result<(), RangeParseError> {
    if first < second {
        // Reject "2A", "9T", etc. Standard notation is high-low.
        return Err(RangeParseError::UnknownToken(raw.to_string()));
    }

    let tail = &body[2..];
    // Peel off optional suitedness flag.
    let (suited, rest) = if let Some(r) = tail.strip_prefix('s').or_else(|| tail.strip_prefix('S'))
    {
        (Suitedness::Suited, r)
    } else if let Some(r) = tail.strip_prefix('o').or_else(|| tail.strip_prefix('O')) {
        (Suitedness::Offsuit, r)
    } else {
        (Suitedness::Any, tail)
    };

    match rest {
        "" => {
            // "AKs" / "AKo" / "AK" — single pair of ranks.
            set_two_rank(weights, first, second, suited, weight);
        }
        "+" => {
            // "T9s+" — second rank fixed, first rank iterates up to Ace.
            // Example: "T9s+" = T9s, J9s, Q9s, K9s, A9s.
            for f in first..=12 {
                if f == second {
                    // Shouldn't happen since first > second initially and
                    // second is fixed — but guard against it.
                    continue;
                }
                set_two_rank(weights, f, second, suited, weight);
            }
        }
        _ => {
            return Err(RangeParseError::UnknownToken(raw.to_string()));
        }
    }
    Ok(())
}

/// Set combos for a single (first, second) rank pair with the given
/// suitedness constraint.
fn set_two_rank(
    weights: &mut [f32; NUM_COMBOS],
    first: u8,
    second: u8,
    suited: Suitedness,
    weight: f32,
) {
    // Two different ranks → 16 combos total (4 suited + 12 offsuit).
    for s1 in 0u8..4 {
        for s2 in 0u8..4 {
            let is_suited = s1 == s2;
            let include = match suited {
                Suitedness::Suited => is_suited,
                Suitedness::Offsuit => !is_suited,
                Suitedness::Any => true,
            };
            if !include {
                continue;
            }
            let a = card_u8(first, s1);
            let b = card_u8(second, s2);
            weights[combo_index(a, b)] = weight;
        }
    }
}

// ---------------------------------------------------------------------------
// Local combo-index helper
// ---------------------------------------------------------------------------
//
// TODO (agent A1): swap these to `solver_eval::combo::{combo_index, ...}`
// once A1 lands the canonical implementation. The math below matches the
// documented canonical ordering (lexicographic over unordered pairs
// `a < b`, both in 0..52); swapping is a drop-in.

/// Encode `(rank, suit)` as the Card u8 layout used elsewhere in the repo
/// (see `solver-eval::card::Card::new`). We don't construct `Card` here to
/// avoid coupling the parser to types we don't control.
#[inline]
fn card_u8(rank: u8, suit: u8) -> u8 {
    (rank << 2) | (suit & 0b11)
}

/// Lexicographic index of the unordered pair `{a, b}` in 0..1326.
///
/// Formula for `a < b`, both in 0..52:
///   `a * (2*52 - a - 1) / 2 + (b - a - 1)`
///
/// Which is equivalent to "sum of row widths above row `a` in a strictly
/// upper-triangular enumeration of the 52×52 grid, plus column offset."
#[inline]
fn combo_index(a: u8, b: u8) -> usize {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let lo = lo as usize;
    let hi = hi as usize;
    // n = 52
    lo * (2 * 52 - lo - 1) / 2 + (hi - lo - 1)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Count combos whose weight is near `w` (within 1e-4).
    fn count_near(r: &Range, w: f32) -> usize {
        r.weights.iter().filter(|&&x| (x - w).abs() < 1e-4).count()
    }

    /// Count combos whose weight is > 0.
    fn count_nonzero(r: &Range) -> usize {
        r.weights.iter().filter(|&&x| x > 0.0).count()
    }

    #[test]
    fn combo_index_is_lexicographic_and_covers_1326() {
        // Exhaustive: every unordered pair 0..52 hits a unique index in 0..1326.
        let mut seen = [false; 1326];
        for a in 0u8..52 {
            for b in (a + 1)..52 {
                let idx = combo_index(a, b);
                assert!(idx < 1326, "idx {idx} out of range for ({a},{b})");
                assert!(!seen[idx], "duplicate index {idx} at ({a},{b})");
                seen[idx] = true;
                // Order-independence.
                assert_eq!(combo_index(a, b), combo_index(b, a));
            }
        }
        assert!(seen.iter().all(|&x| x));
    }

    #[test]
    fn full_and_empty() {
        assert_eq!(Range::full().total_weight(), 1326.0);
        assert_eq!(Range::empty().total_weight(), 0.0);
    }

    #[test]
    fn parse_pocket_aces() {
        let r = Range::parse("AA").unwrap();
        assert_eq!(count_near(&r, 1.0), 6);
        assert_eq!(count_nonzero(&r), 6);
    }

    #[test]
    fn parse_aks_suited() {
        let r = Range::parse("AKs").unwrap();
        assert_eq!(count_near(&r, 1.0), 4);
    }

    #[test]
    fn parse_ako_offsuit() {
        let r = Range::parse("AKo").unwrap();
        assert_eq!(count_near(&r, 1.0), 12);
    }

    #[test]
    fn parse_ak_any() {
        let r = Range::parse("AK").unwrap();
        assert_eq!(count_near(&r, 1.0), 16);
    }

    #[test]
    fn parse_all_pairs_plus() {
        let r = Range::parse("22+").unwrap();
        assert_eq!(count_near(&r, 1.0), 78); // 13 pairs × 6 combos.
    }

    #[test]
    fn parse_pair_range_inclusive() {
        let r = Range::parse("88-TT").unwrap();
        // 88, 99, TT = 3 pairs × 6 combos = 18.
        assert_eq!(count_near(&r, 1.0), 18);
    }

    #[test]
    fn parse_pair_minus_is_leq() {
        let r = Range::parse("JJ-").unwrap();
        // 22..JJ = 10 pairs × 6 = 60.
        assert_eq!(count_near(&r, 1.0), 60);
    }

    #[test]
    fn parse_t9s_plus_iterates_first_rank() {
        let r = Range::parse("T9s+").unwrap();
        // T9s, J9s, Q9s, K9s, A9s = 5 tokens × 4 suited combos = 20.
        assert_eq!(count_near(&r, 1.0), 20);
    }

    #[test]
    fn parse_qts_plus() {
        let r = Range::parse("QTs+").unwrap();
        // QTs, KTs, ATs = 3 × 4 = 12.
        assert_eq!(count_near(&r, 1.0), 12);
    }

    #[test]
    fn parse_multiple_tokens() {
        let r = Range::parse("AA, KK, AKs").unwrap();
        assert_eq!(count_near(&r, 1.0), 6 + 6 + 4);
    }

    #[test]
    fn parse_case_insensitive() {
        let upper = Range::parse("AKs").unwrap();
        let mixed = Range::parse("aks").unwrap();
        assert_eq!(upper.total_weight(), mixed.total_weight());
        // And the same combos specifically.
        for i in 0..NUM_COMBOS {
            assert!((upper.weights[i] - mixed.weights[i]).abs() < 1e-6);
        }
    }

    #[test]
    fn parse_weight_suffix_applies_to_every_combo() {
        let r = Range::parse("AA:0.5").unwrap();
        assert_eq!(count_near(&r, 0.5), 6);
        assert!((r.total_weight() - 3.0).abs() < 1e-4);
    }

    #[test]
    fn parse_weight_suffix_on_two_rank() {
        let r = Range::parse("AKs:0.25").unwrap();
        assert_eq!(count_near(&r, 0.25), 4);
    }

    #[test]
    fn parse_last_write_wins() {
        // AKs should overwrite the weight AK already wrote.
        let r = Range::parse("AK, AKs:0.5").unwrap();
        assert_eq!(count_near(&r, 0.5), 4); // the 4 suited combos.
        assert_eq!(count_near(&r, 1.0), 12); // the 12 offsuit combos remain.
    }

    #[test]
    fn parse_whitespace_tolerant() {
        let r = Range::parse("  AA ,\tKK ,AKs  ").unwrap();
        assert_eq!(count_near(&r, 1.0), 16);
    }

    #[test]
    fn parse_empty_string_yields_empty_range() {
        let r = Range::parse("").unwrap();
        assert_eq!(r.total_weight(), 0.0);
    }

    #[test]
    fn parse_trailing_comma_is_fine() {
        let r = Range::parse("AA,").unwrap();
        assert_eq!(count_near(&r, 1.0), 6);
    }

    #[test]
    fn malformed_returns_err_not_panic() {
        // Unknown rank character.
        assert!(Range::parse("ZZ").is_err());
        // Junk trailing characters.
        assert!(Range::parse("AAz").is_err());
        // Unclosed weight suffix.
        assert!(Range::parse("AA:").is_err());
        // Non-numeric weight.
        assert!(Range::parse("AA:xxx").is_err());
        // Non-pair in a pair-range token.
        assert!(Range::parse("88-A9").is_err());
        // Reversed ranks.
        assert!(Range::parse("2A").is_err());
        // Empty token-body (just a suffix).
        assert!(Range::parse(":0.5").is_err());
        // Single character.
        assert!(Range::parse("A").is_err());
    }
}
