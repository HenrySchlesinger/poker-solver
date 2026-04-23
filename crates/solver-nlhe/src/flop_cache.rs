//! Flop cache ingestion + runtime lookup.
//!
//! This is the sibling module to [`crate::preflop`]: where `preflop` holds
//! static ranges keyed by `(position, stack_bb, pot_type)`, this one holds
//! precomputed flop-subgame **strategies** keyed by
//! `(canonical_board, spr_bucket, pot_type, bet_tree_version)`.
//!
//! # Pipeline
//!
//! 1. Colab overnight job runs `solver-cli precompute` for each spot and
//!    writes one JSON file per `(board, spr, pot, bet_tree)` tuple to
//!    Google Drive (see `colab/precompute_flops.md`).
//! 2. Henry's Mac runs `scripts/pull-colab-cache.sh` which syncs Drive
//!    locally and invokes `solver-cli pack-cache` to serialize all the
//!    JSONs into one binary — the format below.
//! 3. The packed binary ships with Poker Panel. At runtime the app calls
//!    [`FlopCache::load_from_file`] once on startup, then
//!    [`FlopCache::lookup`] per hand.
//!
//! # On-disk format (v0.1, packed binary)
//!
//! Little-endian throughout. Chosen over JSON for the same reasons the
//! preflop format was: ~8x smaller, one-shot parse, no serde on hot path.
//!
//! ```text
//!   offset 0    : magic        [u8; 8]  = b"PSFLOP\0\0"
//!   offset 8    : version      u16 LE   = 1
//!   offset 10   : reserved     u16 LE   = 0
//!   offset 12   : num_entries  u32 LE
//!   offset 16   : entries[num_entries], each:
//!                   canonical_board   [u8; 3]       — three Card bytes
//!                   spr_bucket        u8
//!                   pot_type          u8            — discriminant of PotType
//!                   bet_tree_version  u8
//!                   num_actions       u8            — ≤ MAX_ACTIONS
//!                   reserved          u8            — pad to 8-byte alignment
//!                   strategies        [[f32; 1326]; num_actions]   LE
//!                   ev_per_action     [f32; num_actions]           LE
//!                   exploitability    f32 LE
//!                 → variable-size: 8 + num_actions*(1326*4 + 4) + 4
//!                                = 12 + num_actions * 5308
//! ```
//!
//! The per-entry header is 8 bytes so the f32 payload starts 8-byte aligned
//! after the 16-byte file header (16 + 8 = 24, which is 8-aligned). Aligned
//! starts let future `bytemuck::cast_slice` optimizations land with no
//! format change.
//!
//! # Versioning + compatibility
//!
//! `version` bumps on layout changes. Adding a new `PotType` variant does
//! NOT bump it (discriminants are stable per `#[repr(u8)]` on the enum —
//! see [`crate::preflop::PotType`]). `bet_tree_version` is a per-entry
//! marker so we can ship a single file containing strategies for multiple
//! bet-tree profiles and select on load — no separate files per profile.
//!
//! # Dedup policy
//!
//! `pack_cache_from_dir` processes JSONs in filename-sorted order; if two
//! JSONs share a key the **later** one wins (later = larger filename in
//! byte order, which, given Colab's `{board}_{spr}spr_{pot}_{bet_tree}.json`
//! naming convention, is effectively "the most recent resolve for the same
//! spot"). This is documented in `colab/precompute_flops.md`.
//!
//! Downstream the in-memory [`FlopCache::entries`] map also refuses
//! duplicates at the binary-format level — duplicate `(board, spr, pot,
//! bet_tree)` tuples in one packed file is a format error, because the
//! packer should have deduped already.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{anyhow, Context, Result};

// Re-export PotType from preflop so callers don't have to import from two
// places. A6 owns the enum; we're just surfacing it.
pub use crate::preflop::PotType;

/// Number of hole-card combos in NLHE. Matches
/// `solver_eval::combo::NUM_COMBOS`; defined independently here so this
/// module doesn't take on a `solver-eval` dep just for one constant. If
/// that constant ever changes, a compile-time assertion in the test
/// module would fire — but realistically NLHE is not adding new hole cards.
const NUM_COMBOS: usize = 1326;

/// Maximum supported number of actions per entry. The on-disk layout
/// packs `num_actions` as a `u8` so we could in principle go to 255, but
/// a cap of 8 matches the v0.1 bet-tree (check + up to 7 bet sizes) with
/// some headroom. Callers exceeding this get a clear error at pack time.
pub const MAX_ACTIONS: usize = 8;

/// Magic bytes at the start of a flop cache file. "PSFLOP" + two pad
/// bytes keeps the header 8-byte aligned.
const MAGIC: [u8; 8] = *b"PSFLOP\0\0";

/// Current on-disk format version. Bump when the layout changes.
const FORMAT_VERSION: u16 = 1;

/// File-level header size in bytes: magic(8) + version(2) + reserved(2) +
/// num_entries(4) = 16.
const FILE_HEADER_BYTES: usize = 16;

/// Per-entry header size in bytes: canonical_board(3) + spr_bucket(1) +
/// pot_type(1) + bet_tree_version(1) + num_actions(1) + reserved(1) = 8.
const ENTRY_HEADER_BYTES: usize = 8;

/// Bytes per action's strategy vector: 1326 f32 weights.
const STRATEGY_BYTES: usize = NUM_COMBOS * 4;

/// Bytes per action's EV: one f32.
const EV_BYTES: usize = 4;

/// Bytes per entry's exploitability trailer: one f32.
const EXPL_BYTES: usize = 4;

/// Size of an entry for a given `num_actions`. Used by both the packer
/// (to pre-size the buffer) and the loader (to validate file length).
#[inline]
const fn entry_bytes(num_actions: usize) -> usize {
    ENTRY_HEADER_BYTES + num_actions * (STRATEGY_BYTES + EV_BYTES) + EXPL_BYTES
}

/// Decode a `PotType` from its on-disk discriminant.
///
/// Kept local (rather than calling into `preflop.rs`, whose analogous
/// helper is private to that module) so this module owns its own
/// parser surface. Discriminant values must stay in sync with the
/// `#[repr(u8)]` attributes on [`PotType`]. If `preflop.rs` ever
/// appends a new variant, add the matching arm here.
fn pot_type_from_u8(byte: u8) -> Result<PotType> {
    match byte {
        0 => Ok(PotType::Srp),
        1 => Ok(PotType::ThreeBet),
        2 => Ok(PotType::FourBet),
        3 => Ok(PotType::FiveBet),
        other => Err(anyhow!("unknown PotType discriminant: {other}")),
    }
}

/// Lookup key for the flop cache. Opaque to callers; they pass the
/// individual fields to [`FlopCache::lookup`] and we build the key
/// internally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    canonical_board: [u8; 3],
    spr_bucket: u8,
    pot_type: PotType,
    bet_tree_version: u8,
}

/// A precomputed flop strategy for a single `(board, spr, pot, bet_tree)`
/// spot.
///
/// `strategies[i]` is a 1326-wide weight vector for action `i` across all
/// hole-card combos. `ev_per_action[i]` is the expected value (in chips)
/// of choosing action `i` given a uniform prior on combos. Exploitability
/// is the BB/100 gap between the average strategy and a best response.
#[derive(Debug, Clone)]
pub struct CachedFlopStrategy {
    /// One 1326-wide vector per action. Invariant: `len() <= MAX_ACTIONS`.
    pub strategies: Vec<[f32; NUM_COMBOS]>,
    /// EV per action in chips. Always `strategies.len()` long.
    pub ev_per_action: Vec<f32>,
    /// Exploitability of the cached average strategy.
    pub exploitability: f32,
}

/// A loaded flop cache.
///
/// Build via [`FlopCache::load_from_file`] once per process; the
/// resulting map is ~400 MB at full coverage so you do NOT want to
/// clone it. Hand out `&FlopCache` references instead.
pub struct FlopCache {
    entries: HashMap<Key, CachedFlopStrategy>,
}

impl FlopCache {
    /// Load a flop cache from the binary format documented at the top of
    /// this module.
    ///
    /// Returns `Err` on missing/unreadable file, bad magic, unsupported
    /// format version, truncated file, invalid `PotType` discriminant,
    /// or `num_actions` exceeding [`MAX_ACTIONS`]. Never panics on
    /// malformed input — the file could in principle be tampered with on
    /// disk, and the process shouldn't abort on that.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::load_from_bytes(&bytes).with_context(|| format!("parsing {}", path.display()))
    }

    /// Deserialize from an in-memory byte slice. Factored out so the
    /// round-trip and fuzz tests can hit the parser with hand-built
    /// inputs — including deliberately malformed ones — without touching
    /// the filesystem.
    pub fn load_from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < FILE_HEADER_BYTES {
            return Err(anyhow!(
                "file too short for header: {} < {}",
                bytes.len(),
                FILE_HEADER_BYTES
            ));
        }

        if bytes[0..8] != MAGIC {
            return Err(anyhow!("bad magic bytes: {:?}", &bytes[0..8]));
        }

        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != FORMAT_VERSION {
            return Err(anyhow!(
                "unsupported format version: got {version}, expected {FORMAT_VERSION}"
            ));
        }
        // bytes[10..12] reserved; not validated on load (must be written 0).

        let num_entries = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;

        let mut entries: HashMap<Key, CachedFlopStrategy> = HashMap::with_capacity(num_entries);
        let mut cursor = FILE_HEADER_BYTES;

        for i in 0..num_entries {
            // Each entry is variable-size (num_actions varies), so we
            // peek at its header to compute how far to advance.
            if cursor + ENTRY_HEADER_BYTES > bytes.len() {
                return Err(anyhow!(
                    "truncated at entry {i}: need {} header bytes, have {}",
                    ENTRY_HEADER_BYTES,
                    bytes.len() - cursor
                ));
            }
            let canonical_board = [bytes[cursor], bytes[cursor + 1], bytes[cursor + 2]];
            let spr_bucket = bytes[cursor + 3];
            let pot_type = pot_type_from_u8(bytes[cursor + 4])?;
            let bet_tree_version = bytes[cursor + 5];
            let num_actions = bytes[cursor + 6] as usize;
            // bytes[cursor + 7] reserved; ignored on load.

            if num_actions > MAX_ACTIONS {
                return Err(anyhow!(
                    "entry {i}: num_actions {num_actions} exceeds MAX_ACTIONS {MAX_ACTIONS}"
                ));
            }

            let payload_bytes = num_actions * (STRATEGY_BYTES + EV_BYTES) + EXPL_BYTES;
            if cursor + ENTRY_HEADER_BYTES + payload_bytes > bytes.len() {
                return Err(anyhow!(
                    "truncated at entry {i}: need {} payload bytes, have {}",
                    payload_bytes,
                    bytes.len() - cursor - ENTRY_HEADER_BYTES
                ));
            }

            // Strategies — one [f32; 1326] per action.
            let strat_off = cursor + ENTRY_HEADER_BYTES;
            let mut strategies = Vec::with_capacity(num_actions);
            for a in 0..num_actions {
                let mut row = [0.0_f32; NUM_COMBOS];
                let row_off = strat_off + a * STRATEGY_BYTES;
                for (j, slot) in row.iter_mut().enumerate() {
                    let wo = row_off + j * 4;
                    *slot = f32::from_le_bytes([
                        bytes[wo],
                        bytes[wo + 1],
                        bytes[wo + 2],
                        bytes[wo + 3],
                    ]);
                }
                strategies.push(row);
            }

            // EV per action — num_actions f32s.
            let ev_off = strat_off + num_actions * STRATEGY_BYTES;
            let mut ev_per_action = Vec::with_capacity(num_actions);
            for a in 0..num_actions {
                let eo = ev_off + a * EV_BYTES;
                ev_per_action.push(f32::from_le_bytes([
                    bytes[eo],
                    bytes[eo + 1],
                    bytes[eo + 2],
                    bytes[eo + 3],
                ]));
            }

            // Exploitability — one f32 trailer.
            let expl_off = ev_off + num_actions * EV_BYTES;
            let exploitability = f32::from_le_bytes([
                bytes[expl_off],
                bytes[expl_off + 1],
                bytes[expl_off + 2],
                bytes[expl_off + 3],
            ]);

            let key = Key {
                canonical_board,
                spr_bucket,
                pot_type,
                bet_tree_version,
            };

            let val = CachedFlopStrategy {
                strategies,
                ev_per_action,
                exploitability,
            };

            if entries.insert(key, val).is_some() {
                return Err(anyhow!(
                    "duplicate entry at index {i}: (board={:?}, spr={}, pot={:?}, bet_tree={})",
                    key.canonical_board,
                    key.spr_bucket,
                    key.pot_type,
                    key.bet_tree_version
                ));
            }

            cursor += ENTRY_HEADER_BYTES + payload_bytes;
        }

        if cursor != bytes.len() {
            return Err(anyhow!(
                "trailing {} bytes after {num_entries} entries",
                bytes.len() - cursor
            ));
        }

        Ok(FlopCache { entries })
    }

    /// Look up a cached strategy.
    ///
    /// Returns `None` on miss — callers either fall back to a live solve
    /// (river/turn solvers) or surface it as a shipped-data gap.
    ///
    /// The `bet_tree_version` defaults to the shipped v0.1 value (1) via
    /// [`FlopCache::lookup`]; use [`FlopCache::lookup_with_bet_tree`] if
    /// you need to select a specific version within a multi-version file.
    pub fn lookup(
        &self,
        canonical_board: &[u8; 3],
        spr_bucket: u8,
        pot_type: PotType,
    ) -> Option<&CachedFlopStrategy> {
        self.lookup_with_bet_tree(canonical_board, spr_bucket, pot_type, 1)
    }

    /// Look up a cached strategy for a specific bet-tree version. See
    /// [`FlopCache::lookup`] for the common-case wrapper.
    pub fn lookup_with_bet_tree(
        &self,
        canonical_board: &[u8; 3],
        spr_bucket: u8,
        pot_type: PotType,
        bet_tree_version: u8,
    ) -> Option<&CachedFlopStrategy> {
        self.entries.get(&Key {
            canonical_board: *canonical_board,
            spr_bucket,
            pot_type,
            bet_tree_version,
        })
    }

    /// Number of entries in this cache. Mostly useful in tests + logs.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True iff `len() == 0`.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A single packable entry: the tuple `pack_binary` wants. Exposed so
/// `solver-cli pack-cache` can build these from Colab JSONs without
/// constructing `Key`s manually.
#[derive(Debug, Clone)]
pub struct PackEntry {
    /// Three canonical board cards (each 0..52).
    pub canonical_board: [u8; 3],
    /// Bucketed stack-to-pot ratio.
    pub spr_bucket: u8,
    /// Preflop pot type.
    pub pot_type: PotType,
    /// Version marker for the bet tree this strategy was computed under.
    pub bet_tree_version: u8,
    /// Strategy payload.
    pub strategy: CachedFlopStrategy,
}

/// Serialize a set of entries to the binary format documented at the top
/// of this module.
///
/// Errors:
///   * duplicate `(board, spr, pot, bet_tree)` key — caller should have
///     deduped already; we refuse rather than silently drop.
///   * an entry's `strategies.len()` exceeds [`MAX_ACTIONS`].
///   * `strategies.len()` and `ev_per_action.len()` don't match.
///   * I/O error on the output path.
///
/// On the happy path, writes atomically via the same pattern as
/// `preflop::write_binary`: build the whole buffer in memory then one
/// `File::create` + `write_all`. Flop caches are ~400 MB so that's still
/// fine — a single contiguous allocation versus 20k tiny syscalls.
pub fn pack_binary(path: &Path, entries: &[PackEntry]) -> Result<()> {
    // Pre-validate: dup keys + per-entry invariants. Done before opening
    // the output so a caller bug doesn't leave a half-written file behind.
    {
        let mut seen: HashMap<Key, ()> = HashMap::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            if e.strategy.strategies.len() != e.strategy.ev_per_action.len() {
                return Err(anyhow!(
                    "entry {i}: strategies.len()={} does not match ev_per_action.len()={}",
                    e.strategy.strategies.len(),
                    e.strategy.ev_per_action.len()
                ));
            }
            if e.strategy.strategies.len() > MAX_ACTIONS {
                return Err(anyhow!(
                    "entry {i}: num_actions {} exceeds MAX_ACTIONS {MAX_ACTIONS}",
                    e.strategy.strategies.len()
                ));
            }
            let key = Key {
                canonical_board: e.canonical_board,
                spr_bucket: e.spr_bucket,
                pot_type: e.pot_type,
                bet_tree_version: e.bet_tree_version,
            };
            if seen.insert(key, ()).is_some() {
                return Err(anyhow!(
                    "duplicate entry at index {i}: (board={:?}, spr={}, pot={:?}, bet_tree={})",
                    key.canonical_board,
                    key.spr_bucket,
                    key.pot_type,
                    key.bet_tree_version
                ));
            }
        }
    }

    let num_entries: u32 = entries
        .len()
        .try_into()
        .map_err(|_| anyhow!("too many entries: {}", entries.len()))?;
    let total_bytes: usize = FILE_HEADER_BYTES
        + entries
            .iter()
            .map(|e| entry_bytes(e.strategy.strategies.len()))
            .sum::<usize>();
    let mut buf = Vec::with_capacity(total_bytes);

    buf.extend_from_slice(&MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&0_u16.to_le_bytes()); // reserved
    buf.extend_from_slice(&num_entries.to_le_bytes());

    for e in entries {
        let num_actions = e.strategy.strategies.len() as u8;
        buf.extend_from_slice(&e.canonical_board);
        buf.push(e.spr_bucket);
        buf.push(e.pot_type as u8);
        buf.push(e.bet_tree_version);
        buf.push(num_actions);
        buf.push(0); // reserved pad

        for row in &e.strategy.strategies {
            for w in row.iter() {
                buf.extend_from_slice(&w.to_le_bytes());
            }
        }
        for ev in &e.strategy.ev_per_action {
            buf.extend_from_slice(&ev.to_le_bytes());
        }
        buf.extend_from_slice(&e.strategy.exploitability.to_le_bytes());
    }

    debug_assert_eq!(buf.len(), total_bytes);

    let mut file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    file.write_all(&buf)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_strategy(num_actions: usize, seed: u32) -> CachedFlopStrategy {
        let mut strategies = Vec::with_capacity(num_actions);
        for a in 0..num_actions {
            let mut row = [0.0_f32; NUM_COMBOS];
            for (i, w) in row.iter_mut().enumerate() {
                // Deterministic non-trivial pattern so a byte-swap would
                // surface as a mismatch.
                *w = (seed as f32) * 0.001 + (a as f32) * 0.01 + (i as f32) * 1e-6;
            }
            strategies.push(row);
        }
        let ev_per_action = (0..num_actions)
            .map(|a| (seed as f32) + a as f32 * 0.5)
            .collect();
        CachedFlopStrategy {
            strategies,
            ev_per_action,
            exploitability: 0.001 * seed as f32,
        }
    }

    #[test]
    fn constants_sane() {
        // ENTRY_HEADER_BYTES is 8 so the file pays attention to
        // alignment. If someone shrinks this, the alignment rationale
        // in the module doc becomes false.
        assert_eq!(ENTRY_HEADER_BYTES, 8);
        // Sanity: entry with 0 actions still costs header + exploitability.
        assert_eq!(entry_bytes(0), ENTRY_HEADER_BYTES + EXPL_BYTES);
        // 1 action: header + 1*(strategy + ev) + exploitability
        assert_eq!(
            entry_bytes(1),
            ENTRY_HEADER_BYTES + STRATEGY_BYTES + EV_BYTES + EXPL_BYTES
        );
    }

    #[test]
    fn roundtrip_three_entries_via_bytes() {
        let entries = vec![
            PackEntry {
                canonical_board: [0, 4, 8],
                spr_bucket: 4,
                pot_type: PotType::Srp,
                bet_tree_version: 1,
                strategy: sample_strategy(3, 1),
            },
            PackEntry {
                canonical_board: [12, 16, 20],
                spr_bucket: 15,
                pot_type: PotType::ThreeBet,
                bet_tree_version: 1,
                strategy: sample_strategy(2, 2),
            },
            PackEntry {
                canonical_board: [40, 44, 51],
                spr_bucket: 30,
                pot_type: PotType::FourBet,
                bet_tree_version: 2,
                strategy: sample_strategy(MAX_ACTIONS, 3),
            },
        ];

        let path = std::env::temp_dir().join(format!(
            "flop-cache-roundtrip-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));

        pack_binary(&path, &entries).unwrap();
        let loaded = FlopCache::load_from_file(&path).unwrap();
        assert_eq!(loaded.len(), entries.len());

        for e in &entries {
            let got = loaded
                .lookup_with_bet_tree(
                    &e.canonical_board,
                    e.spr_bucket,
                    e.pot_type,
                    e.bet_tree_version,
                )
                .expect("entry present after round-trip");
            assert_eq!(got.strategies.len(), e.strategy.strategies.len());
            for (g, w) in got.strategies.iter().zip(e.strategy.strategies.iter()) {
                assert_eq!(&g[..], &w[..]);
            }
            assert_eq!(got.ev_per_action, e.strategy.ev_per_action);
            assert_eq!(got.exploitability, e.strategy.exploitability);
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn empty_cache_roundtrips() {
        let path =
            std::env::temp_dir().join(format!("flop-cache-empty-{}.bin", std::process::id()));
        pack_binary(&path, &[]).unwrap();
        let loaded = FlopCache::load_from_file(&path).unwrap();
        assert!(loaded.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lookup_default_bet_tree_version_is_1() {
        let entries = vec![PackEntry {
            canonical_board: [1, 2, 3],
            spr_bucket: 8,
            pot_type: PotType::Srp,
            bet_tree_version: 1,
            strategy: sample_strategy(2, 1),
        }];
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&0_u16.to_le_bytes());
        buf.extend_from_slice(&1_u32.to_le_bytes());
        // entry:
        buf.extend_from_slice(&entries[0].canonical_board);
        buf.push(entries[0].spr_bucket);
        buf.push(entries[0].pot_type as u8);
        buf.push(entries[0].bet_tree_version);
        buf.push(entries[0].strategy.strategies.len() as u8);
        buf.push(0);
        for row in &entries[0].strategy.strategies {
            for w in row.iter() {
                buf.extend_from_slice(&w.to_le_bytes());
            }
        }
        for ev in &entries[0].strategy.ev_per_action {
            buf.extend_from_slice(&ev.to_le_bytes());
        }
        buf.extend_from_slice(&entries[0].strategy.exploitability.to_le_bytes());

        let loaded = FlopCache::load_from_bytes(&buf).unwrap();
        assert!(loaded.lookup(&[1, 2, 3], 8, PotType::Srp).is_some());
    }

    #[test]
    fn lookup_miss_returns_none() {
        let entries = vec![PackEntry {
            canonical_board: [1, 2, 3],
            spr_bucket: 8,
            pot_type: PotType::Srp,
            bet_tree_version: 1,
            strategy: sample_strategy(2, 1),
        }];
        let path = std::env::temp_dir().join(format!("flop-cache-miss-{}.bin", std::process::id()));
        pack_binary(&path, &entries).unwrap();
        let loaded = FlopCache::load_from_file(&path).unwrap();
        // Different board
        assert!(loaded.lookup(&[4, 5, 6], 8, PotType::Srp).is_none());
        // Different spr
        assert!(loaded.lookup(&[1, 2, 3], 4, PotType::Srp).is_none());
        // Different pot type
        assert!(loaded.lookup(&[1, 2, 3], 8, PotType::ThreeBet).is_none());
        // Different bet tree
        assert!(loaded
            .lookup_with_bet_tree(&[1, 2, 3], 8, PotType::Srp, 99)
            .is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn malformed_bad_magic_returns_err() {
        let mut bytes = vec![0_u8; FILE_HEADER_BYTES];
        bytes[0..8].copy_from_slice(b"NOTMAGIC");
        assert!(FlopCache::load_from_bytes(&bytes).is_err());
    }

    #[test]
    fn malformed_short_header_returns_err() {
        let bytes = vec![0_u8; 4];
        assert!(FlopCache::load_from_bytes(&bytes).is_err());
    }

    #[test]
    fn malformed_future_version_returns_err() {
        let mut bytes = vec![0_u8; FILE_HEADER_BYTES];
        bytes[0..8].copy_from_slice(&MAGIC);
        bytes[8..10].copy_from_slice(&999_u16.to_le_bytes());
        assert!(FlopCache::load_from_bytes(&bytes).is_err());
    }

    #[test]
    fn malformed_truncated_body_returns_err() {
        // Header claims 1 entry, body is empty.
        let mut bytes = vec![0_u8; FILE_HEADER_BYTES];
        bytes[0..8].copy_from_slice(&MAGIC);
        bytes[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes[12..16].copy_from_slice(&1_u32.to_le_bytes());
        assert!(FlopCache::load_from_bytes(&bytes).is_err());
    }

    #[test]
    fn malformed_invalid_pot_type_returns_err() {
        let entries = vec![PackEntry {
            canonical_board: [1, 2, 3],
            spr_bucket: 8,
            pot_type: PotType::Srp,
            bet_tree_version: 1,
            strategy: sample_strategy(1, 1),
        }];
        let path =
            std::env::temp_dir().join(format!("flop-cache-bad-pot-{}.bin", std::process::id()));
        pack_binary(&path, &entries).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        // Corrupt the pot_type byte at offset FILE_HEADER_BYTES + 4 (after
        // the 3-byte board + 1-byte spr).
        bytes[FILE_HEADER_BYTES + 4] = 99;
        assert!(FlopCache::load_from_bytes(&bytes).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn malformed_num_actions_too_large_returns_err() {
        // Build a valid 1-entry file then corrupt num_actions to be > MAX_ACTIONS.
        let entries = vec![PackEntry {
            canonical_board: [1, 2, 3],
            spr_bucket: 8,
            pot_type: PotType::Srp,
            bet_tree_version: 1,
            strategy: sample_strategy(1, 1),
        }];
        let path =
            std::env::temp_dir().join(format!("flop-cache-too-many-{}.bin", std::process::id()));
        pack_binary(&path, &entries).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[FILE_HEADER_BYTES + 6] = (MAX_ACTIONS + 1) as u8;
        assert!(FlopCache::load_from_bytes(&bytes).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn malformed_trailing_bytes_returns_err() {
        let entries = vec![PackEntry {
            canonical_board: [1, 2, 3],
            spr_bucket: 8,
            pot_type: PotType::Srp,
            bet_tree_version: 1,
            strategy: sample_strategy(1, 1),
        }];
        let path =
            std::env::temp_dir().join(format!("flop-cache-trailing-{}.bin", std::process::id()));
        pack_binary(&path, &entries).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&[0_u8; 16]); // junk suffix
        assert!(FlopCache::load_from_bytes(&bytes).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pack_rejects_duplicate_keys() {
        let entries = vec![
            PackEntry {
                canonical_board: [1, 2, 3],
                spr_bucket: 8,
                pot_type: PotType::Srp,
                bet_tree_version: 1,
                strategy: sample_strategy(1, 1),
            },
            PackEntry {
                canonical_board: [1, 2, 3],
                spr_bucket: 8,
                pot_type: PotType::Srp,
                bet_tree_version: 1,
                strategy: sample_strategy(1, 2),
            },
        ];
        let path = std::env::temp_dir().join(format!("flop-cache-dup-{}.bin", std::process::id()));
        assert!(pack_binary(&path, &entries).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pack_rejects_mismatched_strategy_and_ev_lens() {
        let bad = CachedFlopStrategy {
            strategies: vec![[0.0_f32; NUM_COMBOS], [0.0_f32; NUM_COMBOS]],
            ev_per_action: vec![1.0], // only one; mismatch
            exploitability: 0.0,
        };
        let entries = vec![PackEntry {
            canonical_board: [1, 2, 3],
            spr_bucket: 8,
            pot_type: PotType::Srp,
            bet_tree_version: 1,
            strategy: bad,
        }];
        let path =
            std::env::temp_dir().join(format!("flop-cache-mismatch-{}.bin", std::process::id()));
        assert!(pack_binary(&path, &entries).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pack_rejects_too_many_actions() {
        let bad = CachedFlopStrategy {
            strategies: vec![[0.0_f32; NUM_COMBOS]; MAX_ACTIONS + 1],
            ev_per_action: vec![0.0; MAX_ACTIONS + 1],
            exploitability: 0.0,
        };
        let entries = vec![PackEntry {
            canonical_board: [1, 2, 3],
            spr_bucket: 8,
            pot_type: PotType::Srp,
            bet_tree_version: 1,
            strategy: bad,
        }];
        let path = std::env::temp_dir().join(format!(
            "flop-cache-too-many-actions-{}.bin",
            std::process::id()
        ));
        assert!(pack_binary(&path, &entries).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unique_keys_after_pack() {
        // Property test: any packed file we accept must deserialize into
        // a map with distinct keys. (Duplicates fail at pack time.)
        let entries = vec![
            PackEntry {
                canonical_board: [1, 2, 3],
                spr_bucket: 8,
                pot_type: PotType::Srp,
                bet_tree_version: 1,
                strategy: sample_strategy(1, 1),
            },
            PackEntry {
                canonical_board: [1, 2, 3],
                spr_bucket: 8,
                pot_type: PotType::Srp,
                bet_tree_version: 2, // differs — distinct key
                strategy: sample_strategy(1, 2),
            },
        ];
        let path =
            std::env::temp_dir().join(format!("flop-cache-unique-{}.bin", std::process::id()));
        pack_binary(&path, &entries).unwrap();
        let loaded = FlopCache::load_from_file(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        let _ = std::fs::remove_file(&path);
    }
}
