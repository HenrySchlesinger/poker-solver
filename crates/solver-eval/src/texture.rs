//! Board-texture bucketing.
//!
//! Two boards are "the same texture" if they play similarly. The texture
//! bucket is a key for the flop cache when we don't want the full
//! (1,755 canonical flops) resolution — e.g., for warm-starting or for
//! coarse-grained strategy clustering.
//!
//! # Bit layout of [`TextureBucket`]
//!
//! All fields packed into a `u16`. Bits are numbered from the LSB.
//!
//! | Bits  | Field                   | Type             | Values                        |
//! |-------|-------------------------|------------------|-------------------------------|
//! | 0..=1 | pairedness              | [`Pairedness`]   | Unpaired / Paired / Trips / Quads |
//! | 2..=3 | suitedness              | [`Suitedness`]   | Rainbow / TwoTone / Monotone  |
//! | 4..=6 | high-card rank bucket   | [`RankBucket`]   | Low / Mid / High / Ace        |
//! | 7..=9 | connectedness           | [`ConnectBucket`]| Tight / Middling / Gapped     |
//! | 10..=11 | straight-draw potential | [`DrawBucket`] | None / Weak (OESD) / Strong (wrap) |
//! | 12..=13 | flush-draw potential    | [`DrawBucket`] | None / Weak (any) / Strong (dominant) |
//! | 14    | paired-rank-is-top      | bool             | 0/1 (0 if no pair)            |
//! | 15    | reserved                | —                | always 0                      |
//!
//! Total used bits: 15. Remaining bits are reserved and guaranteed zero.
//!
//! # What the features mean
//!
//! - **Pairedness**: count of the most frequent rank on the board.
//! - **Suitedness**: count of the most frequent suit on the board.
//! - **High rank bucket**: the top rank on the board, bucketed coarsely.
//!   Buckets defined in the [`RankBucket`] doc.
//! - **Connectedness**: sum of rank gaps between adjacent *distinct* ranks
//!   on the board. E.g., JT9 → gaps 1+1=2 (Tight); 982 → 1+6=7 (Gapped).
//!   A board with only one distinct rank (trips/quads) is Tight by
//!   convention.
//! - **Straight-draw potential**: how much the board contributes to
//!   straight draws. Strong for 3-card sequences (or very wrap-heavy
//!   boards), Weak for 2-card connectors / 1-gap, None otherwise.
//! - **Flush-draw potential**: None (rainbow), Weak (two-tone; a single
//!   suit has 2 cards), Strong (monotone; a single suit has 3 cards).
//! - **Paired-rank-is-top**: for paired/trips/quads boards, whether the
//!   pair rank is the highest-ranking card on the board. Zero for
//!   unpaired boards.

use crate::board::Board;
use crate::card::Card;

/// Compact texture identifier. Opaque; use equality and hashing only.
///
/// See the module docs for the bit layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct TextureBucket(pub u16);

/// How many of the same rank appear on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Pairedness {
    /// No two cards share a rank.
    Unpaired = 0,
    /// Exactly two cards share a rank.
    Paired = 1,
    /// Three cards share a rank.
    Trips = 2,
    /// Four cards share a rank (only possible on turn+).
    Quads = 3,
}

/// How many of the same suit appear on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Suitedness {
    /// All cards have different suits up to the per-board maximum.
    ///
    /// For a flop this means three different suits; for a turn or river
    /// it means no suit appears more than twice (i.e., no flush-draw
    /// contribution above two-tone level — but on turn/river we don't
    /// degrade to "rainbow" if 3 share a suit; see [`Suitedness::Monotone`]
    /// for the 3-of-a-suit case).
    Rainbow = 0,
    /// Exactly two cards share a suit.
    TwoTone = 1,
    /// Three (or more) cards share a suit.
    Monotone = 2,
}

/// Coarse bucket for the board's highest-ranking card.
///
/// Strict ranges:
/// - `Low`    → top card is 2 through 8
/// - `Mid`    → top card is 9 or T
/// - `High`   → top card is J, Q, or K
/// - `Ace`    → top card is an Ace
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RankBucket {
    /// Top card is 2..=8.
    Low = 0,
    /// Top card is 9 or T.
    Mid = 1,
    /// Top card is J, Q, or K.
    High = 2,
    /// Top card is an Ace.
    Ace = 3,
}

/// Connectedness of the distinct ranks on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ConnectBucket {
    /// Sum of gaps between adjacent distinct ranks is 0..=2.
    Tight = 0,
    /// Sum of gaps between adjacent distinct ranks is 3..=5.
    Middling = 1,
    /// Sum of gaps between adjacent distinct ranks is 6 or more.
    Gapped = 2,
}

/// Draw-potential bucket (used for both straight and flush draws).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DrawBucket {
    /// No draw possible from this board texture.
    None = 0,
    /// Weak draw (e.g., OESD-possible, or two-tone suit).
    Weak = 1,
    /// Strong draw (e.g., wrap / 3-straight, or monotone suit).
    Strong = 2,
}

// ---------- Bit layout constants ----------
//
// Packed into u16. See module docs.

const BITS_PAIRED: u32 = 0;
const BITS_SUIT: u32 = 2;
const BITS_RANK: u32 = 4;
const BITS_CONNECT: u32 = 7;
const BITS_STRAIGHT_DRAW: u32 = 10;
const BITS_FLUSH_DRAW: u32 = 12;
const BITS_PAIR_IS_TOP: u32 = 14;

const MASK_2: u16 = 0b11;
const MASK_3: u16 = 0b111;
const MASK_1: u16 = 0b1;

impl TextureBucket {
    /// Pairedness field.
    pub fn paired(self) -> Pairedness {
        match (self.0 >> BITS_PAIRED) & MASK_2 {
            0 => Pairedness::Unpaired,
            1 => Pairedness::Paired,
            2 => Pairedness::Trips,
            3 => Pairedness::Quads,
            _ => unreachable!(),
        }
    }

    /// Suitedness field.
    pub fn suited(self) -> Suitedness {
        match (self.0 >> BITS_SUIT) & MASK_2 {
            0 => Suitedness::Rainbow,
            1 => Suitedness::TwoTone,
            2 => Suitedness::Monotone,
            // 3 reserved; return Monotone to be safe.
            _ => Suitedness::Monotone,
        }
    }

    /// High-card rank bucket field.
    pub fn high_rank(self) -> RankBucket {
        match (self.0 >> BITS_RANK) & MASK_3 {
            0 => RankBucket::Low,
            1 => RankBucket::Mid,
            2 => RankBucket::High,
            3 => RankBucket::Ace,
            _ => RankBucket::Ace,
        }
    }

    /// Connectedness field.
    pub fn connect(self) -> ConnectBucket {
        match (self.0 >> BITS_CONNECT) & MASK_3 {
            0 => ConnectBucket::Tight,
            1 => ConnectBucket::Middling,
            2 => ConnectBucket::Gapped,
            _ => ConnectBucket::Gapped,
        }
    }

    /// Straight-draw potential field.
    pub fn straight_draw(self) -> DrawBucket {
        match (self.0 >> BITS_STRAIGHT_DRAW) & MASK_2 {
            0 => DrawBucket::None,
            1 => DrawBucket::Weak,
            2 => DrawBucket::Strong,
            _ => DrawBucket::Strong,
        }
    }

    /// Flush-draw potential field.
    pub fn flush_draw(self) -> DrawBucket {
        match (self.0 >> BITS_FLUSH_DRAW) & MASK_2 {
            0 => DrawBucket::None,
            1 => DrawBucket::Weak,
            2 => DrawBucket::Strong,
            _ => DrawBucket::Strong,
        }
    }

    /// Whether the pair rank (if any) is also the board's top rank.
    /// Always `false` for unpaired boards.
    pub fn pair_is_top(self) -> bool {
        ((self.0 >> BITS_PAIR_IS_TOP) & MASK_1) != 0
    }
}

/// Compute the texture bucket for a board.
///
/// Accepts flops (3 cards), turns (4), and rivers (5). For preflop
/// (0 cards) the returned bucket is all zeros (no texture).
pub fn texture_of(board: &Board) -> TextureBucket {
    let cards = board.as_slice();
    if cards.is_empty() {
        return TextureBucket(0);
    }

    let (paired, pair_is_top) = compute_pairedness(cards);
    let suited = compute_suitedness(cards);
    let high = compute_rank_bucket(cards);
    let connect = compute_connect(cards);
    let straight = compute_straight_draw(cards);
    let flush = compute_flush_draw(cards);

    let mut bits: u16 = 0;
    bits |= (paired as u16) << BITS_PAIRED;
    bits |= (suited as u16) << BITS_SUIT;
    bits |= (high as u16) << BITS_RANK;
    bits |= (connect as u16) << BITS_CONNECT;
    bits |= (straight as u16) << BITS_STRAIGHT_DRAW;
    bits |= (flush as u16) << BITS_FLUSH_DRAW;
    bits |= (pair_is_top as u16) << BITS_PAIR_IS_TOP;

    TextureBucket(bits)
}

// ----------------------------------------------------------------------
// Feature computation
// ----------------------------------------------------------------------

/// Returns counts per rank index (0..13), length = card count.
fn rank_counts(cards: &[Card]) -> [u8; 13] {
    let mut counts = [0u8; 13];
    for c in cards {
        counts[c.rank() as usize] += 1;
    }
    counts
}

/// Returns counts per suit index (0..4).
fn suit_counts(cards: &[Card]) -> [u8; 4] {
    let mut counts = [0u8; 4];
    for c in cards {
        counts[c.suit() as usize] += 1;
    }
    counts
}

/// (pairedness, pair_is_top_flag).
///
/// pair_is_top is true iff there is a pair/trips/quads AND the repeated
/// rank is the highest rank on the board.
fn compute_pairedness(cards: &[Card]) -> (Pairedness, bool) {
    let counts = rank_counts(cards);
    let max = *counts.iter().max().unwrap_or(&0);

    let pairedness = match max {
        0 | 1 => Pairedness::Unpaired,
        2 => Pairedness::Paired,
        3 => Pairedness::Trips,
        _ => Pairedness::Quads,
    };

    // pair_is_top: the highest rank on the board is the one that repeats.
    let pair_is_top = if max >= 2 {
        let top_rank = top_rank_index(cards);
        counts[top_rank] == max
    } else {
        false
    };

    (pairedness, pair_is_top)
}

fn compute_suitedness(cards: &[Card]) -> Suitedness {
    let counts = suit_counts(cards);
    let max = *counts.iter().max().unwrap_or(&0);
    if max >= 3 {
        Suitedness::Monotone
    } else if max == 2 {
        Suitedness::TwoTone
    } else {
        Suitedness::Rainbow
    }
}

fn top_rank_index(cards: &[Card]) -> usize {
    cards.iter().map(|c| c.rank() as usize).max().unwrap_or(0)
}

fn compute_rank_bucket(cards: &[Card]) -> RankBucket {
    // Rank indices: 0 = Two, 12 = Ace.
    let top = top_rank_index(cards);
    match top {
        0..=6 => RankBucket::Low,   // 2..=8
        7..=8 => RankBucket::Mid,   // 9, T
        9..=11 => RankBucket::High, // J, Q, K
        _ => RankBucket::Ace,       // A
    }
}

/// Sum of gaps between adjacent distinct ranks on the board.
///
/// "Gap" here is `r_{i+1} - r_i`, where ranks are sorted descending.
/// A board with only one distinct rank (trips/quads) has zero gap.
fn distinct_ranks_sorted_desc(cards: &[Card]) -> smallvec_like::RankVec {
    let counts = rank_counts(cards);
    let mut ranks = smallvec_like::RankVec::new();
    for (i, &c) in counts.iter().enumerate().rev() {
        if c > 0 {
            ranks.push(i as u8);
        }
    }
    ranks
}

fn compute_connect(cards: &[Card]) -> ConnectBucket {
    let ranks = distinct_ranks_sorted_desc(cards);
    if ranks.len() <= 1 {
        return ConnectBucket::Tight;
    }
    let mut gap_sum: u8 = 0;
    for i in 0..ranks.len() - 1 {
        // ranks[i] > ranks[i+1] because sorted desc; gap counted as the
        // number of ranks strictly between them minus 1 is ambiguous, so
        // we use "distance between ranks" = ranks[i] - ranks[i+1]. For
        // adjacent ranks (e.g., T,9) this is 1.
        gap_sum = gap_sum.saturating_add(ranks[i] - ranks[i + 1]);
    }
    // Also consider the ace-low wrap: ace (12) adjacent to 5 (3) counts
    // as a 2-gap for straight purposes, but for the overall "connectedness"
    // metric we keep it simple and ignore wrap here. Straight-draw logic
    // below handles A-5 specifically.
    match gap_sum {
        0..=2 => ConnectBucket::Tight,
        3..=5 => ConnectBucket::Middling,
        _ => ConnectBucket::Gapped,
    }
}

/// A board's straight-draw potential.
///
/// Classification rules (flop-centric; extends naturally to turn):
/// - **Strong**: 3 distinct ranks in a span of 4 (e.g., JT9, 987, 654),
///   or a board that contains 3 cards of any 5-rank window *and* one of
///   them can combine with two one-card-apart ranks (a "wrap" texture).
///   Also includes A-5 wheel clumps (A-2-3, A-3-4, A-2-4, etc.).
/// - **Weak**: at least one pair of distinct ranks within 4 of each
///   other but not strong — covers 2-card connectors or 1-gappers (e.g.,
///   KQ5, JT4, 862).
/// - **None**: no two distinct ranks within 4.
fn compute_straight_draw(cards: &[Card]) -> DrawBucket {
    let ranks = distinct_ranks_sorted_desc(cards);
    if ranks.len() < 2 {
        return DrawBucket::None;
    }

    // Consider ace-low: if an ace is present, add a synthetic "rank -1"
    // (represented here by wrapping around and treating ace as index 12
    // OR -1). We check both.
    let has_ace = ranks[0] == 12;

    // Check for "Strong" = 3 distinct ranks fitting in a 5-rank window
    // (gap sum <= 4 among 3 distinct ranks = strongly drawy).
    let n = ranks.len();
    if n >= 3 {
        for i in 0..n - 2 {
            let top = ranks[i];
            let bot = ranks[i + 2];
            if top - bot <= 4 {
                return DrawBucket::Strong;
            }
        }
    }
    if has_ace && n >= 3 {
        // Ace-low wrap: map ace to -1 and see if the three lowest ranks
        // (ranks[n-1], ranks[n-2], ace-as-low) fit in a 5-window.
        // Equivalently, check if the two smallest ranks + ace-as-low fit.
        let bot = ranks[n - 1] as i8;
        let mid = ranks[n - 2] as i8;
        let ace_low: i8 = -1;
        // Sorted desc: mid, bot, ace_low. top - ace_low <= 4 means
        // mid - ace_low <= 4, i.e. mid <= 3 (that's 5).
        if mid - ace_low <= 4 {
            return DrawBucket::Strong;
        }
        let _ = bot;
    }

    // Check for "Weak": any two distinct ranks within 4.
    for i in 0..n - 1 {
        if ranks[i] - ranks[i + 1] <= 4 {
            return DrawBucket::Weak;
        }
    }
    // Also check ace-low pairs: ace with a low card.
    if has_ace {
        // smallest rank
        let small = ranks[n - 1] as i8;
        // ace-as-low is -1
        if small - (-1) <= 4 {
            return DrawBucket::Weak;
        }
    }

    DrawBucket::None
}

fn compute_flush_draw(cards: &[Card]) -> DrawBucket {
    let counts = suit_counts(cards);
    let max = *counts.iter().max().unwrap_or(&0);
    if max >= 3 {
        DrawBucket::Strong
    } else if max == 2 {
        DrawBucket::Weak
    } else {
        DrawBucket::None
    }
}

// ----------------------------------------------------------------------
// Tiny fixed-capacity rank vector (avoids allocation and smallvec dep).
// ----------------------------------------------------------------------
mod smallvec_like {
    /// Fixed-capacity stack vector for rank indices. Up to 5 distinct
    /// ranks on a river.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct RankVec {
        data: [u8; 5],
        len: u8,
    }
    impl RankVec {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn push(&mut self, v: u8) {
            debug_assert!((self.len as usize) < self.data.len());
            self.data[self.len as usize] = v;
            self.len += 1;
        }
        pub fn len(&self) -> usize {
            self.len as usize
        }
    }
    impl std::ops::Index<usize> for RankVec {
        type Output = u8;
        fn index(&self, i: usize) -> &u8 {
            debug_assert!(i < self.len());
            &self.data[i]
        }
    }
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn flop(s: &str) -> Board {
        Board::parse(s).unwrap_or_else(|| panic!("bad flop: {s}"))
    }

    // ---- field-level checks: ----

    #[test]
    fn asksqs_monotone_high_tight_strong_flush_strong_straight() {
        let b = flop("AsKsQs");
        let t = texture_of(&b);
        assert_eq!(t.paired(), Pairedness::Unpaired);
        assert_eq!(t.suited(), Suitedness::Monotone);
        assert_eq!(t.high_rank(), RankBucket::Ace);
        assert_eq!(t.connect(), ConnectBucket::Tight);
        assert_eq!(t.flush_draw(), DrawBucket::Strong);
        // AKQ has 3 distinct ranks in a 3-wide window → strong straight draw.
        assert_eq!(t.straight_draw(), DrawBucket::Strong);
        assert!(!t.pair_is_top());
    }

    #[test]
    fn jhth9c_two_tone_high_tight_strong_straight_weak_flush() {
        let b = flop("JhTh9c");
        let t = texture_of(&b);
        assert_eq!(t.paired(), Pairedness::Unpaired);
        assert_eq!(t.suited(), Suitedness::TwoTone);
        assert_eq!(t.high_rank(), RankBucket::High);
        assert_eq!(t.connect(), ConnectBucket::Tight);
        assert_eq!(t.straight_draw(), DrawBucket::Strong);
        assert_eq!(t.flush_draw(), DrawBucket::Weak);
        assert!(!t.pair_is_top());
    }

    #[test]
    fn eights_paired_rainbow_low() {
        let b = flop("8h8c3d");
        let t = texture_of(&b);
        assert_eq!(t.paired(), Pairedness::Paired);
        assert_eq!(t.suited(), Suitedness::Rainbow);
        assert_eq!(t.high_rank(), RankBucket::Low); // rank 8 → Low (2-8)
        assert_eq!(t.flush_draw(), DrawBucket::None);
        assert!(t.pair_is_top(), "the pair of 8s IS the top rank");
    }

    #[test]
    fn low_disconnected_board() {
        let b = flop("2c3d8h");
        let t = texture_of(&b);
        assert_eq!(t.paired(), Pairedness::Unpaired);
        assert_eq!(t.suited(), Suitedness::Rainbow);
        assert_eq!(t.high_rank(), RankBucket::Low);
        // gaps: (8-3)+(3-2)=6 → Gapped
        assert_eq!(t.connect(), ConnectBucket::Gapped);
        assert_eq!(t.flush_draw(), DrawBucket::None);
    }

    #[test]
    fn aces_trips_rainbow_ace() {
        let b = flop("AcAdAh");
        let t = texture_of(&b);
        assert_eq!(t.paired(), Pairedness::Trips);
        assert_eq!(t.suited(), Suitedness::Rainbow);
        assert_eq!(t.high_rank(), RankBucket::Ace);
        assert_eq!(t.connect(), ConnectBucket::Tight); // only 1 distinct rank
        assert_eq!(t.flush_draw(), DrawBucket::None);
        assert_eq!(t.straight_draw(), DrawBucket::None);
        assert!(t.pair_is_top());
    }

    #[test]
    fn low_straight_flush_board() {
        let b = flop("7s6s5s");
        let t = texture_of(&b);
        assert_eq!(t.paired(), Pairedness::Unpaired);
        assert_eq!(t.suited(), Suitedness::Monotone);
        // Top rank 7 → Low per bucket spec (2-8 low).
        assert_eq!(t.high_rank(), RankBucket::Low);
        assert_eq!(t.connect(), ConnectBucket::Tight);
        assert_eq!(t.straight_draw(), DrawBucket::Strong);
        assert_eq!(t.flush_draw(), DrawBucket::Strong);
    }

    // ---- table of canonical flops: ----

    #[test]
    fn canonical_flop_table() {
        // Format: (flop, expected paired, suited, rank, connect, strdraw, flushdraw, pair_is_top)
        type Row = (
            &'static str,
            Pairedness,
            Suitedness,
            RankBucket,
            ConnectBucket,
            DrawBucket,
            DrawBucket,
            bool,
        );
        let rows: &[Row] = &[
            (
                "AsKsQs",
                Pairedness::Unpaired,
                Suitedness::Monotone,
                RankBucket::Ace,
                ConnectBucket::Tight,
                DrawBucket::Strong,
                DrawBucket::Strong,
                false,
            ),
            (
                "JhTh9c",
                Pairedness::Unpaired,
                Suitedness::TwoTone,
                RankBucket::High,
                ConnectBucket::Tight,
                DrawBucket::Strong,
                DrawBucket::Weak,
                false,
            ),
            (
                "8h8c3d",
                Pairedness::Paired,
                Suitedness::Rainbow,
                RankBucket::Low,
                ConnectBucket::Middling, // distinct ranks 8,3 → gap 5
                DrawBucket::None,        // 8(idx6) - 3(idx1) = 5 → not within 4
                DrawBucket::None,
                true,
            ),
            (
                "2c3d8h",
                Pairedness::Unpaired,
                Suitedness::Rainbow,
                RankBucket::Low,
                ConnectBucket::Gapped, // gaps sum 6
                DrawBucket::Weak,      // 3-2 within 4
                DrawBucket::None,
                false,
            ),
            (
                "AcAdAh",
                Pairedness::Trips,
                Suitedness::Rainbow,
                RankBucket::Ace,
                ConnectBucket::Tight,
                DrawBucket::None,
                DrawBucket::None,
                true,
            ),
            (
                "7s6s5s",
                Pairedness::Unpaired,
                Suitedness::Monotone,
                RankBucket::Low,
                ConnectBucket::Tight,
                DrawBucket::Strong,
                DrawBucket::Strong,
                false,
            ),
            (
                "AhKh2s",
                Pairedness::Unpaired,
                Suitedness::TwoTone,
                RankBucket::Ace,
                ConnectBucket::Gapped, // (A-K)+(K-2) = 1+11 = 12
                DrawBucket::Weak,      // A-K within 4; A-2 is ace-low wrap → within 4
                DrawBucket::Weak,
                false,
            ),
            (
                "QsJd2c",
                Pairedness::Unpaired,
                Suitedness::Rainbow,
                RankBucket::High,
                ConnectBucket::Gapped, // (Q-J)+(J-2) = 1+9 = 10
                DrawBucket::Weak,      // Q-J connected
                DrawBucket::None,
                false,
            ),
            (
                "KhQhJc",
                Pairedness::Unpaired,
                Suitedness::TwoTone,
                RankBucket::High,
                ConnectBucket::Tight,
                DrawBucket::Strong,
                DrawBucket::Weak,
                false,
            ),
            (
                "9s9h4d",
                Pairedness::Paired,
                Suitedness::Rainbow,
                RankBucket::Mid,         // 9 → Mid (9-T)
                ConnectBucket::Middling, // 9(idx7) - 4(idx2) = 5 → Middling
                DrawBucket::None,        // 5 apart → not within 4
                DrawBucket::None,
                true,
            ),
            (
                "5h5d5c",
                Pairedness::Trips,
                Suitedness::Rainbow,
                RankBucket::Low,
                ConnectBucket::Tight,
                DrawBucket::None,
                DrawBucket::None,
                true,
            ),
            (
                "Ad5d4d",
                Pairedness::Unpaired,
                Suitedness::Monotone,
                RankBucket::Ace,
                ConnectBucket::Gapped, // (A-5)+(5-4)=7+1=8
                DrawBucket::Strong,    // A-5-4 wheel → strong (ace-low wrap)
                DrawBucket::Strong,
                false,
            ),
            (
                "TdTs4c",
                Pairedness::Paired,
                Suitedness::Rainbow,
                RankBucket::Mid,       // T = rank idx 8 → Mid
                ConnectBucket::Gapped, // T(idx8) - 4(idx2) = 6 → Gapped
                DrawBucket::None,      // T-4 differ by 6 → not within 4
                DrawBucket::None,
                true,
            ),
            (
                "Ks7h2s",
                Pairedness::Unpaired,
                Suitedness::TwoTone,
                RankBucket::High,
                ConnectBucket::Gapped, // (K-7)+(7-2)=4+5=9
                DrawBucket::None,      // no two within 4
                DrawBucket::Weak,
                false,
            ),
            (
                "6c5c4c",
                Pairedness::Unpaired,
                Suitedness::Monotone,
                RankBucket::Low,
                ConnectBucket::Tight,
                DrawBucket::Strong,
                DrawBucket::Strong,
                false,
            ),
        ];

        // First pass: ensure internal consistency between the table and
        // actual compute.
        for (flop_s, paired, suited, rank, connect, strd, fld, pair_top) in rows {
            let b = flop(flop_s);
            let t = texture_of(&b);
            assert_eq!(t.paired(), *paired, "{flop_s} pairedness");
            assert_eq!(t.suited(), *suited, "{flop_s} suitedness");
            assert_eq!(t.high_rank(), *rank, "{flop_s} rank bucket");
            assert_eq!(t.connect(), *connect, "{flop_s} connect");
            assert_eq!(t.straight_draw(), *strd, "{flop_s} straight draw");
            assert_eq!(t.flush_draw(), *fld, "{flop_s} flush draw");
            assert_eq!(t.pair_is_top(), *pair_top, "{flop_s} pair_is_top");
        }
    }

    // ---- property tests: ----

    #[test]
    fn deterministic() {
        // Same input, same output (trivially true for pure code, but sanity).
        let b = flop("AhKh2s");
        let a = texture_of(&b);
        let c = texture_of(&b);
        assert_eq!(a, c);
    }

    #[test]
    fn all_flops_fit_in_u16_with_reserved_bit_zero() {
        use crate::card::{Rank, Suit};
        // Try every distinct flop (order-independent): sample a spread.
        let mut count = 0;
        for r0 in 0..13u8 {
            for s0 in 0..4u8 {
                for r1 in 0..13u8 {
                    for s1 in 0..4u8 {
                        for r2 in 0..13u8 {
                            for s2 in 0..4u8 {
                                let c0 = (r0 << 2) | s0;
                                let c1 = (r1 << 2) | s1;
                                let c2 = (r2 << 2) | s2;
                                if c0 == c1 || c0 == c2 || c1 == c2 {
                                    continue;
                                }
                                let rank0: Rank = unsafe { std::mem::transmute(r0) };
                                let suit0: Suit = unsafe { std::mem::transmute(s0) };
                                let rank1: Rank = unsafe { std::mem::transmute(r1) };
                                let suit1: Suit = unsafe { std::mem::transmute(s1) };
                                let rank2: Rank = unsafe { std::mem::transmute(r2) };
                                let suit2: Suit = unsafe { std::mem::transmute(s2) };
                                let b = Board::flop(
                                    Card::new(rank0, suit0),
                                    Card::new(rank1, suit1),
                                    Card::new(rank2, suit2),
                                );
                                let t = texture_of(&b);
                                // Reserved bit 15 always zero.
                                assert_eq!(t.0 & (1 << 15), 0, "reserved bit must be zero");
                                count += 1;
                                // Sample cap to keep test fast.
                                if count > 5000 {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn empty_board_is_zero() {
        let b = Board::empty();
        assert_eq!(texture_of(&b), TextureBucket(0));
    }

    #[test]
    fn permutation_of_flop_gives_same_texture() {
        // The texture function should not care about card ORDER on the
        // board (only composition).
        let a = flop("AhKh2s");
        let b = flop("2sKhAh");
        let c = flop("KhAh2s");
        assert_eq!(texture_of(&a), texture_of(&b));
        assert_eq!(texture_of(&a), texture_of(&c));
    }

    #[test]
    fn suit_relabel_preserves_texture_when_counts_unchanged() {
        // Relabeling suits consistently should not change the texture
        // (which is the whole point of this being a cache key).
        // Ah Kh 2s vs Ad Kd 2c — both two-tone with same rank profile.
        let a = flop("AhKh2s");
        let b = flop("AdKd2c");
        assert_eq!(texture_of(&a), texture_of(&b));
    }

    #[test]
    fn accessor_roundtrip_matches_packing() {
        // For each accessor, setting a field through packing and reading
        // it back must round-trip. Do it for a handful of synthetic bits.
        let cases = &[
            (Pairedness::Quads, Suitedness::Monotone, RankBucket::Ace),
            (Pairedness::Paired, Suitedness::TwoTone, RankBucket::High),
            (Pairedness::Unpaired, Suitedness::Rainbow, RankBucket::Low),
            (Pairedness::Trips, Suitedness::Monotone, RankBucket::Mid),
        ];
        for &(p, s, r) in cases {
            let bits =
                ((p as u16) << BITS_PAIRED) | ((s as u16) << BITS_SUIT) | ((r as u16) << BITS_RANK);
            let t = TextureBucket(bits);
            assert_eq!(t.paired(), p);
            assert_eq!(t.suited(), s);
            assert_eq!(t.high_rank(), r);
        }
    }

    #[test]
    fn isomorphic_boards_same_bucket() {
        // canonical_board renames suits but preserves suit multiplicities
        // and ranks — so the texture bucket must be invariant under it.
        use crate::iso::canonical_board;
        let cases = &[
            "AhKh2s", "AsKsQs", "JhTh9c", "8h8c3d", "2c3d8h", "AcAdAh", "7s6s5s", "KhQhJc",
            "9s9h4d", "Ad5d4d", "TdTs4c", "Ks7h2s",
        ];
        for s in cases {
            let a = flop(s);
            let ca = canonical_board(&a);
            assert_eq!(
                texture_of(&a),
                texture_of(&ca),
                "canonicalized {s} -> {ca} changed texture"
            );
        }
    }
}
