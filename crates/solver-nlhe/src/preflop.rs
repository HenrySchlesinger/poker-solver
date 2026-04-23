//! Preflop range ingestion: load a precomputed preflop range database at
//! runtime, keyed by `(position, stack_depth_bb, pot_type)`.
//!
//! # Why this exists
//!
//! Preflop ranges are fixed by position + stack depth + pot type (SRP, 3BP,
//! 4BP, 5BP). Rather than resolve them live we ship a precomputed database;
//! the solver's live path looks them up in O(1).
//!
//! The ranges themselves are generated offline on Colab (see `docs/COLAB.md`
//! section "Preflop range generation" — a one-time ~8-hour job on Day 5 of
//! the v0.1 sprint). This module is the **runtime loader** that brings the
//! resulting file into process memory and exposes a lookup API. The
//! serializer (`write_binary`) is exposed so `solver-cli precompute` on the
//! Colab side can emit the same format — i.e. both halves of the contract
//! live in this one file.
//!
//! # On-disk format (v0.1, packed binary)
//!
//! Little-endian throughout. We chose packed binary over JSON because:
//!   * ~4x smaller (each entry is 5312 bytes of weights + a 4-byte key
//!     header vs ~20 KB of textual JSON at full precision).
//!   * Loads via a single `Read::read_to_end` + `bytemuck::cast_slice`,
//!     no parser state machine.
//!   * The file is write-once / read-many so hand-editability doesn't
//!     matter; `solver-cli dump-preflop` can print it as JSON for humans.
//!
//! Layout:
//!
//! ```text
//!   offset 0    : magic  [u8; 8]    = b"PSPRE\0\0\0"   (Poker-Solver PREflop)
//!   offset 8    : version u16 LE    = 1
//!   offset 10   : reserved u16 LE   = 0
//!   offset 12   : num_entries u32 LE
//!   offset 16   : entries[num_entries], each:
//!                   position       u8      (discriminant of `Position`)
//!                   pot_type       u8      (discriminant of `PotType`)
//!                   stack_bb       u16 LE
//!                   weights        [f32; 1326]   (5304 bytes, LE)
//!                 → total per entry = 4 + 5304 = 5308 bytes
//! ```
//!
//! A valid file therefore has size `16 + 5308 * num_entries`. Any deviation
//! is a format error and [`PreflopRanges::load_from_file`] returns `Err`
//! rather than panicking.
//!
//! # Versioning
//!
//! `version` bumps when the on-disk layout changes (adding a new `Position`
//! variant does NOT bump it — discriminant numbers are stable; see the
//! `#[repr(u8)]` attributes below). If you rearrange enum variants or change
//! the weight count, bump it and teach [`PreflopRanges::load_from_file`] to
//! read the old version.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::Range;

/// Number of hole-card combos in NLHE. Must match `Range::weights.len()`.
const NUM_COMBOS: usize = 1326;

/// Bytes per on-disk entry: 1 (position) + 1 (pot_type) + 2 (stack_bb) +
/// 4 * 1326 (weights) = 5308.
const ENTRY_BYTES: usize = 4 + 4 * NUM_COMBOS;

/// File header size in bytes.
const HEADER_BYTES: usize = 16;

/// Magic bytes at the start of an **uncompressed** preflop range file.
/// Readable as "PSPRE" plus three zero pad bytes, keeps the header
/// 8-byte aligned.
const MAGIC: [u8; 8] = *b"PSPRE\0\0\0";

/// Magic bytes at the start of a **zstd-compressed** preflop range file.
/// Readable as "PSPREZST" — same 8-byte alignment as [`MAGIC`], still
/// begins with "PSPRE" so `file` / grep still identifies it as one of
/// ours. The 16-byte header (version + num_entries) stays uncompressed
/// so a reader can validate and preallocate before decompressing. The
/// entries section is a single zstd frame.
///
/// Added by agent A33 (2026-04-22). The magic change is backward
/// compatible: [`PreflopRanges::load_from_file`] dispatches on the first
/// 8 bytes and falls through to the uncompressed reader for files that
/// predate this format.
const MAGIC_ZSTD: [u8; 8] = *b"PSPREZST";

/// Compression level passed to [`zstd::stream::encode_all`] when writing
/// compressed preflop files. Level 19 gives near-maximum ratio at
/// reasonable encode speed (a few seconds on a 100 MB input); decode is
/// level-independent. Drop to 11 if encode time becomes painful on
/// Colab — ratio only loses ~5% but encode speeds up ~10x.
const ZSTD_LEVEL: i32 = 19;

/// Current on-disk format version. Bump when the layout changes.
const FORMAT_VERSION: u16 = 1;

/// Seat / spot the hero is in, relative to villain.
///
/// v0.1 ships only the heads-up matchup (BTN vs BB). 6-max and full-ring
/// positions will be added in v0.2 — when you do, **append** new variants
/// at the end and do NOT reorder existing ones, or you'll silently break
/// every preflop file already on disk (the discriminant is what we
/// serialize).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Position {
    /// Button (small blind in heads-up), acting as the aggressor.
    BtnVsBb = 0,
    /// Big blind, defending vs a BTN open / continuation.
    BbVsBtn = 1,
    // TODO (v0.2): add 6-max positions (UTG, MP, CO, BTN, SB, BB) and
    // full-ring. Append; don't reorder.
}

impl Position {
    /// Decode a Position from its on-disk discriminant.
    fn from_u8(byte: u8) -> Result<Self> {
        match byte {
            0 => Ok(Position::BtnVsBb),
            1 => Ok(Position::BbVsBtn),
            other => Err(anyhow!("unknown Position discriminant: {other}")),
        }
    }
}

/// Type of preflop pot — single-raised, 3-bet, 4-bet, 5-bet.
///
/// Like [`Position`], discriminants are stable on disk. Append, don't
/// reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PotType {
    /// Single-raised pot (SRP). Open + call.
    Srp = 0,
    /// 3-bet pot.
    ThreeBet = 1,
    /// 4-bet pot.
    FourBet = 2,
    /// 5-bet pot (typically shove-or-fold depth).
    FiveBet = 3,
}

impl PotType {
    /// Decode a PotType from its on-disk discriminant.
    fn from_u8(byte: u8) -> Result<Self> {
        match byte {
            0 => Ok(PotType::Srp),
            1 => Ok(PotType::ThreeBet),
            2 => Ok(PotType::FourBet),
            3 => Ok(PotType::FiveBet),
            other => Err(anyhow!("unknown PotType discriminant: {other}")),
        }
    }
}

/// Lookup key for the preflop range database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    position: Position,
    stack_bb: u16,
    pot_type: PotType,
}

/// A loaded preflop range database.
///
/// Construct via [`PreflopRanges::load_from_file`] at startup, then call
/// [`PreflopRanges::lookup`] on every hand. Cheap to clone the reference,
/// never clone the struct itself (~5 KB per entry).
pub struct PreflopRanges {
    entries: HashMap<Key, Range>,
}

impl PreflopRanges {
    /// Load a preflop range database from the binary format documented in
    /// the module doc-comment.
    ///
    /// Returns `Err` on:
    ///   * missing or unreadable file
    ///   * bad magic bytes
    ///   * unsupported format version
    ///   * truncated file (declared `num_entries` doesn't match actual size)
    ///   * unknown enum discriminant
    ///
    /// Never panics on malformed input — that would be a runtime DoS vector
    /// since the file could in principle be tampered with on disk.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::load_from_bytes(&bytes).with_context(|| format!("parsing {}", path.display()))
    }

    /// Deserialize from an in-memory byte slice. Factored out so tests can
    /// hit the parser with hand-built (or deliberately-mangled) inputs
    /// without touching the filesystem.
    ///
    /// Dispatches on magic bytes:
    ///   * [`MAGIC`]      → uncompressed body (original v0.1 layout).
    ///   * [`MAGIC_ZSTD`] → zstd-compressed body (added by A33).
    ///
    /// In either case the 16-byte file header (magic + version + reserved
    /// + num_entries) is validated first, so a truncated or wrong-version
    /// file is rejected before we spend a decompression.
    fn load_from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_BYTES {
            return Err(anyhow!(
                "file too short for header: {} < {}",
                bytes.len(),
                HEADER_BYTES
            ));
        }

        let magic = &bytes[0..8];
        let is_compressed = if magic == MAGIC {
            false
        } else if magic == MAGIC_ZSTD {
            true
        } else {
            return Err(anyhow!("bad magic bytes: {:?}", magic));
        };

        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != FORMAT_VERSION {
            return Err(anyhow!(
                "unsupported format version: got {version}, expected {FORMAT_VERSION}"
            ));
        }

        // bytes[10..12] reserved; ignored on load (must be written as 0).
        let num_entries = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;

        let expected_payload_len = num_entries * ENTRY_BYTES;

        // The entry-decode loop below indexes into a "payload" slice of
        // length `expected_payload_len`. For uncompressed files that's
        // just `&bytes[HEADER_BYTES..]` (zero-copy). For compressed files
        // we decompress into a fresh `Vec` and verify its length. Either
        // way the downstream parse path is identical.
        let owned: Vec<u8>;
        let payload: &[u8] = if is_compressed {
            owned = zstd::stream::decode_all(&bytes[HEADER_BYTES..])
                .map_err(|e| anyhow!("zstd decode failed: {e}"))?;
            if owned.len() != expected_payload_len {
                return Err(anyhow!(
                    "decompressed size mismatch: num_entries={num_entries} implies {expected_payload_len} bytes, got {}",
                    owned.len()
                ));
            }
            &owned[..]
        } else {
            let expected_len = HEADER_BYTES + expected_payload_len;
            if bytes.len() != expected_len {
                return Err(anyhow!(
                    "file size mismatch: num_entries={num_entries} implies {expected_len} bytes, got {}",
                    bytes.len()
                ));
            }
            &bytes[HEADER_BYTES..]
        };

        let mut entries = HashMap::with_capacity(num_entries);
        for i in 0..num_entries {
            let off = i * ENTRY_BYTES;
            let position = Position::from_u8(payload[off])?;
            let pot_type = PotType::from_u8(payload[off + 1])?;
            let stack_bb = u16::from_le_bytes([payload[off + 2], payload[off + 3]]);

            let weights_off = off + 4;
            let mut weights = Box::new([0.0_f32; NUM_COMBOS]);
            for (j, slot) in weights.iter_mut().enumerate() {
                let wo = weights_off + j * 4;
                *slot = f32::from_le_bytes([
                    payload[wo],
                    payload[wo + 1],
                    payload[wo + 2],
                    payload[wo + 3],
                ]);
            }
            let range = Range { weights };
            let key = Key {
                position,
                stack_bb,
                pot_type,
            };

            if entries.insert(key, range).is_some() {
                return Err(anyhow!(
                    "duplicate entry at index {i}: ({:?}, {}, {:?})",
                    key.position,
                    key.stack_bb,
                    key.pot_type
                ));
            }
        }

        Ok(PreflopRanges { entries })
    }

    /// Look up a range by its key. Returns `None` on cache miss — callers
    /// should either fall back to a live solve or (for the preflop layer
    /// specifically) treat it as a configuration error and surface it.
    pub fn lookup(&self, position: Position, stack_bb: u16, pot_type: PotType) -> Option<&Range> {
        self.entries.get(&Key {
            position,
            stack_bb,
            pot_type,
        })
    }

    /// Number of entries currently loaded. Mostly useful in tests / logs.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the database is empty. See [`len`].
    ///
    /// [`len`]: Self::len
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Test/bench-only constructor: a handful of hardcoded canonical
    /// ranges so other modules can exercise the lookup path without
    /// waiting for Colab output.
    ///
    /// Ranges here are stand-ins, not solved ranges. `BTN_vs_BB @ 100bb
    /// SRP` is a "full" range (weight 1.0 on every combo) purely so tests
    /// have something non-trivial to assert on. Swap with real ranges once
    /// the Colab job lands.
    #[cfg(test)]
    pub fn test_fixture() -> Self {
        let mut entries = HashMap::new();
        let keys = [
            (Position::BtnVsBb, 100, PotType::Srp),
            (Position::BbVsBtn, 100, PotType::Srp),
            (Position::BtnVsBb, 100, PotType::ThreeBet),
            (Position::BbVsBtn, 50, PotType::FourBet),
        ];
        for (pos, stack, pot) in keys {
            // Give each fixture entry a distinguishable weight pattern so
            // a round-trip test can detect byte-swaps / off-by-one.
            let mut weights = Box::new([0.0_f32; NUM_COMBOS]);
            let seed =
                (pos as u8) as f32 * 0.01 + (pot as u8) as f32 * 0.001 + stack as f32 * 0.0001;
            for (i, w) in weights.iter_mut().enumerate() {
                *w = seed + (i as f32) * 1e-6;
            }
            entries.insert(
                Key {
                    position: pos,
                    stack_bb: stack,
                    pot_type: pot,
                },
                Range { weights },
            );
        }
        PreflopRanges { entries }
    }
}

/// Serialize a set of `(position, stack_bb, pot_type, range)` entries to
/// the binary format documented at the top of this module.
///
/// Invoked from `solver-cli precompute` on the Colab side (Day 5). Kept
/// in this module (rather than a sibling `preflop_writer.rs`) so the
/// writer and the parser share their private constants and can't drift.
///
/// Errors:
///   * duplicate key in `entries` — the same `(position, stack_bb,
///     pot_type)` appearing twice is a caller bug; we refuse rather than
///     silently dropping one.
///   * I/O error on the output path.
pub fn write_binary(path: &Path, entries: &[(Position, u16, PotType, &Range)]) -> Result<()> {
    // Pre-check for dup keys so we don't write a partial file before
    // discovering the caller handed us garbage.
    {
        let mut seen = HashMap::with_capacity(entries.len());
        for (pos, stack, pot, _) in entries {
            if seen.insert((*pos, *stack, *pot), ()).is_some() {
                return Err(anyhow!(
                    "duplicate entry: ({:?}, {}, {:?})",
                    pos,
                    stack,
                    pot
                ));
            }
        }
    }

    let num_entries: u32 = entries
        .len()
        .try_into()
        .map_err(|_| anyhow!("too many entries: {}", entries.len()))?;
    let total_bytes = HEADER_BYTES + entries.len() * ENTRY_BYTES;
    let mut buf = Vec::with_capacity(total_bytes);

    buf.extend_from_slice(&MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&0_u16.to_le_bytes()); // reserved
    buf.extend_from_slice(&num_entries.to_le_bytes());

    for (pos, stack, pot, range) in entries {
        buf.push(*pos as u8);
        buf.push(*pot as u8);
        buf.extend_from_slice(&stack.to_le_bytes());
        for w in range.weights.iter() {
            buf.extend_from_slice(&w.to_le_bytes());
        }
    }

    debug_assert_eq!(buf.len(), total_bytes);

    let mut file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    file.write_all(&buf)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Serialize to the **zstd-compressed** on-disk variant (magic
/// [`MAGIC_ZSTD`]). Same input shape as [`write_binary`]; same per-entry
/// byte layout inside the compressed frame. Only the entries section is
/// compressed — the 16-byte file header stays uncompressed so the reader
/// can validate + allocate before it decodes.
///
/// On the preflop data (mostly-smooth f32 weights in [0, 1], with many
/// near-duplicate entries across position / pot-type variants), zstd
/// level [`ZSTD_LEVEL`] typically delivers 3-5x compression — see the
/// `cache_compression` bench for numbers on the current fixture.
///
/// Error classes mirror [`write_binary`] — duplicate key, too many
/// entries, I/O — plus zstd encode errors (OOM or catastrophic system
/// failure in practice).
pub fn write_binary_compressed(
    path: &Path,
    entries: &[(Position, u16, PotType, &Range)],
) -> Result<()> {
    // Pre-check for dup keys so we don't produce any file on bad input.
    // Same check as write_binary; kept inline because factoring to a
    // helper would obscure the short-circuit pattern at the top of both
    // writers.
    {
        let mut seen = HashMap::with_capacity(entries.len());
        for (pos, stack, pot, _) in entries {
            if seen.insert((*pos, *stack, *pot), ()).is_some() {
                return Err(anyhow!(
                    "duplicate entry: ({:?}, {}, {:?})",
                    pos,
                    stack,
                    pot
                ));
            }
        }
    }

    let num_entries: u32 = entries
        .len()
        .try_into()
        .map_err(|_| anyhow!("too many entries: {}", entries.len()))?;

    // Build the raw (uncompressed) payload: byte-for-byte the same as
    // the corresponding slice in write_binary's output. Deliberately
    // shared so once the reader consumes the header it doesn't care
    // which writer produced the file.
    let payload_bytes = entries.len() * ENTRY_BYTES;
    let mut payload = Vec::with_capacity(payload_bytes);
    for (pos, stack, pot, range) in entries {
        payload.push(*pos as u8);
        payload.push(*pot as u8);
        payload.extend_from_slice(&stack.to_le_bytes());
        for w in range.weights.iter() {
            payload.extend_from_slice(&w.to_le_bytes());
        }
    }
    debug_assert_eq!(payload.len(), payload_bytes);

    // One-shot zstd frame. We don't need streaming — the uncompressed
    // buffer fits in memory by construction (we just built it).
    let compressed = zstd::stream::encode_all(&payload[..], ZSTD_LEVEL)
        .map_err(|e| anyhow!("zstd encode failed: {e}"))?;

    let mut buf = Vec::with_capacity(HEADER_BYTES + compressed.len());
    buf.extend_from_slice(&MAGIC_ZSTD);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&0_u16.to_le_bytes()); // reserved
    buf.extend_from_slice(&num_entries.to_le_bytes());
    buf.extend_from_slice(&compressed);

    let mut file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    file.write_all(&buf)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique per-test file path under the OS temp dir. Avoids a `tempfile`
    /// dep — we do not want to grow the dependency tree for a handful of
    /// unit tests.
    fn tmp_path(name: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "poker-solver-preflop-test-{pid}-{nanos}-{name}.bin"
        ));
        p
    }

    /// Serialize a fixture, load it, and verify every entry round-trips.
    #[test]
    fn fixture_roundtrip() {
        let fixture = PreflopRanges::test_fixture();
        assert!(!fixture.is_empty());

        // Collect as the shape write_binary wants.
        let mut refs: Vec<(Position, u16, PotType, &Range)> = fixture
            .entries
            .iter()
            .map(|(k, r)| (k.position, k.stack_bb, k.pot_type, r))
            .collect();
        // Sort for deterministic on-disk order (write_binary doesn't care,
        // but this gives stable hex dumps when someone's debugging).
        refs.sort_by_key(|(p, s, pt, _)| (*p as u8, *s, *pt as u8));

        let path = tmp_path("roundtrip");
        write_binary(&path, &refs).unwrap();

        let loaded = PreflopRanges::load_from_file(&path).unwrap();
        assert_eq!(loaded.len(), fixture.len());

        for (k, r) in &fixture.entries {
            let got = loaded
                .lookup(k.position, k.stack_bb, k.pot_type)
                .expect("entry present after round-trip");
            assert_eq!(&got.weights[..], &r.weights[..]);
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lookup_returns_some_for_present_key() {
        let fixture = PreflopRanges::test_fixture();
        assert!(fixture
            .lookup(Position::BtnVsBb, 100, PotType::Srp)
            .is_some());
    }

    #[test]
    fn lookup_returns_none_for_absent_key() {
        let fixture = PreflopRanges::test_fixture();
        // Different stack depth than any fixture key.
        assert!(fixture
            .lookup(Position::BtnVsBb, 42, PotType::Srp)
            .is_none());
        // Different pot type than any BtnVsBb @ 50 key (only BbVsBtn @ 50).
        assert!(fixture
            .lookup(Position::BtnVsBb, 50, PotType::FourBet)
            .is_none());
    }

    #[test]
    fn malformed_bad_magic_returns_err() {
        let mut bytes = vec![0_u8; HEADER_BYTES];
        bytes[0..8].copy_from_slice(b"NOTMAGIC");
        assert!(PreflopRanges::load_from_bytes(&bytes).is_err());
    }

    #[test]
    fn malformed_short_file_returns_err() {
        let bytes = vec![0_u8; 4]; // much smaller than HEADER_BYTES
        assert!(PreflopRanges::load_from_bytes(&bytes).is_err());
    }

    #[test]
    fn malformed_wrong_version_returns_err() {
        let mut bytes = vec![0_u8; HEADER_BYTES];
        bytes[0..8].copy_from_slice(&MAGIC);
        bytes[8..10].copy_from_slice(&999_u16.to_le_bytes());
        // num_entries = 0
        assert!(PreflopRanges::load_from_bytes(&bytes).is_err());
    }

    #[test]
    fn malformed_truncated_entries_returns_err() {
        // Header claims 2 entries but body is empty.
        let mut bytes = vec![0_u8; HEADER_BYTES];
        bytes[0..8].copy_from_slice(&MAGIC);
        bytes[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes[12..16].copy_from_slice(&2_u32.to_le_bytes());
        assert!(PreflopRanges::load_from_bytes(&bytes).is_err());
    }

    #[test]
    fn malformed_unknown_position_byte_returns_err() {
        // One entry, valid size, but a Position discriminant out of range.
        let mut bytes = vec![0_u8; HEADER_BYTES + ENTRY_BYTES];
        bytes[0..8].copy_from_slice(&MAGIC);
        bytes[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes[12..16].copy_from_slice(&1_u32.to_le_bytes());
        bytes[HEADER_BYTES] = 99; // invalid Position
        assert!(PreflopRanges::load_from_bytes(&bytes).is_err());
    }

    #[test]
    fn empty_database_roundtrips() {
        let path = tmp_path("empty");
        write_binary(&path, &[]).unwrap();
        let loaded = PreflopRanges::load_from_file(&path).unwrap();
        assert_eq!(loaded.len(), 0);
        assert!(loaded.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_binary_rejects_duplicate_keys() {
        let r = Range::full();
        let entries = vec![
            (Position::BtnVsBb, 100, PotType::Srp, &r),
            (Position::BtnVsBb, 100, PotType::Srp, &r),
        ];
        let path = tmp_path("dup");
        assert!(write_binary(&path, &entries).is_err());
        // write_binary short-circuits before creating the file on dup, so
        // don't fail the test if it doesn't exist.
        let _ = std::fs::remove_file(&path);
    }

    // ----- Compressed (zstd) variant — added by A33 ------------------

    /// Round-trip a non-trivial fixture through `write_binary_compressed`
    /// and verify every entry comes back identical after decompression.
    #[test]
    fn fixture_roundtrip_compressed() {
        let fixture = PreflopRanges::test_fixture();
        assert!(!fixture.is_empty());

        let refs: Vec<(Position, u16, PotType, &Range)> = fixture
            .entries
            .iter()
            .map(|(k, r)| (k.position, k.stack_bb, k.pot_type, r))
            .collect();

        let path = tmp_path("roundtrip-zstd");
        write_binary_compressed(&path, &refs).unwrap();

        // Sanity: the file starts with the compressed-variant magic, not
        // the uncompressed one — guards against a writer bug that would
        // silently emit the wrong magic but otherwise parse (since the
        // payload decoder is shared).
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(&on_disk[0..8], &MAGIC_ZSTD);

        let loaded = PreflopRanges::load_from_file(&path).unwrap();
        assert_eq!(loaded.len(), fixture.len());
        for (k, r) in &fixture.entries {
            let got = loaded
                .lookup(k.position, k.stack_bb, k.pot_type)
                .expect("entry present after compressed round-trip");
            assert_eq!(&got.weights[..], &r.weights[..]);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// The same reader must transparently load both variants — that's
    /// the whole point of magic-byte dispatch. Write the same fixture
    /// uncompressed and compressed, load both, diff by-key.
    #[test]
    fn reader_auto_detects_compressed_vs_uncompressed() {
        let fixture = PreflopRanges::test_fixture();
        let refs: Vec<(Position, u16, PotType, &Range)> = fixture
            .entries
            .iter()
            .map(|(k, r)| (k.position, k.stack_bb, k.pot_type, r))
            .collect();

        let p_raw = tmp_path("raw");
        let p_zst = tmp_path("zst");
        write_binary(&p_raw, &refs).unwrap();
        write_binary_compressed(&p_zst, &refs).unwrap();

        // Compressed file should be strictly smaller on this fixture
        // (several entries sharing identical init patterns). If zstd
        // ever can't shrink our test data the assumption that this
        // format wins on real data is suspect.
        let size_raw = std::fs::metadata(&p_raw).unwrap().len();
        let size_zst = std::fs::metadata(&p_zst).unwrap().len();
        assert!(
            size_zst < size_raw,
            "expected compressed < raw, got {} >= {}",
            size_zst,
            size_raw
        );

        let loaded_raw = PreflopRanges::load_from_file(&p_raw).unwrap();
        let loaded_zst = PreflopRanges::load_from_file(&p_zst).unwrap();
        assert_eq!(loaded_raw.len(), loaded_zst.len());
        for (k, r) in &fixture.entries {
            let a = loaded_raw
                .lookup(k.position, k.stack_bb, k.pot_type)
                .unwrap();
            let b = loaded_zst
                .lookup(k.position, k.stack_bb, k.pot_type)
                .unwrap();
            assert_eq!(&a.weights[..], &r.weights[..]);
            assert_eq!(&b.weights[..], &r.weights[..]);
        }

        let _ = std::fs::remove_file(&p_raw);
        let _ = std::fs::remove_file(&p_zst);
    }

    /// Empty compressed files must round-trip just like empty
    /// uncompressed ones — edge case caught a bug in an earlier draft
    /// where zstd produced a non-empty frame for a zero-byte input and
    /// the size-check mis-accounted for that.
    #[test]
    fn empty_compressed_roundtrips() {
        let path = tmp_path("empty-zst");
        write_binary_compressed(&path, &[]).unwrap();
        let loaded = PreflopRanges::load_from_file(&path).unwrap();
        assert!(loaded.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// Truncating the compressed body must fail gracefully (not panic).
    /// We produce a valid file, chop the last few bytes off, and assert
    /// the loader surfaces an error.
    #[test]
    fn malformed_truncated_compressed_returns_err() {
        let r = Range::full();
        let entries = vec![(Position::BtnVsBb, 100, PotType::Srp, &r)];
        let path = tmp_path("trunc-zst");
        write_binary_compressed(&path, &entries).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        // Chop the last 4 bytes of the compressed frame. zstd decoder
        // should fail; even if somehow it succeeded, the size check
        // inside load_from_bytes would catch a short payload.
        let cut_to = bytes.len() - 4;
        bytes.truncate(cut_to);
        assert!(PreflopRanges::load_from_bytes(&bytes).is_err());
        let _ = std::fs::remove_file(&path);
    }

    /// Valid zstd-compressed header, but the decompressed body is
    /// shorter than `num_entries * ENTRY_BYTES` implies. Must return
    /// Err (and specifically the "decompressed size mismatch" error).
    #[test]
    fn malformed_compressed_size_mismatch_returns_err() {
        // Compress an empty payload but claim num_entries=1 in the header.
        // Reader will decompress to 0 bytes, see mismatch, fail.
        let empty_frame = zstd::stream::encode_all(&[][..], ZSTD_LEVEL).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC_ZSTD);
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes()); // lies: num_entries = 1
        bytes.extend_from_slice(&empty_frame);
        assert!(PreflopRanges::load_from_bytes(&bytes).is_err());
    }
}
