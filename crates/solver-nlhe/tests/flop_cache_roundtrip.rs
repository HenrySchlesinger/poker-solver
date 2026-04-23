//! Integration tests for the flop cache on-disk format.
//!
//! Unit tests inside `src/flop_cache.rs` cover the parser's malformed-input
//! arms; these tests cover the **public API shape** (`FlopCache`, `PackEntry`,
//! `pack_binary`, `PotType` re-export) the way downstream crates will see it.
//!
//! If one of these breaks, it means the public contract has shifted —
//! downstream callers (`solver-cli pack-cache`, the runtime loader in
//! the macOS app via FFI, and the Colab-side emitter) will break too.

use solver_nlhe::flop_cache::{pack_binary, CachedFlopStrategy, FlopCache, PackEntry, PotType};

const NUM_COMBOS: usize = 1326;

/// `FlopCache` doesn't derive `Debug` (intentional — a ~400 MB debug
/// print is never useful), so `Result::unwrap_err` can't format the `Ok`
/// side directly. This helper extracts the `Err` without that bound.
fn expect_err<T>(r: anyhow::Result<T>, ctx: &str) -> anyhow::Error {
    match r {
        Ok(_) => panic!("expected Err ({ctx}), got Ok"),
        Err(e) => e,
    }
}

/// Deterministic non-trivial strategy so a byte-swap would show up as a
/// mismatch rather than all-zeros equals all-zeros.
fn sample_strategy(num_actions: usize, seed: u32) -> CachedFlopStrategy {
    let mut strategies = Vec::with_capacity(num_actions);
    for a in 0..num_actions {
        let mut row = [0.0_f32; NUM_COMBOS];
        for (i, w) in row.iter_mut().enumerate() {
            *w = (seed as f32) * 0.001 + (a as f32) * 0.01 + (i as f32) * 1e-6;
        }
        strategies.push(row);
    }
    CachedFlopStrategy {
        strategies,
        ev_per_action: (0..num_actions).map(|a| seed as f32 + a as f32).collect(),
        exploitability: (seed as f32) * 0.01,
    }
}

/// Unique tempfile path so parallel test processes don't collide. Avoids
/// a `tempfile` dep to match the preflop module's policy.
fn tmp_path(label: &str) -> std::path::PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("poker-solver-flop-cache-{label}-{pid}-{nanos}.bin"))
}

/// Build a synthetic cache (3 entries), pack to disk, load it back, and
/// verify every entry round-trips **bit-identical**.
#[test]
fn three_entries_roundtrip_bit_identical() {
    let entries = vec![
        PackEntry {
            canonical_board: [0, 4, 8],
            spr_bucket: 1,
            pot_type: PotType::Srp,
            bet_tree_version: 1,
            strategy: sample_strategy(4, 7),
        },
        PackEntry {
            canonical_board: [17, 23, 51],
            spr_bucket: 15,
            pot_type: PotType::ThreeBet,
            bet_tree_version: 1,
            strategy: sample_strategy(3, 42),
        },
        PackEntry {
            canonical_board: [2, 10, 30],
            spr_bucket: 30,
            pot_type: PotType::FourBet,
            bet_tree_version: 2,
            strategy: sample_strategy(8, 123),
        },
    ];

    let path = tmp_path("three-entries");
    pack_binary(&path, &entries).expect("pack should succeed");

    let loaded = FlopCache::load_from_file(&path).expect("load should succeed");
    assert_eq!(loaded.len(), entries.len());

    for e in &entries {
        let got = loaded
            .lookup_with_bet_tree(
                &e.canonical_board,
                e.spr_bucket,
                e.pot_type,
                e.bet_tree_version,
            )
            .unwrap_or_else(|| {
                panic!(
                    "missing entry after round-trip: board={:?} spr={} pot={:?} bt={}",
                    e.canonical_board, e.spr_bucket, e.pot_type, e.bet_tree_version,
                )
            });

        // Bit-identical comparison — using `f32::to_bits` would be safer
        // if any weight could be NaN, but our sample_strategy never
        // generates NaN, so straight equality here catches byte swaps.
        assert_eq!(
            got.strategies.len(),
            e.strategy.strategies.len(),
            "num_actions drift"
        );
        for (g, w) in got.strategies.iter().zip(e.strategy.strategies.iter()) {
            assert_eq!(&g[..], &w[..], "strategy row mismatch");
        }
        assert_eq!(
            got.ev_per_action, e.strategy.ev_per_action,
            "ev_per_action mismatch"
        );
        assert_eq!(
            got.exploitability, e.strategy.exploitability,
            "exploitability mismatch"
        );
    }

    let _ = std::fs::remove_file(&path);
}

/// Truncating the binary must cause the loader to return `Err` — never
/// panic. A truncated-file panic would be a runtime DoS vector (the file
/// can in principle be tampered with on disk post-install).
#[test]
fn truncated_file_returns_err_not_panic() {
    let entries = vec![PackEntry {
        canonical_board: [1, 2, 3],
        spr_bucket: 4,
        pot_type: PotType::Srp,
        bet_tree_version: 1,
        strategy: sample_strategy(2, 99),
    }];
    let path = tmp_path("truncated");
    pack_binary(&path, &entries).unwrap();
    let bytes = std::fs::read(&path).unwrap();

    // Try every truncation point shy of the full file. None should panic.
    for chop in 0..bytes.len() {
        let truncated = &bytes[..chop];
        let result = FlopCache::load_from_bytes(truncated);
        // Everything shorter than full must fail.
        assert!(
            result.is_err(),
            "expected Err at chop={chop}/{}, got Ok",
            bytes.len()
        );
    }

    // The full file must load cleanly (sanity).
    assert!(FlopCache::load_from_bytes(&bytes).is_ok());

    let _ = std::fs::remove_file(&path);
}

/// Version-mismatch: a future-version file must be rejected with a
/// clear error, not silently half-loaded.
#[test]
fn future_version_rejected() {
    let entries = vec![PackEntry {
        canonical_board: [1, 2, 3],
        spr_bucket: 4,
        pot_type: PotType::Srp,
        bet_tree_version: 1,
        strategy: sample_strategy(1, 1),
    }];
    let path = tmp_path("future-version");
    pack_binary(&path, &entries).unwrap();

    let mut bytes = std::fs::read(&path).unwrap();
    // Bump the version field at offset 8 to something unsupported.
    bytes[8..10].copy_from_slice(&0xFF_FFu16.to_le_bytes());

    let err = expect_err(FlopCache::load_from_bytes(&bytes), "future version");
    let msg = err.to_string();
    // Don't over-specify the exact phrasing, but the word "version" has
    // to appear somewhere in the error chain for the user to know what
    // went wrong.
    assert!(
        msg.to_lowercase().contains("version"),
        "expected version-mismatch error, got: {msg}"
    );

    let _ = std::fs::remove_file(&path);
}

/// After `pack_binary`, the loaded map must have exactly one entry per
/// `(canonical_board, spr_bucket, pot_type, bet_tree_version)` key.
#[test]
fn unique_keys_after_pack() {
    let entries = vec![
        PackEntry {
            canonical_board: [1, 2, 3],
            spr_bucket: 4,
            pot_type: PotType::Srp,
            bet_tree_version: 1,
            strategy: sample_strategy(1, 1),
        },
        PackEntry {
            canonical_board: [4, 5, 6],
            spr_bucket: 4,
            pot_type: PotType::Srp,
            bet_tree_version: 1,
            strategy: sample_strategy(1, 2),
        },
        PackEntry {
            canonical_board: [1, 2, 3],
            spr_bucket: 8,
            pot_type: PotType::Srp,
            bet_tree_version: 1,
            strategy: sample_strategy(1, 3),
        },
        PackEntry {
            canonical_board: [1, 2, 3],
            spr_bucket: 4,
            pot_type: PotType::ThreeBet,
            bet_tree_version: 1,
            strategy: sample_strategy(1, 4),
        },
    ];
    let path = tmp_path("unique");
    pack_binary(&path, &entries).unwrap();
    let loaded = FlopCache::load_from_file(&path).unwrap();
    assert_eq!(loaded.len(), 4);

    // All four distinct-key lookups hit.
    assert!(loaded.lookup(&[1, 2, 3], 4, PotType::Srp).is_some());
    assert!(loaded.lookup(&[4, 5, 6], 4, PotType::Srp).is_some());
    assert!(loaded.lookup(&[1, 2, 3], 8, PotType::Srp).is_some());
    assert!(loaded.lookup(&[1, 2, 3], 4, PotType::ThreeBet).is_some());

    let _ = std::fs::remove_file(&path);
}

/// A packed file where two entries share the same key is malformed —
/// the loader must reject it rather than silently keeping the later one.
/// (The packer refuses at write time, but hand-crafted / adversarial
/// files can still present dupes.)
#[test]
fn packed_duplicate_key_rejected_by_loader() {
    // Manually build a 2-entry file with identical keys.
    //
    // We can't use pack_binary (it would error on dup), so we hand-roll
    // the bytes. This test also serves as a reference implementation
    // for anyone writing a non-Rust encoder.
    let magic: [u8; 8] = *b"PSFLOP\0\0";
    let version: u16 = 1;
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(&magic);
    buf.extend_from_slice(&version.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved
    buf.extend_from_slice(&2u32.to_le_bytes()); // num_entries

    for _ in 0..2 {
        // canonical_board
        buf.extend_from_slice(&[1u8, 2, 3]);
        buf.push(4); // spr_bucket
        buf.push(PotType::Srp as u8);
        buf.push(1); // bet_tree_version
        buf.push(1); // num_actions
        buf.push(0); // reserved
                     // strategy row
        for _ in 0..NUM_COMBOS {
            buf.extend_from_slice(&0.0_f32.to_le_bytes());
        }
        // ev
        buf.extend_from_slice(&0.0_f32.to_le_bytes());
        // exploitability
        buf.extend_from_slice(&0.0_f32.to_le_bytes());
    }

    let err = expect_err(FlopCache::load_from_bytes(&buf), "duplicate key");
    assert!(
        err.to_string().to_lowercase().contains("duplicate"),
        "expected dup error, got: {err}",
    );
}
