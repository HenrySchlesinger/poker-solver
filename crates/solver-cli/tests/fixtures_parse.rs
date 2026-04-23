//! Schema-validates every `spot_NNN.json` in `tests/fixtures/`.
//!
//! This test is intentionally **only** a schema check — it does not call
//! the solver, it does not run CFR+, it does not compare against any
//! reference. It exists so that when someone adds or edits a fixture,
//! they learn immediately if the JSON is malformed, instead of finding
//! out later when Agent A14's differential runner blows up with a
//! cryptic serde error.
//!
//! The schema is documented in `tests/fixtures/SCHEMA.md`; keep the two
//! in sync.
//!
//! This file is owned by Agent A15 (fixtures). The validation runner
//! that actually solves each fixture and diffs against TexasSolver lives
//! in Agent A14's crate (`solver-cli validate --spot <file>`).
//!
//! # Dep note
//!
//! We deliberately use `serde_json::Value` (already in `solver-cli`'s
//! dependencies) instead of deriving `Deserialize` on a struct. Adding
//! `serde` with `derive` here would require touching `solver-cli`'s
//! `Cargo.toml`, which the fixtures-agent task spec forbids. The
//! trade-off is slightly more verbose validation code — acceptable.

use std::fs;
use std::path::Path;

use serde_json::Value;
use solver_eval::card::Card;
use solver_nlhe::range::Range;

// ---------------------------------------------------------------------------
// The test itself
// ---------------------------------------------------------------------------

/// Locate the fixtures directory relative to the crate root.
///
/// `CARGO_MANIFEST_DIR` points at `crates/solver-cli/`, so the fixtures
/// live next door. This avoids depending on the current-working-directory
/// when `cargo test` is invoked from elsewhere in the workspace.
fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Required char-length of a `board` string for each street.
/// 3 cards on the flop → 6 chars; 4 on turn → 8; 5 on river → 10.
fn expected_board_chars(street: &str) -> Option<usize> {
    match street {
        "flop" => Some(6),
        "turn" => Some(8),
        "river" => Some(10),
        _ => None,
    }
}

/// Parse a concatenated board string like `"AhKd2cQc4d"` into a vec of
/// `Card`. Returns an error (string) on any malformed card.
fn parse_board(board: &str) -> Result<Vec<Card>, String> {
    if !board.is_ascii() {
        return Err(format!("board contains non-ASCII: {board:?}"));
    }
    if board.len() % 2 != 0 {
        return Err(format!("board length {} not even: {board:?}", board.len()));
    }
    let mut out = Vec::with_capacity(board.len() / 2);
    let bytes = board.as_bytes();
    for chunk_start in (0..bytes.len()).step_by(2) {
        let chunk = std::str::from_utf8(&bytes[chunk_start..chunk_start + 2])
            .map_err(|e| format!("board slice utf8: {e}"))?;
        let card = Card::parse(chunk)
            .ok_or_else(|| format!("bad card {chunk:?} in board {board:?}"))?;
        out.push(card);
    }
    Ok(out)
}

/// Small helpers to extract typed fields from a JSON object or panic
/// with a descriptive message. Keeping these near the call sites makes
/// the validation flow readable.
fn require_str<'a>(obj: &'a Value, field: &str, ctx: &str) -> &'a str {
    obj.get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{ctx}: missing or non-string field {field:?}"))
}

fn require_obj<'a>(obj: &'a Value, field: &str, ctx: &str) -> &'a Value {
    let v = obj
        .get(field)
        .unwrap_or_else(|| panic!("{ctx}: missing object field {field:?}"));
    assert!(v.is_object(), "{ctx}: field {field:?} is not an object");
    v
}

fn require_u32(obj: &Value, field: &str, ctx: &str) -> u32 {
    let n = obj
        .get(field)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{ctx}: missing or non-integer field {field:?}"));
    u32::try_from(n)
        .unwrap_or_else(|_| panic!("{ctx}: field {field:?} value {n} exceeds u32"))
}

fn require_f32(obj: &Value, field: &str, ctx: &str) -> f32 {
    let n = obj
        .get(field)
        .and_then(Value::as_f64)
        .unwrap_or_else(|| panic!("{ctx}: missing or non-number field {field:?}"));
    n as f32
}

/// Whitelist of top-level field names. A stray unknown key (e.g. a typo
/// like `"decription"`) causes this check to fail, which is the whole
/// point — we can't silently drop half a fixture's data.
const TOP_LEVEL_FIELDS: &[&str] = &[
    "id",
    "name",
    "description",
    "street",
    "input",
    "iterations",
    "tolerances",
    "expected_reference",
    "expected_notes",
];

const INPUT_FIELDS: &[&str] = &[
    "board",
    "hero_range",
    "villain_range",
    "pot",
    "effective_stack",
    "to_act",
    "bet_tree",
];

const TOLERANCE_FIELDS: &[&str] = &["action_freq_abs", "ev_bb_abs"];

fn assert_only_known_fields(obj: &Value, allowed: &[&str], ctx: &str) {
    let map = obj
        .as_object()
        .unwrap_or_else(|| panic!("{ctx}: expected JSON object"));
    for key in map.keys() {
        assert!(
            allowed.contains(&key.as_str()),
            "{ctx}: unknown field {key:?} (allowed: {allowed:?})"
        );
    }
}

/// Check every requirement from SCHEMA.md's "Parse-test contract" section.
/// Panics on failure — `#[test]` consumes that.
fn validate_fixture(path: &Path, fixture: &Value) {
    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_else(|| panic!("bad file stem: {path:?}"));
    let ctx = file_stem.to_string();

    // Top-level shape check: all required fields present, no unknown fields.
    assert_only_known_fields(fixture, TOP_LEVEL_FIELDS, &ctx);

    let id = require_str(fixture, "id", &ctx);
    let name = require_str(fixture, "name", &ctx);
    let description = require_str(fixture, "description", &ctx);
    let street = require_str(fixture, "street", &ctx);
    let input = require_obj(fixture, "input", &ctx);
    let iterations = require_u32(fixture, "iterations", &ctx);
    let tolerances = require_obj(fixture, "tolerances", &ctx);
    let expected_reference = require_str(fixture, "expected_reference", &ctx);
    let expected_notes = require_str(fixture, "expected_notes", &ctx);

    assert_only_known_fields(input, INPUT_FIELDS, &format!("{ctx}.input"));
    assert_only_known_fields(
        tolerances,
        TOLERANCE_FIELDS,
        &format!("{ctx}.tolerances"),
    );

    // (3) id matches filename stem.
    assert_eq!(
        id, file_stem,
        "id {id:?} must match filename stem {file_stem:?} (path: {path:?})"
    );

    // (4) street enumeration.
    let board_len = expected_board_chars(street).unwrap_or_else(|| {
        panic!("{ctx}: street {street:?} must be one of flop|turn|river")
    });

    // (5) to_act enumeration.
    let to_act = require_str(input, "to_act", &format!("{ctx}.input"));
    assert!(
        matches!(to_act, "hero" | "villain"),
        "{ctx}: to_act {to_act:?} must be 'hero' or 'villain'"
    );

    // (6) board length matches street.
    let board = require_str(input, "board", &format!("{ctx}.input"));
    assert_eq!(
        board.len(),
        board_len,
        "{ctx}: board {board:?} length {} does not match street {street:?} (expected {board_len})",
        board.len(),
    );

    // (7) each board card parses.
    let cards = parse_board(board).unwrap_or_else(|e| panic!("{ctx}: {e}"));

    // No duplicate board cards.
    let mut seen = std::collections::HashSet::new();
    for c in &cards {
        assert!(
            seen.insert(c.0),
            "{ctx}: duplicate card {:?} in board {board:?}",
            c
        );
    }

    // (8) ranges parse via the real parser.
    let hero_range = require_str(input, "hero_range", &format!("{ctx}.input"));
    let villain_range = require_str(input, "villain_range", &format!("{ctx}.input"));
    Range::parse(hero_range).unwrap_or_else(|e| {
        panic!("{ctx}: hero_range parse failed: {e} (range = {hero_range:?})")
    });
    Range::parse(villain_range).unwrap_or_else(|e| {
        panic!("{ctx}: villain_range parse failed: {e} (range = {villain_range:?})")
    });

    // (9) pot / stack positivity.
    let pot = require_u32(input, "pot", &format!("{ctx}.input"));
    let stack = require_u32(input, "effective_stack", &format!("{ctx}.input"));
    assert!(pot > 0, "{ctx}: pot must be > 0");
    assert!(stack > 0, "{ctx}: effective_stack must be > 0");

    // (10) iterations positivity.
    assert!(iterations > 0, "{ctx}: iterations must be > 0");

    // Tolerances sanity.
    let action_freq_abs =
        require_f32(tolerances, "action_freq_abs", &format!("{ctx}.tolerances"));
    let ev_bb_abs = require_f32(tolerances, "ev_bb_abs", &format!("{ctx}.tolerances"));
    assert!(
        action_freq_abs > 0.0 && action_freq_abs <= 1.0,
        "{ctx}: action_freq_abs must be in (0, 1], got {action_freq_abs}"
    );
    assert!(
        ev_bb_abs > 0.0,
        "{ctx}: ev_bb_abs must be > 0, got {ev_bb_abs}"
    );

    // Bet-tree: only one preset exists today.
    let bet_tree = require_str(input, "bet_tree", &format!("{ctx}.input"));
    assert_eq!(
        bet_tree, "default_v0_1",
        "{ctx}: bet_tree {bet_tree:?} is not a known preset (only default_v0_1 exists)"
    );

    // Reference is always texassolver for v0.1.
    assert_eq!(
        expected_reference, "texassolver",
        "{ctx}: expected_reference {expected_reference:?} unexpected (only texassolver supported)"
    );

    // Non-empty string fields (catch empty-description copy-paste bugs).
    assert!(!name.is_empty(), "{ctx}: name empty");
    assert!(!description.is_empty(), "{ctx}: description empty");
    assert!(!expected_notes.is_empty(), "{ctx}: expected_notes empty");
}

/// List every `spot_NNN.json` in the fixtures dir, sorted by name.
fn list_spot_files() -> Vec<std::path::PathBuf> {
    let dir = fixtures_dir();
    let mut out = Vec::new();
    let entries =
        fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}"));
    for entry in entries {
        let entry = entry.expect("read entry");
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with("spot_") && name.ends_with(".json") {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[test]
fn every_fixture_parses_and_validates() {
    let files = list_spot_files();
    assert!(
        !files.is_empty(),
        "no spot_*.json fixtures found in {:?}",
        fixtures_dir()
    );

    let mut seen_ids = std::collections::HashSet::new();
    for path in &files {
        let text = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let fixture: Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("parse {path:?}: {e}"));
        validate_fixture(path, &fixture);
        let id = fixture
            .get("id")
            .and_then(Value::as_str)
            .expect("id is a string (already checked)")
            .to_string();
        assert!(
            seen_ids.insert(id.clone()),
            "duplicate fixture id {id:?} (path: {path:?})"
        );
    }
}

#[test]
fn twenty_canonical_fixtures_exist() {
    // The spec calls for exactly 20 canonical spots, spot_001 .. spot_020.
    // This test guards against accidental deletions or gaps (spot_007
    // missing, etc.). If we ever intentionally grow past 20, update this
    // test — it's deliberately rigid.
    let files = list_spot_files();
    assert_eq!(
        files.len(),
        20,
        "expected exactly 20 canonical fixtures, found {}: {:?}",
        files.len(),
        files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    );
    for n in 1..=20 {
        let expected = fixtures_dir().join(format!("spot_{n:03}.json"));
        assert!(
            expected.exists(),
            "missing canonical fixture: {expected:?}"
        );
    }
}
