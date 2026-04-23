//! `translate-fixture` subcommand: convert our fixture JSON into a
//! TexasSolver `.tsconfig` config file.
//!
//! ## Why this exists (Rust, not Python)
//!
//! The differential-testing harness (Agent A14) compares our solver's
//! output to [TexasSolver](https://github.com/bupticybee/TexasSolver)
//! on canonical fixtures. TexasSolver's console binary does not consume
//! our fixture format — it consumes its own line-based imperative config.
//! So we need a translator.
//!
//! An earlier draft lived at `scripts/translate_fixture.py`. Project rule
//! (see root `CLAUDE.md` §"Rust wherever possible"): dev tools live in
//! `solver-cli`, not in stray Python scripts. This module is the
//! replacement. The Python script is removed (or reduced to a one-line
//! shim that execs this binary).
//!
//! ## Input: our fixture JSON
//!
//! The schema is defined by Agent A15 in
//! `crates/solver-cli/tests/fixtures/SCHEMA.md`. Relevant fields for
//! translation:
//!
//! | Field | Used for |
//! |---|---|
//! | `input.board` | concatenated cards, e.g. `"AhKd2c"` (6/8/10 chars) |
//! | `input.hero_range` | range string in our parser's notation |
//! | `input.villain_range` | ditto |
//! | `input.pot` | pot in chips |
//! | `input.effective_stack` | stack in chips |
//! | `input.to_act` | `"hero"` or `"villain"` |
//! | `input.bet_tree` | preset name, only `"default_v0_1"` is known |
//! | `street` | `"flop"`, `"turn"`, `"river"` — controls which bet-size lines get emitted |
//! | `iterations` | cap passed as `set_max_iteration` |
//!
//! ## Output: TexasSolver `.tsconfig`
//!
//! Line-based commands, one per line. The exact command set is
//! documented on the [console branch README](https://github.com/bupticybee/TexasSolver/tree/console).
//! We emit (in this order):
//!
//! ```text
//! set_pot <pot>
//! set_effective_stack <stack>
//! set_board <C,C,C[,C[,C]]>
//! set_range_ip <range>
//! set_range_oop <range>
//! set_bet_sizes oop,flop,bet,33,66,100      (only if flop still live)
//! set_bet_sizes oop,flop,raise,33,66,100
//! set_bet_sizes oop,flop,allin
//! set_bet_sizes ip,flop,bet,33,66,100
//! set_bet_sizes ip,flop,raise,33,66,100
//! set_bet_sizes ip,flop,allin
//! ...turn bet-size lines if turn still live...
//! ...river bet-size lines...
//! set_allin_threshold 0.67
//! build_tree
//! set_thread_num <N>
//! set_accuracy 0.3
//! set_max_iteration <iterations>
//! set_print_interval 10
//! set_use_isomorphism 1
//! start_solve
//! set_dump_rounds 2
//! dump_result <path>
//! ```
//!
//! **Format quirks discovered against TexasSolver's actual parser**
//! (see `vendor/TexasSolver/src/tools/CommandLineTool.cpp` and
//! `vendor/TexasSolver/src/tools/PrivateRangeConverter.cpp`):
//!
//! 1. **Two-token cap per line.** `CommandLineTool::processCommand`
//!    splits each line on a single space and errors with
//!    `command not valid` if the result has more than two tokens.
//!    Consequences:
//!    - **No comment lines.** `# auto-generated ...` is rejected.
//!      Fixture identity is preserved via the output filename
//!      (e.g. `spot_001.tsconfig`) and the dumped result JSON,
//!      not inline comments.
//!    - **No spaces inside range strings.** Our fixtures write the
//!      human-friendly `"AA, KK, AKs"` form; we strip every
//!      whitespace character before emitting. TexasSolver's sample
//!      input uses the no-space form `"AA,KK,AKs"`.
//! 2. **Length-2-or-3 tokens only in ranges.**
//!    `PrivateRangeConverter::rangeStr2Cards` only understands
//!    tokens of length 2 (`"AA"`, `"AK"`) or 3 (`"AKs"`, `"AKo"`).
//!    It rejects the compound forms our fixtures use — `"77-TT"`,
//!    `"22+"`, `"T9s+"` — with `range str ... len not valid`. We
//!    expand those to explicit comma-lists before emitting; see
//!    `normalize_range_for_texassolver`.
//!
//! ## Design decisions (same judgment calls the Python script made)
//!
//! 1. **hero/villain → ip/oop mapping.** TexasSolver thinks in positions
//!    (IP/OOP); our fixtures think in hero/villain. We assume
//!    **hero = OOP** by default — the most common study convention is
//!    BB (hero) defending vs BTN raise (villain=IP). If a fixture's
//!    `to_act = "villain"`, that doesn't change the position — it only
//!    changes whose turn it is, which TexasSolver infers from the tree
//!    and the street, not from our config. So `to_act` is informational
//!    for us and does **not** swap IP/OOP.
//!
//! 2. **Bet sizes.** Only `"default_v0_1"` is a known preset today. We
//!    hard-code its shape here (flop: 33/66/100, turn: 50/100/200,
//!    river: 33/66/100/200; plus allin on every street). Keeping this
//!    in sync with `solver-nlhe::bet_tree::BetTree::default_v0_1` is a
//!    manual discipline; the integration tests in
//!    `tests/translate_fixture.rs` assert the numbers match.
//!
//! 3. **Streets emitted.** We only emit bet-size lines for streets that
//!    still have live betting given the board length. A river fixture
//!    (5-card board) gets only `river` lines; a turn fixture gets
//!    `turn` and `river`; a flop fixture gets all three.
//!
//! 4. **Accuracy / iteration cap.** TexasSolver takes both
//!    `set_accuracy` (exploitability target) and `set_max_iteration`
//!    (hard cap). Our fixtures specify `iterations` (a count) and leave
//!    accuracy implicit. We set accuracy to `0.3` — loose enough that
//!    the iteration cap always dominates, giving us reproducible
//!    solve depth across runs.
//!
//! 5. **Thread count.** TexasSolver old builds default to 1 thread.
//!    We default to a reasonable parallel number so the oracle runs
//!    roughly at parity with our solver on the same machine. Default
//!    picked via [`std::thread::available_parallelism`] with a
//!    minimum of 1.
//!
//! 6. **Unknown bet-tree presets.** Reject loudly. We don't silently
//!    default — a typo in the fixture should fail the translator, not
//!    produce a sneaky wrong config.

use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

/// Output-format selector. Today only TexasSolver's line-based config is
/// supported; leaving the enum in place so adding (say) Piosolver in the
/// future is a drop-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFormat {
    TexasSolver,
}

impl TargetFormat {
    /// Parse a `--format` argument. Returns `Err` for anything unknown so
    /// the user sees a clear error rather than a silent default.
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "texassolver" | "ts" => Ok(Self::TexasSolver),
            other => bail!("unknown --format {:?} (supported: \"texassolver\")", other),
        }
    }
}

/// Arguments for `solver-cli translate-fixture`, parsed up front so the
/// translator itself is unit-testable without going through `clap`.
#[derive(Debug)]
pub struct TranslateArgs {
    /// Path to the input fixture JSON.
    pub input: String,
    /// Path to write the translated config to.
    pub output: String,
    /// Target solver format.
    pub format: TargetFormat,
    /// Path that TexasSolver should dump its result JSON to (baked into
    /// the emitted config as `dump_result <path>`).
    pub dump_path: String,
}

/// Run the translation end-to-end: read the fixture file, translate,
/// write the output config.
pub fn run_translate(args: &TranslateArgs) -> Result<()> {
    let fixture_text = std::fs::read_to_string(&args.input)
        .with_context(|| format!("read fixture {:?}", args.input))?;
    let fixture: Value = serde_json::from_str(&fixture_text)
        .with_context(|| format!("parse fixture JSON {:?}", args.input))?;

    let config = match args.format {
        TargetFormat::TexasSolver => translate_to_texassolver(&fixture, &args.dump_path)
            .with_context(|| format!("translate fixture {:?}", args.input))?,
    };

    write_output(&args.output, &config)
        .with_context(|| format!("write output {:?}", args.output))?;
    Ok(())
}

/// Write the config string to the output path. If the path is `-`, writes
/// to stdout.
fn write_output(path: &str, config: &str) -> Result<()> {
    if path == "-" {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        lock.write_all(config.as_bytes())?;
        lock.flush()?;
        return Ok(());
    }
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir for {path:?}"))?;
        }
    }
    std::fs::write(path, config)?;
    Ok(())
}

/// Produce the TexasSolver line-based config from one of our fixtures.
///
/// `dump_path` is interpolated verbatim into the emitted `dump_result`
/// line. It's the caller's responsibility to pick a path the runner
/// will be able to write to.
pub fn translate_to_texassolver(fixture: &Value, dump_path: &str) -> Result<String> {
    let root = fixture
        .as_object()
        .ok_or_else(|| anyhow!("fixture is not a JSON object"))?;

    // Top-level required fields. We don't schema-check everything the
    // parse test checks (that's `fixtures_parse.rs`'s job) — we only
    // check the fields we actually read.
    let id = require_str(root, "id")?;
    let street = require_str(root, "street")?;
    let iterations = require_u32(root, "iterations")?;

    let input = root
        .get("input")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("fixture {id:?}: missing 'input' object"))?;

    let board = require_str_field(input, "board", "input")?;
    let hero_range = require_str_field(input, "hero_range", "input")?;
    let villain_range = require_str_field(input, "villain_range", "input")?;
    let pot = require_u32_field(input, "pot", "input")?;
    let effective_stack = require_u32_field(input, "effective_stack", "input")?;
    let to_act = require_str_field(input, "to_act", "input")?;
    let bet_tree_name = require_str_field(input, "bet_tree", "input")?;

    // Sanity: bet-tree preset must be known. Reject unknown loudly.
    if bet_tree_name != "default_v0_1" {
        bail!(
            "fixture {id:?}: unknown bet_tree preset {:?} (only \"default_v0_1\" is supported)",
            bet_tree_name
        );
    }

    // Sanity: street / to_act enumerations. Duplicate of the parse-test
    // checks, but we do them here so the translator can be used on ad-hoc
    // fixtures that haven't been through the test harness.
    if !matches!(street, "flop" | "turn" | "river") {
        bail!(
            "fixture {id:?}: street {:?} must be flop/turn/river",
            street
        );
    }
    if !matches!(to_act, "hero" | "villain") {
        bail!("fixture {id:?}: to_act {:?} must be hero/villain", to_act);
    }

    // Board length must agree with street.
    let expected_chars = match street {
        "flop" => 6,
        "turn" => 8,
        "river" => 10,
        _ => unreachable!(),
    };
    if board.len() != expected_chars {
        bail!(
            "fixture {id:?}: board {:?} has length {} but street {} requires {}",
            board,
            board.len(),
            street,
            expected_chars
        );
    }

    let board_csv = format_board_csv(board)?;

    // Position mapping. hero = OOP by default; see module-level docs
    // for the rationale.
    //
    // We normalize both ranges through `normalize_range_for_texassolver`
    // which (a) strips whitespace (required: TexasSolver's line parser
    // rejects >2-token lines) and (b) expands compound tokens like
    // `"77-TT"` and `"22+"` into explicit comma-lists (required:
    // TexasSolver's range parser only understands length-2/3 tokens).
    let oop_range = normalize_range_for_texassolver(hero_range)
        .with_context(|| format!("fixture {id:?}: hero_range"))?;
    let ip_range = normalize_range_for_texassolver(villain_range)
        .with_context(|| format!("fixture {id:?}: villain_range"))?;

    // Thread count: default to available parallelism, min 1.
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);

    // Build output line-by-line.
    //
    // IMPORTANT: we do NOT emit comment lines. TexasSolver's parser
    // rejects any line with more than two space-separated tokens, so
    // `# auto-generated ...` would crash the binary with
    // `command not valid: # ...`. Fixture identity is preserved via
    // the output filename (e.g. `spot_001.tsconfig`) and the dumped
    // result JSON.
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("set_pot {pot}"));
    lines.push(format!("set_effective_stack {effective_stack}"));
    lines.push(format!("set_board {board_csv}"));
    lines.push(format!("set_range_ip {ip_range}"));
    lines.push(format!("set_range_oop {oop_range}"));

    // Bet-size lines: only for streets that still have live betting.
    for (st, sizes) in streets_to_emit(street) {
        let size_csv = join_u32(sizes);
        // oop sizings
        lines.push(format!("set_bet_sizes oop,{st},bet,{size_csv}"));
        lines.push(format!("set_bet_sizes oop,{st},raise,{size_csv}"));
        lines.push(format!("set_bet_sizes oop,{st},allin"));
        // ip sizings
        lines.push(format!("set_bet_sizes ip,{st},bet,{size_csv}"));
        lines.push(format!("set_bet_sizes ip,{st},raise,{size_csv}"));
        lines.push(format!("set_bet_sizes ip,{st},allin"));
    }

    lines.push("set_allin_threshold 0.67".into());
    lines.push("build_tree".into());
    lines.push(format!("set_thread_num {threads}"));
    lines.push("set_accuracy 0.3".into());
    lines.push(format!("set_max_iteration {iterations}"));
    lines.push("set_print_interval 10".into());
    lines.push("set_use_isomorphism 1".into());
    lines.push("start_solve".into());
    lines.push("set_dump_rounds 2".into());
    lines.push(format!("dump_result {dump_path}"));

    let mut out = lines.join("\n");
    out.push('\n');
    Ok(out)
}

// ---------------------------------------------------------------------------
// Bet-tree preset (must match solver-nlhe::bet_tree::BetTree::default_v0_1)
// ---------------------------------------------------------------------------

/// Pot-fraction percentages per street for the `default_v0_1` preset.
///
/// **Keep in sync** with `crates/solver-nlhe/src/bet_tree.rs`:
/// - flop: 33%, 66%, 100% pot (+ all-in, emitted separately)
/// - turn: 50%, 100%, 200% pot
/// - river: 33%, 66%, 100%, 200% pot
///
/// The all-in bucket is NOT listed here — it's emitted as its own
/// `set_bet_sizes oop,<street>,allin` line.
const FLOP_PCT: &[u32] = &[33, 66, 100];
const TURN_PCT: &[u32] = &[50, 100, 200];
const RIVER_PCT: &[u32] = &[33, 66, 100, 200];

/// Which streets still have live betting given the street the fixture
/// starts on.
///
/// A flop fixture: flop, turn, river are all live.
/// A turn fixture: turn, river.
/// A river fixture: river only.
fn streets_to_emit(starting_street: &str) -> Vec<(&'static str, &'static [u32])> {
    match starting_street {
        "flop" => vec![("flop", FLOP_PCT), ("turn", TURN_PCT), ("river", RIVER_PCT)],
        "turn" => vec![("turn", TURN_PCT), ("river", RIVER_PCT)],
        "river" => vec![("river", RIVER_PCT)],
        _ => unreachable!("caller validates street enum"),
    }
}

fn join_u32(nums: &[u32]) -> String {
    nums.iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

// ---------------------------------------------------------------------------
// Range normalization
// ---------------------------------------------------------------------------

/// Convert one of our fixture range strings into a form TexasSolver's
/// `PrivateRangeConverter::rangeStr2Cards` can consume.
///
/// TexasSolver's range parser only understands tokens of length 2
/// (`"AA"`, `"AK"`) or 3 (`"AKs"`, `"AKo"`). Anything longer is
/// rejected with `range str ... len not valid`. Our fixtures use
/// several compact forms that don't fit:
///
/// | Our token | Expanded to                                            |
/// |-----------|--------------------------------------------------------|
/// | `RR-SS`   | `RR,(R+1)(R+1),...,SS` (pair range, inclusive)          |
/// | `RR+`     | `RR,(R+1)(R+1),...,AA` (all pairs ≥ RR)                 |
/// | `RR-`     | `22,33,...,RR` (all pairs ≤ RR)                         |
/// | `XYs+`    | `XYs,(X+1)Ys,...,AYs` (kicker fixed, first rank up)     |
/// | `XYo+`    | `XYo,(X+1)Yo,...,AYo` (ditto, offsuit)                  |
///
/// `:weight` suffixes (e.g. `"88:0.75"`) pass through to every
/// sub-token we emit.
///
/// Finally we strip all whitespace, because TexasSolver's line-level
/// parser (`CommandLineTool::processCommand`) rejects any line with
/// more than two space-separated tokens — our fixtures' `"AA, KK"`
/// form would crash otherwise.
///
/// Returns `Err` on syntactically unrecognized tokens — better to
/// fail loudly than to emit a silently-wrong config.
fn normalize_range_for_texassolver(s: &str) -> Result<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in s.split(',') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        // Split off optional `:weight` suffix; we'll re-attach it to
        // every sub-token we emit.
        let (body, weight) = match token.rsplit_once(':') {
            Some((b, w)) => (b.trim(), Some(w.trim())),
            None => (token, None),
        };
        let emit = |sub: &str| match weight {
            Some(w) => format!("{sub}:{w}"),
            None => sub.to_string(),
        };
        let bytes = body.as_bytes();

        // Case: pair range `RR-SS` (length 5) — e.g. "77-TT".
        if bytes.len() == 5 && bytes[2] == b'-' && bytes[0] == bytes[1] && bytes[3] == bytes[4] {
            let a =
                rank_value(bytes[0] as char).ok_or_else(|| anyhow!("bad rank in {:?}", token))?;
            let b =
                rank_value(bytes[3] as char).ok_or_else(|| anyhow!("bad rank in {:?}", token))?;
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            for r in lo..=hi {
                let ch = rank_char(r);
                out.push(emit(&format!("{ch}{ch}")));
            }
            continue;
        }
        // Case: pair ≥ `RR+` (length 3) — e.g. "22+".
        if bytes.len() == 3 && bytes[0] == bytes[1] && bytes[2] == b'+' {
            let lo =
                rank_value(bytes[0] as char).ok_or_else(|| anyhow!("bad rank in {:?}", token))?;
            for r in lo..=12 {
                let ch = rank_char(r);
                out.push(emit(&format!("{ch}{ch}")));
            }
            continue;
        }
        // Case: pair ≤ `RR-` (length 3) — e.g. "JJ-".
        if bytes.len() == 3 && bytes[0] == bytes[1] && bytes[2] == b'-' {
            let hi =
                rank_value(bytes[0] as char).ok_or_else(|| anyhow!("bad rank in {:?}", token))?;
            for r in 0..=hi {
                let ch = rank_char(r);
                out.push(emit(&format!("{ch}{ch}")));
            }
            continue;
        }
        // Case: `XYs+` / `XYo+` (length 4) — kicker fixed, first
        // rank iterates up through Ace.
        if bytes.len() == 4 && bytes[3] == b'+' && (bytes[2] == b's' || bytes[2] == b'o') {
            let kicker =
                rank_value(bytes[1] as char).ok_or_else(|| anyhow!("bad rank in {:?}", token))?;
            let start =
                rank_value(bytes[0] as char).ok_or_else(|| anyhow!("bad rank in {:?}", token))?;
            let kicker_char = rank_char(kicker);
            let suffix = bytes[2] as char;
            // Skip the top-rank==kicker step (would emit nonsense
            // like "99s" — a pair with a suit suffix).
            for r in start..=12 {
                if r == kicker {
                    continue;
                }
                let first = rank_char(r);
                out.push(emit(&format!("{first}{kicker_char}{suffix}")));
            }
            continue;
        }
        // Length 2 or 3 — TexasSolver can parse directly.
        if bytes.len() == 2 || bytes.len() == 3 {
            out.push(emit(body));
            continue;
        }
        bail!(
            "unrecognized range token {:?} in range string {:?}",
            token,
            s
        );
    }
    let joined = out.join(",");
    Ok(strip_whitespace(&joined))
}

/// Remove every whitespace character from `s`.
///
/// TexasSolver's `CommandLineTool::processCommand` splits each line
/// on a single space and rejects lines with more than two tokens.
/// Ranges must therefore contain no embedded whitespace.
fn strip_whitespace(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Map rank 0..13 to its character. 0 = '2', 8 = 'T', 12 = 'A'.
fn rank_char(r: u8) -> char {
    match r {
        0 => '2',
        1 => '3',
        2 => '4',
        3 => '5',
        4 => '6',
        5 => '7',
        6 => '8',
        7 => '9',
        8 => 'T',
        9 => 'J',
        10 => 'Q',
        11 => 'K',
        12 => 'A',
        _ => '?',
    }
}

/// Map rank char (2-9, T, J, Q, K, A; case-insensitive) to 0..13.
fn rank_value(c: char) -> Option<u8> {
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
// Helpers
// ---------------------------------------------------------------------------

/// Turn a concatenated board string like `"AhKd2c"` into the comma-separated
/// form TexasSolver wants, `"Ah,Kd,2c"`.
fn format_board_csv(board: &str) -> Result<String> {
    if !board.is_ascii() || board.len() % 2 != 0 {
        bail!("board {:?} must be ASCII, even length", board);
    }
    let cards: Vec<String> = board
        .as_bytes()
        .chunks(2)
        .map(|ch| String::from_utf8_lossy(ch).to_string())
        .collect();
    Ok(cards.join(","))
}

type JsonObj = serde_json::Map<String, Value>;

fn require_str<'a>(obj: &'a JsonObj, field: &str) -> Result<&'a str> {
    obj.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing or non-string field {:?}", field))
}

fn require_u32(obj: &JsonObj, field: &str) -> Result<u32> {
    let n = obj
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("missing or non-integer field {:?}", field))?;
    u32::try_from(n).map_err(|_| anyhow!("field {:?} value {n} exceeds u32", field))
}

fn require_str_field<'a>(obj: &'a JsonObj, field: &str, parent: &str) -> Result<&'a str> {
    obj.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("{parent}: missing or non-string field {:?}", field))
}

fn require_u32_field(obj: &JsonObj, field: &str, parent: &str) -> Result<u32> {
    let n = obj
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("{parent}: missing or non-integer field {:?}", field))?;
    u32::try_from(n).map_err(|_| anyhow!("{parent}: field {:?} value {n} exceeds u32", field))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn example_flop_fixture() -> Value {
        json!({
            "id": "spot_001",
            "name": "Dry AK-high flop",
            "description": "Canonical single-raised pot c-bet study spot.",
            "street": "flop",
            "input": {
                "board": "AhKd2c",
                "hero_range": "AA, KK, AKs, AKo, QQ",
                "villain_range": "77, 88, 99, TT, AQs",
                "pot": 60,
                "effective_stack": 970,
                "to_act": "hero",
                "bet_tree": "default_v0_1"
            },
            "iterations": 1000,
            "tolerances": { "action_freq_abs": 0.05, "ev_bb_abs": 0.10 },
            "expected_reference": "texassolver",
            "expected_notes": "placeholder"
        })
    }

    #[test]
    fn flop_fixture_emits_all_three_streets() {
        let fix = example_flop_fixture();
        let cfg = translate_to_texassolver(&fix, "out.json").unwrap();
        assert!(cfg.contains("set_pot 60"));
        assert!(cfg.contains("set_effective_stack 970"));
        assert!(cfg.contains("set_board Ah,Kd,2c"));
        // Ranges are emitted with all whitespace stripped (TexasSolver
        // line parser rejects >2-token lines; see module docs).
        assert!(cfg.contains("set_range_oop AA,KK,AKs,AKo,QQ"));
        assert!(cfg.contains("set_range_ip 77,88,99,TT,AQs"));
        assert!(cfg.contains("set_bet_sizes oop,flop,bet,33,66,100"));
        assert!(cfg.contains("set_bet_sizes ip,turn,bet,50,100,200"));
        assert!(cfg.contains("set_bet_sizes oop,river,bet,33,66,100,200"));
        assert!(cfg.contains("set_bet_sizes oop,flop,allin"));
        assert!(cfg.contains("set_allin_threshold 0.67"));
        assert!(cfg.contains("build_tree"));
        assert!(cfg.contains("set_max_iteration 1000"));
        assert!(cfg.contains("start_solve"));
        assert!(cfg.contains("dump_result out.json"));
    }

    #[test]
    fn river_fixture_omits_flop_and_turn_bet_sizes() {
        let mut fix = example_flop_fixture();
        fix["street"] = json!("river");
        fix["input"]["board"] = json!("AhKd2cQc4d");
        let cfg = translate_to_texassolver(&fix, "/tmp/r.json").unwrap();
        assert!(cfg.contains("set_bet_sizes oop,river,bet,33,66,100,200"));
        assert!(!cfg.contains("set_bet_sizes oop,flop"));
        assert!(!cfg.contains("set_bet_sizes oop,turn"));
        assert!(cfg.contains("set_board Ah,Kd,2c,Qc,4d"));
    }

    #[test]
    fn turn_fixture_emits_turn_and_river_only() {
        let mut fix = example_flop_fixture();
        fix["street"] = json!("turn");
        fix["input"]["board"] = json!("AhKd2cQc");
        let cfg = translate_to_texassolver(&fix, "out.json").unwrap();
        assert!(cfg.contains("set_bet_sizes oop,turn,bet,50,100,200"));
        assert!(cfg.contains("set_bet_sizes oop,river,bet,33,66,100,200"));
        assert!(!cfg.contains("set_bet_sizes oop,flop"));
        assert!(cfg.contains("set_board Ah,Kd,2c,Qc"));
    }

    #[test]
    fn unknown_bet_tree_rejects() {
        let mut fix = example_flop_fixture();
        fix["input"]["bet_tree"] = json!("experimental_v0_9");
        let err = translate_to_texassolver(&fix, "out.json").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("bet_tree") && msg.contains("experimental_v0_9"),
            "got: {msg}"
        );
    }

    #[test]
    fn bad_street_rejects() {
        let mut fix = example_flop_fixture();
        fix["street"] = json!("turn");
        // Street says turn, but board length says flop. Should fail the
        // length check.
        fix["input"]["board"] = json!("AhKd2c");
        let err = translate_to_texassolver(&fix, "out.json").unwrap_err();
        assert!(err.to_string().contains("board"), "got: {err}");
    }

    #[test]
    fn missing_required_field_rejects() {
        let mut fix = example_flop_fixture();
        // Remove the pot field from input.
        fix["input"].as_object_mut().unwrap().remove("pot");
        let err = translate_to_texassolver(&fix, "out.json").unwrap_err();
        assert!(err.to_string().contains("pot"), "got: {err}");
    }

    #[test]
    fn non_object_fixture_rejects() {
        let bad = json!([1, 2, 3]);
        let err = translate_to_texassolver(&bad, "out.json").unwrap_err();
        assert!(err.to_string().contains("JSON object"), "got: {err}");
    }

    #[test]
    fn target_format_parse() {
        assert_eq!(
            TargetFormat::parse("texassolver").unwrap(),
            TargetFormat::TexasSolver
        );
        assert_eq!(
            TargetFormat::parse("TexasSolver").unwrap(),
            TargetFormat::TexasSolver
        );
        assert_eq!(
            TargetFormat::parse("ts").unwrap(),
            TargetFormat::TexasSolver
        );
        assert!(TargetFormat::parse("piosolver").is_err());
        assert!(TargetFormat::parse("").is_err());
    }

    #[test]
    fn format_board_csv_flop() {
        assert_eq!(format_board_csv("AhKd2c").unwrap(), "Ah,Kd,2c");
        assert_eq!(format_board_csv("AhKd2cQc").unwrap(), "Ah,Kd,2c,Qc");
        assert_eq!(format_board_csv("AhKd2cQc4d").unwrap(), "Ah,Kd,2c,Qc,4d");
    }

    #[test]
    fn format_board_csv_rejects_odd_length() {
        assert!(format_board_csv("AhK").is_err());
    }

    #[test]
    fn emitted_config_ends_with_newline() {
        let fix = example_flop_fixture();
        let cfg = translate_to_texassolver(&fix, "out.json").unwrap();
        assert!(cfg.ends_with('\n'), "config should end with a newline");
    }

    #[test]
    fn no_comment_lines_emitted() {
        // Regression: TexasSolver's line parser errors with
        // `command not valid: # ...` on any `#`-prefixed line. Don't
        // ever add comment lines back.
        let fix = example_flop_fixture();
        let cfg = translate_to_texassolver(&fix, "out.json").unwrap();
        for line in cfg.lines() {
            assert!(
                !line.trim_start().starts_with('#'),
                "emitted config must not contain comment lines, got: {line:?}"
            );
        }
    }

    #[test]
    fn strip_whitespace_helper() {
        assert_eq!(strip_whitespace("AA, KK, AKs"), "AA,KK,AKs");
        assert_eq!(strip_whitespace(" 77-TT\t, AQs\n"), "77-TT,AQs");
        assert_eq!(strip_whitespace("AA,KK"), "AA,KK");
    }

    #[test]
    fn normalize_expands_pair_range_inclusive() {
        // "77-TT" → 77,88,99,TT
        assert_eq!(
            normalize_range_for_texassolver("77-TT").unwrap(),
            "77,88,99,TT"
        );
        // Reversed form `TT-77` also accepted; lo/hi autodetected.
        assert_eq!(
            normalize_range_for_texassolver("TT-77").unwrap(),
            "77,88,99,TT"
        );
    }

    #[test]
    fn normalize_expands_pair_plus() {
        // "22+" → 22..AA
        let out = normalize_range_for_texassolver("22+").unwrap();
        assert!(out.starts_with("22,33,"), "got: {out}");
        assert!(out.ends_with(",KK,AA"), "got: {out}");
    }

    #[test]
    fn normalize_expands_pair_minus() {
        // "JJ-" → 22..JJ
        let out = normalize_range_for_texassolver("JJ-").unwrap();
        assert!(out.starts_with("22,"), "got: {out}");
        assert!(out.ends_with(",TT,JJ"), "got: {out}");
    }

    #[test]
    fn normalize_expands_suited_plus() {
        // "T9s+" → T9s,J9s,Q9s,K9s,A9s
        assert_eq!(
            normalize_range_for_texassolver("T9s+").unwrap(),
            "T9s,J9s,Q9s,K9s,A9s"
        );
    }

    #[test]
    fn normalize_expands_offsuit_plus() {
        // "KTo+" → KTo,ATo (L > kicker, skip pair slot)
        let out = normalize_range_for_texassolver("KTo+").unwrap();
        // Skips "TT" pair slot.
        assert!(out.contains("KTo"), "got: {out}");
        assert!(out.contains("ATo"), "got: {out}");
        assert!(!out.contains("TTo"), "got: {out}");
    }

    #[test]
    fn normalize_passes_through_length_2_and_3() {
        // Plain length-2/3 tokens are passed through verbatim.
        assert_eq!(
            normalize_range_for_texassolver("AA, KK, AKs, AKo").unwrap(),
            "AA,KK,AKs,AKo"
        );
    }

    #[test]
    fn normalize_preserves_weight_suffix() {
        // Weight attaches to every sub-token of an expansion.
        let out = normalize_range_for_texassolver("77-99:0.5").unwrap();
        assert_eq!(out, "77:0.5,88:0.5,99:0.5");
    }

    #[test]
    fn normalize_rejects_bad_token() {
        // "FOOBAR" is length 6 — none of the known forms.
        let err = normalize_range_for_texassolver("FOOBAR").unwrap_err();
        assert!(err.to_string().contains("unrecognized"), "got: {err}");
    }

    #[test]
    fn normalize_tolerates_trailing_comma_and_whitespace() {
        assert_eq!(
            normalize_range_for_texassolver(" AA, KK, ").unwrap(),
            "AA,KK"
        );
    }
}
