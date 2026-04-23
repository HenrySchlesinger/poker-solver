//! `seed-cache` subcommand — generate the v0.1 flop-cache seed binary.
//!
//! # Why this exists
//!
//! The full Colab flop-precompute pipeline (see `colab/precompute_flops.md`)
//! is a Day-5 deliverable. Ahead of that, we need a **non-empty**
//! `data/flop-cache/flop-cache-v0.1.bin` committed to the repo so:
//!
//!   * downstream consumers (Poker Panel, `FlopCache::load_from_file`) can
//!     be wired end-to-end and exercised in tests without waiting on
//!     Colab;
//!   * the format + loader + packer path is proven in CI;
//!   * the repo ships with a sensible default artifact rather than a
//!     missing file, which would force every consumer to special-case
//!     "maybe the cache isn't packed yet".
//!
//! # What v0.1 ships
//!
//! **Format-only placeholder data.** The strategies in the packed file are
//! hand-constructed, deterministic, and NOT the result of a real CFR+
//! solve. They exist purely to give the loader + consumer non-empty,
//! validly-shaped entries to chew on. Real GTO strategies land in Day 5
//! when Colab precompute replaces this file wholesale. See
//! `data/flop-cache/README.md` for the user-facing explanation.
//!
//! Grid (12 boards × 3 SPR buckets × 1 pot type = 36 entries):
//!
//!   * Canonical flops:
//!       `AhKd2c`, `QsJd2c`, `Th7c2d`, `JhTh9c`, `9h8c7d`, `QhJhTs`,
//!       `8h8c3d`, `AhAc5d`, `KhKdKc`, `AhKhQh`, `7s6s5s`, `ThJhKh`
//!   * SPR buckets: `{4, 8, 15}`
//!   * Pot type: `Srp`
//!   * Bet tree version: `1` (matches the default
//!     `FlopCache::lookup` expects)
//!
//! Each entry carries `num_actions = 2` (check + pot-bet), with
//! strategies biased by flop texture:
//!
//!   * Dry/high-card boards (e.g. `AhKd2c`) → check-heavy (~0.7 check).
//!   * Wet/coordinated boards (e.g. `JhTh9c`) → bet-heavy (~0.7 bet).
//!   * Monotone / straight boards fall between.
//!
//! The weights are uniform across all 1326 combos — there's no
//! combo-specific skew, because that would misrepresent the placeholder
//! as real range-aware strategy. Two-action uniform-per-combo data is
//! the smallest dataset that still round-trips meaningfully through the
//! binary format.
//!
//! # Cache file size
//!
//! ```text
//!   FILE_HEADER (16) + 36 * entry_bytes(2)
//!   entry_bytes(2) = 8 + 2*(1326*4 + 4) + 4 = 10_628
//!   total = 16 + 36 * 10_628 = 382_624 bytes ≈ 374 KB
//! ```
//!
//! Well under the 500 KB success criterion.

use std::path::PathBuf;

use anyhow::{Context, Result};

use solver_eval::card::Card;
use solver_nlhe::flop_cache::{pack_binary, CachedFlopStrategy, FlopCache, PackEntry, PotType};

/// Number of hole-card combos in NLHE. Matches the constant inside
/// `solver_nlhe::flop_cache`; duplicated here so this module has no
/// reason to take a `solver-eval::combo` dep.
const NUM_COMBOS: usize = 1326;

/// SPR buckets used for the v0.1 seed grid. Chosen to cover low / medium
/// / deep effective stacks at a realistic streaming table (cash / MTT
/// late-reg / MTT early respectively).
const SPR_BUCKETS: [u8; 3] = [4, 8, 15];

/// The 12 canonical flops in the v0.1 seed grid. Mix of:
///
///   * dry high-card boards: `AhKd2c`, `QsJd2c`, `Th7c2d`
///   * dynamic middle boards: `JhTh9c`, `9h8c7d`
///   * monotone: `QhJhTs`
///   * paired: `8h8c3d`, `AhAc5d`
///   * trips: `KhKdKc`
///   * three-to-flush (non-monotone): `AhKhQh`, `ThJhKh`
///   * three-to-straight (suited): `7s6s5s`
///
/// Each string is exactly 6 chars (three 2-char cards) and is parsed
/// via `Card::parse` below — if any of these is malformed we fail loudly
/// at seed time, not at user runtime.
const BOARDS: [&str; 12] = [
    "AhKd2c", "QsJd2c", "Th7c2d", "JhTh9c", "9h8c7d", "QhJhTs", "8h8c3d", "AhAc5d", "KhKdKc",
    "AhKhQh", "7s6s5s", "ThJhKh",
];

/// The pot type for v0.1. Only `Srp` for now — 3BP/4BP/5BP will ship
/// once Colab generates them on Day 5+.
const POT_TYPE: PotType = PotType::Srp;

/// Bet-tree version baked into every entry. Must equal the default that
/// `FlopCache::lookup` expects so callers who don't specify a version
/// hit these entries.
const BET_TREE_VERSION: u8 = 1;

/// Args for `solver-cli seed-cache`. Parsed from the `clap` struct in
/// `main.rs`.
pub struct SeedCacheArgs {
    /// Output binary path. The canonical location is
    /// `data/flop-cache/flop-cache-v0.1.bin` (see `.gitignore` — only
    /// this exact filename is whitelisted into git).
    pub output: PathBuf,
}

/// Entry point for `solver-cli seed-cache`.
///
/// Builds the 36-entry seed grid in memory, calls `pack_binary` to
/// serialize it, and performs a round-trip verification via
/// `FlopCache::load_from_file`. The round-trip is the "canary" — if it
/// ever fails, we want seed-cache to refuse to ship a bad file rather
/// than let a format regression slip into the committed binary.
pub fn run_seed_cache(args: &SeedCacheArgs) -> Result<()> {
    let entries = build_seed_entries()?;
    let num_entries = entries.len();

    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating output directory {}", parent.display()))?;
        }
    }

    pack_binary(&args.output, &entries)
        .with_context(|| format!("writing {}", args.output.display()))?;

    // Canary round-trip. If this fails, the file on disk is corrupt /
    // format-mismatched and we want to surface that here, not at
    // Poker Panel runtime.
    let loaded = FlopCache::load_from_file(&args.output)
        .with_context(|| format!("round-trip load of {}", args.output.display()))?;
    if loaded.len() != num_entries {
        anyhow::bail!(
            "round-trip length mismatch: packed {} entries, loaded {}",
            num_entries,
            loaded.len()
        );
    }
    for e in &entries {
        let got = loaded
            .lookup_with_bet_tree(
                &e.canonical_board,
                e.spr_bucket,
                e.pot_type,
                e.bet_tree_version,
            )
            .with_context(|| {
                format!(
                    "round-trip lookup miss for board={:?} spr={} pot={:?} bet_tree={}",
                    e.canonical_board, e.spr_bucket, e.pot_type, e.bet_tree_version
                )
            })?;
        if got.strategies.len() != e.strategy.strategies.len() {
            anyhow::bail!(
                "round-trip num_actions mismatch for board={:?} spr={}: packed {}, loaded {}",
                e.canonical_board,
                e.spr_bucket,
                e.strategy.strategies.len(),
                got.strategies.len()
            );
        }
    }

    let size = std::fs::metadata(&args.output)
        .map(|m| m.len())
        .unwrap_or(0);
    eprintln!(
        "seed-cache: wrote {} entries ({} bytes) to {}",
        num_entries,
        size,
        args.output.display()
    );
    Ok(())
}

/// Build the full 36-entry seed grid. Factored out so tests can call it
/// without hitting the filesystem.
pub(crate) fn build_seed_entries() -> Result<Vec<PackEntry>> {
    let mut entries = Vec::with_capacity(BOARDS.len() * SPR_BUCKETS.len());
    for board_str in BOARDS {
        let canonical_board = parse_board_three_cards(board_str)
            .with_context(|| format!("parsing seed board {:?}", board_str))?;
        let texture = classify_texture(board_str);
        for &spr_bucket in &SPR_BUCKETS {
            let strategy = placeholder_strategy(texture, spr_bucket);
            entries.push(PackEntry {
                canonical_board,
                spr_bucket,
                pot_type: POT_TYPE,
                bet_tree_version: BET_TREE_VERSION,
                strategy,
            });
        }
    }
    Ok(entries)
}

/// Parse a 6-char board string into three `Card` bytes. Mirrors the
/// helper in `pack_cache.rs` but private to this module — seed-cache
/// has exactly one input shape (hardcoded boards) and shouldn't reach
/// into pack_cache's private API.
fn parse_board_three_cards(s: &str) -> Result<[u8; 3]> {
    if s.len() != 6 {
        anyhow::bail!("board {:?} must be 6 chars (3 cards)", s);
    }
    let mut out = [0u8; 3];
    for i in 0..3 {
        let card_s = &s[i * 2..i * 2 + 2];
        let card = Card::parse(card_s)
            .with_context(|| format!("bad card {:?} in board {:?}", card_s, s))?;
        out[i] = card.0;
    }
    Ok(out)
}

/// A coarse classification of flop texture used to pick bias for the
/// placeholder strategy. Not GTO — just enough signal that the seed
/// file isn't a flat uniform blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Texture {
    /// High-card, uncoordinated, rainbow (e.g. `AhKd2c`). Favors check.
    DryHigh,
    /// Middle-card coordinated, multiple straight draws (e.g. `9h8c7d`).
    /// Favors bet.
    WetMid,
    /// Monotone (three cards same suit). Favors bet lightly.
    Monotone,
    /// Paired / trips (e.g. `8h8c3d`, `KhKdKc`). Favors check.
    Paired,
    /// Three-to-flush / straight on high cards (e.g. `AhKhQh`, `7s6s5s`).
    /// Favors bet.
    Draws,
}

/// Classify a board string into a `Texture`. Rules are intentionally
/// simple — this is a seed-file heuristic, not a real texture classifier.
///
/// Rank check is by the 'T'/'J'/'Q'/'K'/'A' character in position 0/2/4.
fn classify_texture(board: &str) -> Texture {
    let chars: Vec<char> = board.chars().collect();
    debug_assert_eq!(chars.len(), 6, "classify_texture expects 6-char board");
    let r1 = chars[0];
    let r2 = chars[2];
    let r3 = chars[4];
    let s1 = chars[1];
    let s2 = chars[3];
    let s3 = chars[5];

    if r1 == r2 && r2 == r3 {
        return Texture::Paired; // trips
    }
    if r1 == r2 || r1 == r3 || r2 == r3 {
        return Texture::Paired;
    }
    if s1 == s2 && s2 == s3 {
        return Texture::Monotone;
    }

    // Rough rank order: '2'<'3'<...<'9'<'T'<'J'<'Q'<'K'<'A'. Map each
    // char to a 0..=12 index for numeric comparison.
    let rank_idx = |c: char| -> i32 {
        match c {
            '2' => 0,
            '3' => 1,
            '4' => 2,
            '5' => 3,
            '6' => 4,
            '7' => 5,
            '8' => 6,
            '9' => 7,
            'T' => 8,
            'J' => 9,
            'Q' => 10,
            'K' => 11,
            'A' => 12,
            _ => -1,
        }
    };
    let ranks = [rank_idx(r1), rank_idx(r2), rank_idx(r3)];
    let max_rank = ranks.iter().copied().max().unwrap_or(0);
    let min_rank = ranks.iter().copied().min().unwrap_or(0);
    let span = max_rank - min_rank;

    // Connected broadway boards with a flush draw (e.g. AhKhQh, ThJhKh):
    // two of the same suit + all high ranks.
    let two_suited = s1 == s2 || s1 == s3 || s2 == s3;
    if two_suited && min_rank >= 8 {
        return Texture::Draws;
    }

    // Connected low/mid boards (e.g. 9h8c7d, 7s6s5s): small span, low
    // top rank.
    if span <= 4 && max_rank <= 9 {
        return if s1 == s2 && s2 == s3 {
            Texture::Monotone
        } else if two_suited {
            Texture::Draws
        } else {
            Texture::WetMid
        };
    }

    // Otherwise: one low card + higher cards = dry.
    Texture::DryHigh
}

/// Return (p_check, p_bet) biased by `texture` and nudged by `spr_bucket`.
///
/// Rationale for spr bias: deeper stacks (higher SPR) incentivize checking
/// more of your range on wet boards (pot control); shallower SPRs bet
/// more of their range (they don't need to protect future streets).
/// Magnitude kept small so SPR variation shows up in the data but
/// doesn't dominate texture.
fn action_probs(texture: Texture, spr_bucket: u8) -> (f32, f32) {
    let base_check: f32 = match texture {
        Texture::DryHigh => 0.70,
        Texture::Paired => 0.65,
        Texture::WetMid => 0.30,
        Texture::Monotone => 0.45,
        Texture::Draws => 0.30,
    };
    // Deeper stacks: +0.05 check at spr=15, -0.05 check at spr=4.
    let spr_adjust: f32 = match spr_bucket {
        4 => -0.05,
        8 => 0.0,
        15 => 0.05,
        _ => 0.0,
    };
    let p_check = (base_check + spr_adjust).clamp(0.05_f32, 0.95_f32);
    (p_check, 1.0 - p_check)
}

/// Construct the placeholder `CachedFlopStrategy` for one grid cell.
///
/// Both action rows are uniform across all 1326 combos. The probabilities
/// per combo sum to 1.0 (within f32 precision), matching the invariant
/// that a strategy profile partitions combo weight across actions.
fn placeholder_strategy(texture: Texture, spr_bucket: u8) -> CachedFlopStrategy {
    let (p_check, p_bet) = action_probs(texture, spr_bucket);
    let check_row = [p_check; NUM_COMBOS];
    let bet_row = [p_bet; NUM_COMBOS];

    // EV is scaled coarsely by pot-size units. Not meaningful — just
    // non-zero placeholders so ev_per_action isn't suspiciously flat.
    let ev_check = 0.10;
    let ev_bet = 0.10 + (1.0 - p_check) * 0.20;

    // Exploitability is a small nonzero stand-in; real CFR+ would solve
    // this to < 1 mbb/hand. v0.1 labels the whole cache as placeholder,
    // so a visible 0.5 here makes it obvious the data is synthetic.
    let exploitability = 0.5;

    CachedFlopStrategy {
        strategies: vec![check_row, bet_row],
        ev_per_action: vec![ev_check, ev_bet],
        exploitability,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_has_expected_size() {
        let entries = build_seed_entries().unwrap();
        assert_eq!(entries.len(), BOARDS.len() * SPR_BUCKETS.len());
        assert_eq!(entries.len(), 36);
    }

    #[test]
    fn grid_keys_are_unique() {
        let entries = build_seed_entries().unwrap();
        let mut seen = std::collections::HashSet::new();
        for e in &entries {
            let key = (
                e.canonical_board,
                e.spr_bucket,
                e.pot_type as u8,
                e.bet_tree_version,
            );
            assert!(seen.insert(key), "duplicate key: {:?}", key);
        }
    }

    #[test]
    fn every_entry_has_two_actions() {
        let entries = build_seed_entries().unwrap();
        for e in &entries {
            assert_eq!(e.strategy.strategies.len(), 2);
            assert_eq!(e.strategy.ev_per_action.len(), 2);
        }
    }

    #[test]
    fn strategy_probabilities_sum_to_one() {
        let entries = build_seed_entries().unwrap();
        for e in &entries {
            let p_check = e.strategy.strategies[0][0];
            let p_bet = e.strategy.strategies[1][0];
            let sum = p_check + p_bet;
            assert!(
                (sum - 1.0).abs() < 1e-5,
                "probs don't sum to 1: check={p_check} bet={p_bet}"
            );
        }
    }

    #[test]
    fn seed_writes_and_roundtrips() {
        let tmp = std::env::temp_dir().join(format!(
            "flop-cache-seed-test-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let args = SeedCacheArgs {
            output: tmp.clone(),
        };
        run_seed_cache(&args).expect("seed-cache succeeds");

        let size = std::fs::metadata(&tmp).unwrap().len();
        assert!(size > 0, "seed file is empty");
        assert!(size < 500 * 1024, "seed file exceeds 500KB: {} bytes", size);

        // Loader round-trip is part of run_seed_cache, but re-load here
        // to confirm the written file is self-contained and doesn't
        // require the in-memory entries to be valid.
        let loaded = FlopCache::load_from_file(&tmp).unwrap();
        assert_eq!(loaded.len(), 36);

        // Spot-check one known entry.
        let board = parse_board_three_cards("AhKd2c").unwrap();
        let entry = loaded
            .lookup(&board, 4, PotType::Srp)
            .expect("AhKd2c spr=4 Srp present");
        assert_eq!(entry.strategies.len(), 2);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn classify_texture_known_boards() {
        assert_eq!(classify_texture("AhKd2c"), Texture::DryHigh);
        assert_eq!(classify_texture("JhTh9c"), Texture::Draws); // two-suited broadway
        assert_eq!(classify_texture("QhJhTs"), Texture::Draws); // two-suited broadway
        assert_eq!(classify_texture("9h8c7d"), Texture::WetMid);
        assert_eq!(classify_texture("8h8c3d"), Texture::Paired);
        assert_eq!(classify_texture("KhKdKc"), Texture::Paired);
        assert_eq!(classify_texture("AhKhQh"), Texture::Monotone);
        assert_eq!(classify_texture("7s6s5s"), Texture::Monotone);
        assert_eq!(classify_texture("ThJhKh"), Texture::Monotone);
    }
}
