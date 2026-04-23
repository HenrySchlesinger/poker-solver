//! `pack-cache` / `unpack-cache` subcommands.
//!
//! This is the Mac-side half of the Colab pipeline:
//!
//! 1. Colab runs `solver-cli precompute` for each flop spot overnight,
//!    writing one JSON file per spot to a Drive folder.
//! 2. Henry's Mac runs `scripts/pull-colab-cache.sh`, which syncs Drive
//!    to `data/flop-cache/raw/` and invokes `solver-cli pack-cache`
//!    (implemented here).
//! 3. The packed binary ships with Poker Panel and is loaded at runtime
//!    via `solver_nlhe::flop_cache::FlopCache::load_from_file`.
//!
//! # JSON schema (Colab → Mac)
//!
//! The Colab output for one spot is a single JSON file. Filename
//! convention: `{board}_{spr}spr_{pot_type}_{bet_tree}.json` (matches
//! the pattern `colab/precompute_flops.md` uses in Cell 4).
//!
//! ```json
//! {
//!   "board": "AhKhQh",           // 6 chars, 3 cards, parsed via Card::parse
//!   "spr_bucket": 4,             // 0..=255
//!   "pot_type": "SRP",           // "SRP" | "3BP" | "4BP" | "5BP"
//!   "bet_tree_version": 1,       // 0..=255
//!   "exploitability": 0.003,
//!   "actions": [
//!     { "label": "check",  "ev": 0.12, "weights": [0.73, 0.68, ..., 1326 floats] },
//!     { "label": "bet_33", "ev": 0.14, "weights": [...] },
//!     ...
//!   ]
//! }
//! ```
//!
//! The `label` field is purely for human debugging — the binary format
//! stores strategies by index, so label ordering in the JSON determines
//! on-disk ordering. A Day-5 Colab agent emitting this schema is
//! responsible for keeping label order stable per `(pot_type, bet_tree)`.
//!
//! # Dedup policy
//!
//! `pack_cache_dir` reads JSONs in filename-sorted order. If two JSONs
//! produce the same key (canonical_board, spr_bucket, pot_type,
//! bet_tree_version), the **later** one in sort order wins — matching
//! the "latest resolve wins" convention that falls naturally out of
//! Colab's `{board}_{spr}spr_{pot}_{bet_tree}.json` naming (rerunning
//! the same spot produces the same filename, so the later write clobbers
//! the earlier on Drive). Documented here so a Day-5 agent knows the
//! rule isn't adjustable at call time — if they want merge-of-spots
//! behavior they need a separate tool.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use solver_eval::card::Card;
use solver_nlhe::flop_cache::{
    pack_binary, CachedFlopStrategy, FlopCache, PackEntry, PotType, MAX_ACTIONS,
};

const NUM_COMBOS: usize = 1326;

/// Args for `pack-cache`. Parsed from the `clap` struct in `main.rs`.
pub struct PackCacheArgs {
    /// Directory containing per-spot JSON files.
    pub input: PathBuf,
    /// Output binary path.
    pub output: PathBuf,
    /// Format — only `"binary"` is accepted today. Kept as a parameter so
    /// a future JSON-concat format (debug-only) is straightforward to add.
    pub format: String,
}

/// Args for `unpack-cache`.
pub struct UnpackCacheArgs {
    /// Input binary path.
    pub input: PathBuf,
    /// Output directory for per-entry JSON files.
    pub output: PathBuf,
}

/// Entry point for `solver-cli pack-cache`.
pub fn run_pack_cache(args: &PackCacheArgs) -> Result<()> {
    if args.format != "binary" {
        anyhow::bail!(
            "unsupported format {:?}: only \"binary\" is accepted today",
            args.format
        );
    }
    if !args.input.is_dir() {
        anyhow::bail!(
            "input is not a directory: {} (pack-cache expects a folder of per-spot JSONs)",
            args.input.display()
        );
    }

    let entries = read_entries_from_dir(&args.input)?;
    eprintln!(
        "pack-cache: {} entries from {} -> {}",
        entries.len(),
        args.input.display(),
        args.output.display()
    );

    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent).with_context(|| {
                format!("creating output directory {}", parent.display())
            })?;
        }
    }

    pack_binary(&args.output, &entries)
        .with_context(|| format!("writing {}", args.output.display()))?;
    Ok(())
}

/// Entry point for `solver-cli unpack-cache`.
///
/// Dumps each entry of a packed binary to an individual JSON file in
/// `args.output`, using the same filename convention Colab uses.
/// Intended for debugging, auditing, and re-shipping subsets.
pub fn run_unpack_cache(args: &UnpackCacheArgs) -> Result<()> {
    let cache = FlopCache::load_from_file(&args.input)
        .with_context(|| format!("loading {}", args.input.display()))?;

    if !args.output.exists() {
        fs::create_dir_all(&args.output).with_context(|| {
            format!("creating output directory {}", args.output.display())
        })?;
    }

    // FlopCache doesn't expose its internal map for iteration — by
    // design, callers use lookup(). But unpack-cache is the one place
    // where "dump everything" is legitimate. Rather than punch a hole
    // in the public API, we re-read the bytes and iterate them here.
    // The format parser below mirrors the one in flop_cache.rs; if the
    // layout ever changes both will need updating — marked LOAD-BEARING
    // in the comment there.
    let bytes = fs::read(&args.input)
        .with_context(|| format!("re-reading {} for iteration", args.input.display()))?;
    let count = unpack_bytes_to_dir(&bytes, &args.output)?;

    eprintln!(
        "unpack-cache: wrote {count} JSON files to {} (loaded cache len={})",
        args.output.display(),
        cache.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// JSON reader: dir of per-spot JSONs  →  Vec<PackEntry>
// ---------------------------------------------------------------------------

/// Read every `*.json` in `dir`, parse it as a single flop-cache entry,
/// and deduplicate by key (later-in-sort-order wins).
fn read_entries_from_dir(dir: &Path) -> Result<Vec<PackEntry>> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| format!("reading directory {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();

    // Use a linear scan to dedupe so error messages can point at the
    // exact pair of conflicting files, which a HashMap wouldn't.
    let mut out: Vec<PackEntry> = Vec::with_capacity(paths.len());
    let mut key_to_idx: std::collections::HashMap<
        ([u8; 3], u8, u8, u8),
        (usize, PathBuf),
    > = std::collections::HashMap::new();

    for path in &paths {
        let entry = parse_entry_json(path)
            .with_context(|| format!("parsing {}", path.display()))?;
        let key = (
            entry.canonical_board,
            entry.spr_bucket,
            entry.pot_type as u8,
            entry.bet_tree_version,
        );
        if let Some((existing_idx, existing_path)) = key_to_idx.get(&key).cloned() {
            // Later wins; log the override so a human sees it during pack.
            eprintln!(
                "pack-cache: dedup: {} overrides {}",
                path.display(),
                existing_path.display()
            );
            out[existing_idx] = entry;
            key_to_idx.insert(key, (existing_idx, path.clone()));
        } else {
            key_to_idx.insert(key, (out.len(), path.clone()));
            out.push(entry);
        }
    }

    Ok(out)
}

/// Parse a single Colab-output JSON into a `PackEntry`.
fn parse_entry_json(path: &Path) -> Result<PackEntry> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let v: Value = serde_json::from_str(&text).context("JSON parse")?;

    let board_str = v
        .get("board")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing string field \"board\""))?;
    let canonical_board = parse_board_three_cards(board_str)?;

    let spr_bucket = v
        .get("spr_bucket")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("missing integer field \"spr_bucket\""))?;
    let spr_bucket: u8 = spr_bucket
        .try_into()
        .map_err(|_| anyhow!("spr_bucket {spr_bucket} does not fit in u8"))?;

    let pot_type_str = v
        .get("pot_type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing string field \"pot_type\""))?;
    let pot_type = parse_pot_type_label(pot_type_str)?;

    // bet_tree_version is optional; defaults to 1 for Day-5 Colab output
    // that predates the field. Once every JSON carries it, the default
    // can go away.
    let bet_tree_version = v
        .get("bet_tree_version")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let bet_tree_version: u8 = bet_tree_version
        .try_into()
        .map_err(|_| anyhow!("bet_tree_version {bet_tree_version} does not fit in u8"))?;

    let exploitability = v
        .get("exploitability")
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("missing number field \"exploitability\""))? as f32;

    let actions = v
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("missing array field \"actions\""))?;
    if actions.len() > MAX_ACTIONS {
        return Err(anyhow!(
            "too many actions: {} > MAX_ACTIONS={}",
            actions.len(),
            MAX_ACTIONS
        ));
    }

    let mut strategies = Vec::with_capacity(actions.len());
    let mut ev_per_action = Vec::with_capacity(actions.len());
    for (i, a) in actions.iter().enumerate() {
        let weights = a
            .get("weights")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("action[{i}]: missing array \"weights\""))?;
        if weights.len() != NUM_COMBOS {
            return Err(anyhow!(
                "action[{i}]: weights length {} != {NUM_COMBOS}",
                weights.len()
            ));
        }
        let mut row = [0.0_f32; NUM_COMBOS];
        for (j, w) in weights.iter().enumerate() {
            row[j] = w
                .as_f64()
                .ok_or_else(|| anyhow!("action[{i}].weights[{j}] is not a number"))?
                as f32;
        }
        strategies.push(row);

        let ev = a
            .get("ev")
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow!("action[{i}]: missing number \"ev\""))?
            as f32;
        ev_per_action.push(ev);
    }

    Ok(PackEntry {
        canonical_board,
        spr_bucket,
        pot_type,
        bet_tree_version,
        strategy: CachedFlopStrategy {
            strategies,
            ev_per_action,
            exploitability,
        },
    })
}

/// Parse a 6-char board string like "AhKhQh" into three Card bytes.
///
/// We do NOT canonicalize here — we trust Colab's output to already have
/// applied the suit-isomorphism canonicalization via
/// `solver_eval::iso`. If Day-5 Colab output is caught shipping a
/// non-canonical board, that's a Colab-side bug, not ours to paper over.
fn parse_board_three_cards(s: &str) -> Result<[u8; 3]> {
    if s.len() != 6 {
        return Err(anyhow!("board {:?} must be 6 chars (3 cards)", s));
    }
    let mut out = [0u8; 3];
    for i in 0..3 {
        let card_s = &s[i * 2..i * 2 + 2];
        let card =
            Card::parse(card_s).ok_or_else(|| anyhow!("bad card {:?} in board {:?}", card_s, s))?;
        out[i] = card.0;
    }
    Ok(out)
}

/// Parse `"SRP" | "3BP" | "4BP" | "5BP"` into `PotType`. Accepts the
/// labels Colab's notebook uses (see `colab/precompute_flops.md` Cell 3).
fn parse_pot_type_label(s: &str) -> Result<PotType> {
    match s {
        "SRP" => Ok(PotType::Srp),
        "3BP" => Ok(PotType::ThreeBet),
        "4BP" => Ok(PotType::FourBet),
        "5BP" => Ok(PotType::FiveBet),
        other => Err(anyhow!("unknown pot_type label: {:?}", other)),
    }
}

/// Inverse of `parse_pot_type_label` — for unpack-cache's JSON output.
fn pot_type_label(p: PotType) -> &'static str {
    match p {
        PotType::Srp => "SRP",
        PotType::ThreeBet => "3BP",
        PotType::FourBet => "4BP",
        PotType::FiveBet => "5BP",
    }
}

// ---------------------------------------------------------------------------
// unpack: binary → dir of per-entry JSONs
// ---------------------------------------------------------------------------

/// Magic bytes at the start of a flop cache file. Duplicated from
/// `solver_nlhe::flop_cache` — that module keeps them private. If we
/// need them in more than one place later, push a `pub const MAGIC` up
/// there.
const FLOP_CACHE_MAGIC: [u8; 8] = *b"PSFLOP\0\0";
const FLOP_CACHE_VERSION: u16 = 1;
const FILE_HEADER_BYTES: usize = 16;
const ENTRY_HEADER_BYTES: usize = 8;
const STRATEGY_BYTES: usize = NUM_COMBOS * 4;

fn unpack_bytes_to_dir(bytes: &[u8], out_dir: &Path) -> Result<usize> {
    if bytes.len() < FILE_HEADER_BYTES {
        return Err(anyhow!("file too short for header"));
    }
    if bytes[0..8] != FLOP_CACHE_MAGIC {
        return Err(anyhow!("bad magic bytes"));
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != FLOP_CACHE_VERSION {
        return Err(anyhow!(
            "unsupported format version: got {version}, expected {FLOP_CACHE_VERSION}"
        ));
    }
    let num_entries =
        u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;

    let mut cursor = FILE_HEADER_BYTES;
    for _ in 0..num_entries {
        let board: [u8; 3] = [bytes[cursor], bytes[cursor + 1], bytes[cursor + 2]];
        let spr_bucket = bytes[cursor + 3];
        let pot_byte = bytes[cursor + 4];
        let pot_type = match pot_byte {
            0 => PotType::Srp,
            1 => PotType::ThreeBet,
            2 => PotType::FourBet,
            3 => PotType::FiveBet,
            other => return Err(anyhow!("unknown PotType byte {other}")),
        };
        let bet_tree_version = bytes[cursor + 5];
        let num_actions = bytes[cursor + 6] as usize;
        // bytes[cursor + 7] reserved.

        let strat_off = cursor + ENTRY_HEADER_BYTES;

        let mut actions_json = Vec::with_capacity(num_actions);
        for a in 0..num_actions {
            let row_off = strat_off + a * STRATEGY_BYTES;
            let mut weights = Vec::with_capacity(NUM_COMBOS);
            for j in 0..NUM_COMBOS {
                let wo = row_off + j * 4;
                let w = f32::from_le_bytes([
                    bytes[wo],
                    bytes[wo + 1],
                    bytes[wo + 2],
                    bytes[wo + 3],
                ]);
                weights.push(w);
            }
            let ev_off =
                strat_off + num_actions * STRATEGY_BYTES + a * 4;
            let ev = f32::from_le_bytes([
                bytes[ev_off],
                bytes[ev_off + 1],
                bytes[ev_off + 2],
                bytes[ev_off + 3],
            ]);
            actions_json.push(json!({
                "label": format!("action_{a}"),
                "ev": ev,
                "weights": weights,
            }));
        }
        let expl_off =
            strat_off + num_actions * STRATEGY_BYTES + num_actions * 4;
        let exploitability = f32::from_le_bytes([
            bytes[expl_off],
            bytes[expl_off + 1],
            bytes[expl_off + 2],
            bytes[expl_off + 3],
        ]);

        let board_str = format!(
            "{}{}{}",
            Card(board[0]),
            Card(board[1]),
            Card(board[2]),
        );
        let obj = json!({
            "board": board_str,
            "spr_bucket": spr_bucket,
            "pot_type": pot_type_label(pot_type),
            "bet_tree_version": bet_tree_version,
            "exploitability": exploitability,
            "actions": actions_json,
        });

        let fname = format!(
            "{}_{}spr_{}_bt{}.json",
            board_str,
            spr_bucket,
            pot_type_label(pot_type),
            bet_tree_version,
        );
        let out_path = out_dir.join(fname);
        fs::write(&out_path, serde_json::to_string_pretty(&obj)?)
            .with_context(|| format!("writing {}", out_path.display()))?;

        cursor = strat_off + num_actions * STRATEGY_BYTES + num_actions * 4 + 4;
    }
    Ok(num_entries)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(label: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!(
            "poker-solver-pack-cache-test-{label}-{pid}-{nanos}"
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_colab_json(dir: &Path, name: &str, board: &str, spr: u8, pot: &str) -> PathBuf {
        let mut weights = Vec::with_capacity(NUM_COMBOS);
        for i in 0..NUM_COMBOS {
            weights.push((i as f32) * 1e-4);
        }
        let obj = json!({
            "board": board,
            "spr_bucket": spr,
            "pot_type": pot,
            "bet_tree_version": 1,
            "exploitability": 0.005,
            "actions": [
                { "label": "check",  "ev": 0.12, "weights": weights.clone() },
                { "label": "bet_33", "ev": 0.09, "weights": weights },
            ]
        });
        let path = dir.join(format!("{name}.json"));
        fs::write(&path, serde_json::to_string_pretty(&obj).unwrap()).unwrap();
        path
    }

    #[test]
    fn board_parser_accepts_6_char() {
        let b = parse_board_three_cards("AhKhQh").unwrap();
        assert_eq!(b[0], Card::parse("Ah").unwrap().0);
        assert_eq!(b[1], Card::parse("Kh").unwrap().0);
        assert_eq!(b[2], Card::parse("Qh").unwrap().0);
    }

    #[test]
    fn board_parser_rejects_bad_length() {
        assert!(parse_board_three_cards("AhKh").is_err());
        assert!(parse_board_three_cards("AhKhQhAh").is_err());
    }

    #[test]
    fn board_parser_rejects_bad_card() {
        assert!(parse_board_three_cards("XxYyZz").is_err());
    }

    #[test]
    fn pot_type_labels_roundtrip() {
        for p in [
            PotType::Srp,
            PotType::ThreeBet,
            PotType::FourBet,
            PotType::FiveBet,
        ] {
            let label = pot_type_label(p);
            assert_eq!(parse_pot_type_label(label).unwrap() as u8, p as u8);
        }
    }

    #[test]
    fn pack_dir_happy_path() {
        let dir = tmp_dir("happy");
        write_colab_json(&dir, "AhKhQh_4spr_SRP_default_3", "AhKhQh", 4, "SRP");
        write_colab_json(&dir, "AhKhQh_8spr_3BP_default_3", "AhKhQh", 8, "3BP");

        let out = dir.join("out.bin");
        let args = PackCacheArgs {
            input: dir.clone(),
            output: out.clone(),
            format: "binary".to_string(),
        };
        run_pack_cache(&args).unwrap();

        // Load back and verify both entries exist.
        let cache = FlopCache::load_from_file(&out).unwrap();
        assert_eq!(cache.len(), 2);
        let board = parse_board_three_cards("AhKhQh").unwrap();
        assert!(cache.lookup(&board, 4, PotType::Srp).is_some());
        assert!(cache.lookup(&board, 8, PotType::ThreeBet).is_some());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pack_dir_dedupes_same_key() {
        let dir = tmp_dir("dedup");
        // Two different filenames, same key. Sort order is
        // alphabetical, so `b_` wins over `a_`.
        write_colab_json(&dir, "a_first", "AhKhQh", 4, "SRP");
        write_colab_json(&dir, "b_second", "AhKhQh", 4, "SRP");

        let out = dir.join("out.bin");
        let args = PackCacheArgs {
            input: dir.clone(),
            output: out.clone(),
            format: "binary".to_string(),
        };
        run_pack_cache(&args).unwrap();

        let cache = FlopCache::load_from_file(&out).unwrap();
        // Exactly one entry survives: the dedup collapsed them.
        assert_eq!(cache.len(), 1);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pack_rejects_unknown_format() {
        let dir = tmp_dir("fmt");
        let out = dir.join("out.bin");
        let args = PackCacheArgs {
            input: dir.clone(),
            output: out,
            format: "json-bundle".to_string(),
        };
        assert!(run_pack_cache(&args).is_err());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unpack_then_pack_roundtrip() {
        // Pack a 2-entry dir, unpack to a scratch dir, then re-pack the
        // scratch dir. The two packed binaries should have the same
        // entry count; bitwise equality is NOT guaranteed because
        // unpack renames files, so we verify via FlopCache comparison.
        let dir_a = tmp_dir("rt-a");
        write_colab_json(&dir_a, "a_first", "AhKhQh", 4, "SRP");
        write_colab_json(&dir_a, "b_second", "2c3d4h", 8, "3BP");

        let bin_1 = dir_a.join("pack1.bin");
        run_pack_cache(&PackCacheArgs {
            input: dir_a.clone(),
            output: bin_1.clone(),
            format: "binary".to_string(),
        })
        .unwrap();

        let dir_b = tmp_dir("rt-b");
        run_unpack_cache(&UnpackCacheArgs {
            input: bin_1.clone(),
            output: dir_b.clone(),
        })
        .unwrap();

        let bin_2 = dir_a.join("pack2.bin");
        run_pack_cache(&PackCacheArgs {
            input: dir_b.clone(),
            output: bin_2.clone(),
            format: "binary".to_string(),
        })
        .unwrap();

        let cache_1 = FlopCache::load_from_file(&bin_1).unwrap();
        let cache_2 = FlopCache::load_from_file(&bin_2).unwrap();
        assert_eq!(cache_1.len(), cache_2.len());

        fs::remove_dir_all(&dir_a).ok();
        fs::remove_dir_all(&dir_b).ok();
    }
}
