//! `demo` subcommand — the "30-second demo" renderer.
//!
//! Given a spot id (`royal` / `coinflip` / `bluff_catch` / `all`),
//! writes a polished terminal visualization of the GTO strategy to
//! `out` and returns `Ok(())`.
//!
//! # Design notes
//!
//! This is intentionally divorced from the live solver. We bake
//! analytically known strategies into `demo_spots.rs` so:
//!
//! 1. The demo runs instantly (<100ms) — no wait, no thinking face.
//! 2. It works today, before `NlheSubgame` is fully wired. The task
//!    brief says: "If the NLHE subgame isn't fully wired, your demo
//!    should fall back to a hardcoded canonical strategy." That's
//!    what this is.
//! 3. Output is perfectly deterministic. Same spot, same bytes, every
//!    time — no PRNG noise, no floating-point drift, no wall-clock
//!    leak into the "compute time" display (it's a baked constant).
//!
//! When `NlheSubgame` and `BetTree::default_v0_1` land and stabilize,
//! the Day 2/3 agents can swap `demo_spots::find_spot()` for a live
//! solve without changing anything here in `demo.rs`.
//!
//! # Color discipline
//!
//! Colors are applied through `colored`, which natively respects
//! `NO_COLOR` and the TTY-status of stdout. Because we write to an
//! arbitrary `impl Write` (which includes `Vec<u8>` in the test
//! harness), we also gate color emission on a caller-supplied
//! `use_color` flag so integration tests get clean plain-ASCII output.
//!
//! # Width
//!
//! The header banner is 67 columns wide (the dash line). Everything
//! else fits inside that. Terminals default to 80 columns; we leave
//! a margin so the output isn't squeezed on narrow windows.

use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use colored::{ColoredString, Colorize};

use crate::demo_spots::{self, Decision, Spot};

/// Width of the header banner rules ("═══..."). Chosen so the typical
/// spot renders cleanly in an 80-column terminal and still feels
/// substantial on wide Retina windows.
const BANNER_WIDTH: usize = 67;

/// Width of the per-action bar (characters). Each action's frequency
/// is scaled to this and rendered as a run of `█` followed by padding.
const BAR_WIDTH: usize = 10;

/// Arguments for the `demo` subcommand.
#[derive(Debug, Clone)]
pub struct DemoArgs {
    /// Which spot to render. One of the `VALID_SPOT_IDS` values.
    pub spot: String,
    /// When `false`, suppress ANSI color escapes regardless of what
    /// `colored` thinks about the terminal. Tests pass `false` here;
    /// normal CLI use passes `true` and lets `colored` decide based on
    /// TTY detection + `NO_COLOR`.
    pub use_color: bool,
}

/// Entry point for `solver-cli demo`. Writes to `out` and returns
/// `Ok(())` on a successful render.
///
/// The error arm fires for a single case: the user typed an unknown
/// spot id. In that case we surface the full list of valid ids so the
/// next attempt has a better shot.
pub fn run_demo(args: &DemoArgs, mut out: impl Write) -> Result<()> {
    // `colored` is a process-global flag. Flip it based on the caller's
    // preference; we restore the default (auto-detect) on the way out
    // so that subsequent calls inside the same process don't leak
    // state. (Matters for tests that exercise run_demo twice back to
    // back with different `use_color` values.)
    set_color_mode(args.use_color);

    let result = if args.spot == "all" {
        render_all(&mut out)
    } else {
        render_single(&args.spot, &mut out)
    };

    // Always restore auto-detect before returning, even on error.
    colored::control::unset_override();

    result
}

fn set_color_mode(use_color: bool) {
    if use_color {
        // Let `colored` decide based on TTY + `NO_COLOR`.
        colored::control::unset_override();
    } else {
        colored::control::set_override(false);
    }
}

/// Render every spot in sequence, one per `all_spots()` entry.
fn render_all(out: &mut impl Write) -> Result<()> {
    let spots = demo_spots::all_spots();
    for (i, spot) in spots.iter().enumerate() {
        if i > 0 {
            // Blank separator between spots so the banners don't visually
            // run together.
            writeln!(out)?;
        }
        render_spot(spot, out)?;
    }
    Ok(())
}

/// Render a single spot by id. Returns a helpful error if the id is
/// unknown.
fn render_single(id: &str, out: &mut impl Write) -> Result<()> {
    let spot = demo_spots::find_spot(id).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown spot: {:?} (valid: {})",
            id,
            demo_spots::VALID_SPOT_IDS.join(", "),
        )
    })?;
    render_spot(&spot, out)
}

/// The core render pipeline: banner → inputs block → strategy bars →
/// stats → narration → footer.
fn render_spot(spot: &Spot, out: &mut impl Write) -> Result<()> {
    render_banner(spot, out)?;
    render_inputs(spot, out)?;
    for decision in &spot.decisions {
        render_decision(decision, out)?;
    }
    render_stats(spot, out)?;
    render_narration(spot, out)?;
    render_footer(out)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Banner (top + bottom rules, title line)
// ---------------------------------------------------------------------------

fn render_banner(spot: &Spot, out: &mut impl Write) -> Result<()> {
    let rule = "═".repeat(BANNER_WIDTH);
    writeln!(out, "{}", rule.bright_cyan())?;
    writeln!(
        out,
        "  {} {} {}",
        "POKER-SOLVER DEMO".bold().bright_white(),
        "—".bright_black(),
        spot.title.bright_white(),
    )?;
    writeln!(out, "{}", rule.bright_cyan())?;
    writeln!(out)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Inputs block: hero range, villain range, board, pot/stacks
// ---------------------------------------------------------------------------

fn render_inputs(spot: &Spot, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "  {}    {}",
        "Hero range:".bright_black(),
        spot.hero_range.bright_white(),
    )?;
    writeln!(
        out,
        "  {} {}",
        "Villain range:".bright_black(),
        spot.villain_range.bright_white(),
    )?;

    let board_display = if spot.board.is_empty() {
        "(preflop — no community cards yet)".to_string()
    } else {
        spot.board.to_string()
    };
    let board_line = if spot.board_annotation.is_empty() {
        board_display.bright_white().to_string()
    } else {
        format!(
            "{}  {}{}{}",
            board_display.bright_white(),
            "(".bright_black(),
            spot.board_annotation.bright_black().italic(),
            ")".bright_black(),
        )
    };
    writeln!(out, "  {}         {}", "Board:".bright_black(), board_line,)?;

    writeln!(
        out,
        "  {}           {} chips",
        "Pot:".bright_black(),
        format!("{}", spot.pot).bright_white(),
    )?;
    writeln!(
        out,
        "  {}        {} chips each",
        "Stacks:".bright_black(),
        format!("{}", spot.stack).bright_white(),
    )?;
    writeln!(out)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Strategy bars: one Decision's worth of action frequencies
// ---------------------------------------------------------------------------

/// Width of a decision node's data column (label + bar). Used so
/// multi-action decisions line up vertically.
const ACTION_COL_WIDTH: usize = 18;

fn render_decision(decision: &Decision, out: &mut impl Write) -> Result<()> {
    writeln!(out, "{}", section_rule("GTO STRATEGY").bright_cyan(),)?;

    // Action labels row (e.g. "check      bet 66%    bet (pot)")
    write!(out, "{}", " ".repeat(24))?;
    for action in &decision.actions {
        write!(
            out,
            "{:<width$}",
            action.label.bright_white(),
            width = ACTION_COL_WIDTH,
        )?;
    }
    writeln!(out)?;

    // Bars row, prefixed by the decision label.
    write!(
        out,
        "  {:<20} ",
        format!("{}:", decision.label).bright_black(),
    )?;
    for action in &decision.actions {
        let bar = bar_for(action.frequency);
        write!(out, "{:<width$}", bar, width = ACTION_COL_WIDTH)?;
    }
    writeln!(out)?;

    // Percentages row, aligned under the bars.
    write!(out, "{}", " ".repeat(24))?;
    for action in &decision.actions {
        let pct = format!("{:>3.0}%", action.frequency * 100.0);
        let colored_pct = freq_color(action.frequency, &pct);
        write!(out, "{:<width$}", colored_pct, width = ACTION_COL_WIDTH)?;
    }
    writeln!(out)?;
    writeln!(out)?;
    Ok(())
}

/// Build the `[████      ]` bar for a single action frequency.
///
/// Filled cells are drawn with `█` (U+2588); empty cells are drawn
/// with `░` (U+2591), which gives a ghost of the "full" bar and
/// anchors the eye even at 0% frequency.
fn bar_for(frequency: f32) -> String {
    let filled = (frequency * BAR_WIDTH as f32).round() as usize;
    let filled = filled.min(BAR_WIDTH);
    let empty = BAR_WIDTH - filled;

    let full_part: String = "█".repeat(filled);
    let empty_part: String = "░".repeat(empty);

    let colored_full = freq_color(frequency, &full_part);
    let colored_empty = empty_part.bright_black();

    format!("[{colored_full}{colored_empty}]")
}

/// Pick a color for a frequency. Green = dominant action (>=50%),
/// yellow = mixed (10%–50%), red-ish gray = rare (<10%). Keeps the
/// eye on the important bar.
fn freq_color(frequency: f32, text: &str) -> ColoredString {
    if frequency >= 0.5 {
        text.bright_green()
    } else if frequency >= 0.1 {
        text.bright_yellow()
    } else {
        text.bright_black()
    }
}

// ---------------------------------------------------------------------------
// Stats block: exploitability, iterations, compute time, equity
// ---------------------------------------------------------------------------

fn render_stats(spot: &Spot, out: &mut impl Write) -> Result<()> {
    // This block lives inside the "GTO STRATEGY" section visually, but
    // we don't print another section header — we're continuing the
    // conversation from the bars above.
    if let Some(eq) = spot.hero_equity {
        writeln!(
            out,
            "  {}       {}  {}",
            "Hero equity:".bright_black(),
            format!("{:.1}%", eq * 100.0).bright_white(),
            "(vs villain range)".bright_black().italic(),
        )?;
    }
    writeln!(
        out,
        "  {}  {} bb  {}",
        "Exploitability:".bright_black(),
        format!("{:.3}", spot.exploitability_bb).bright_white(),
        "(Nash ≈ 0)".bright_black().italic(),
    )?;
    writeln!(
        out,
        "  {}      {} CFR+",
        "Iterations:".bright_black(),
        format!("{}", spot.iterations).bright_white(),
    )?;
    writeln!(
        out,
        "  {}    {}",
        "Compute time:".bright_black(),
        format_duration(spot.compute_time).bright_white(),
    )?;
    writeln!(out)?;
    Ok(())
}

/// "42 ms" / "1.2 s" format. Keeps the units intuitive to a poker pro.
fn format_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{} ms", ms)
    } else {
        format!("{:.1} s", (ms as f64) / 1000.0)
    }
}

// ---------------------------------------------------------------------------
// Narration block: hand-authored "what just happened"
// ---------------------------------------------------------------------------

fn render_narration(spot: &Spot, out: &mut impl Write) -> Result<()> {
    writeln!(out, "{}", section_rule("WHAT THIS MEANS").bright_cyan(),)?;
    // Soft-wrap to a width that fits inside the banner with a 2-space
    // indent margin. Preserves the hand-authored sentence breaks.
    let wrap_width = BANNER_WIDTH.saturating_sub(4);
    for line in soft_wrap(spot.narration, wrap_width) {
        writeln!(out, "  {}", line)?;
    }
    writeln!(out)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Footer
// ---------------------------------------------------------------------------

fn render_footer(out: &mut impl Write) -> Result<()> {
    let rule = "═".repeat(BANNER_WIDTH);
    writeln!(out, "{}", rule.bright_cyan())?;
    writeln!(
        out,
        "  {}  {}",
        "Learn more:".bright_black(),
        "https://github.com/HenrySchlesinger/poker-solver".bright_white(),
    )?;
    writeln!(
        out,
        "  {}  {}",
        "Want this live on YOUR stream?".bright_black(),
        "https://pokerpanel.tv".bright_white(),
    )?;
    writeln!(out, "{}", rule.bright_cyan())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// "── <label> ─────────..." section rule, sized to BANNER_WIDTH.
fn section_rule(label: &str) -> String {
    let prefix = "── ";
    let tail_len = BANNER_WIDTH.saturating_sub(prefix.chars().count() + label.chars().count() + 1);
    format!("{prefix}{label} {}", "─".repeat(tail_len))
}

/// Greedy word-wrap. Preserves single spaces; never splits words. Good
/// enough for 2–4 sentence narrations with no inline code.
fn soft_wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if line.is_empty() {
            line.push_str(word);
            continue;
        }
        if line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
            line.push_str(word);
        } else {
            line.push(' ');
            line.push_str(word);
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the demo in no-color mode and return the produced string.
    fn render_no_color(spot_id: &str) -> String {
        let args = DemoArgs {
            spot: spot_id.to_string(),
            use_color: false,
        };
        let mut buf = Vec::new();
        run_demo(&args, &mut buf).expect("render succeeded");
        String::from_utf8(buf).expect("utf-8 output")
    }

    #[test]
    fn royal_renders_without_error() {
        let s = render_no_color("royal");
        assert!(s.contains("POKER-SOLVER DEMO"));
        assert!(s.contains("AhKhQhJhTh"));
        assert!(s.contains("royal flush"));
        assert!(s.contains("GTO STRATEGY"));
        assert!(s.contains("WHAT THIS MEANS"));
    }

    #[test]
    fn coinflip_renders_without_error() {
        let s = render_no_color("coinflip");
        assert!(s.contains("coinflip") || s.contains("preflop"));
        assert!(s.contains("AsKs"));
        assert!(s.contains("2c2d"));
    }

    #[test]
    fn bluff_catch_renders_without_error() {
        let s = render_no_color("bluff_catch");
        assert!(s.contains("bluff"));
        assert!(s.contains("KdQd"));
        // Has a mixed strategy — at least one non-0/100 frequency.
        assert!(s.contains("67%") || s.contains("66%") || s.contains("30%"));
    }

    #[test]
    fn all_renders_every_spot() {
        let s = render_no_color("all");
        assert!(s.contains("AhKhQhJhTh")); // royal
        assert!(s.contains("AsKs")); // coinflip (AKs)
        assert!(s.contains("KdQd")); // bluff_catch
    }

    #[test]
    fn unknown_spot_errors_with_valid_list() {
        let args = DemoArgs {
            spot: "mystery".to_string(),
            use_color: false,
        };
        let mut buf = Vec::new();
        let err = run_demo(&args, &mut buf).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("mystery"), "got: {msg}");
        assert!(msg.contains("valid"), "got: {msg}");
        // Should list every known id.
        for id in demo_spots::VALID_SPOT_IDS {
            assert!(msg.contains(id), "error message missing id {:?}: {msg}", id);
        }
    }

    #[test]
    fn no_color_output_contains_no_ansi_escapes() {
        for id in &["royal", "coinflip", "bluff_catch", "all"] {
            let s = render_no_color(id);
            assert!(
                !s.contains('\x1b'),
                "spot {:?}: unexpected ANSI escape in no-color output: {:?}",
                id,
                // Snip to a short window around the escape for the error.
                s.chars().take(200).collect::<String>(),
            );
        }
    }

    #[test]
    fn render_is_deterministic() {
        // Same spot, same bytes. We bake compute_time etc., so no
        // wall-clock leaks in.
        let a = render_no_color("royal");
        let b = render_no_color("royal");
        assert_eq!(a, b);
    }

    #[test]
    fn bar_for_zero_is_all_empty() {
        let s = bar_for(0.0);
        // Strip ANSI (shouldn't be any in test, but be defensive).
        let plain = strip_ansi(&s);
        assert!(plain.starts_with('['));
        assert!(plain.ends_with(']'));
        // Body length should equal BAR_WIDTH.
        let body_chars: Vec<char> = plain.chars().skip(1).collect();
        let body_chars: Vec<char> = body_chars
            .iter()
            .take(body_chars.len() - 1)
            .copied()
            .collect();
        assert_eq!(body_chars.len(), BAR_WIDTH);
        // All empty boxes.
        assert!(body_chars.iter().all(|c| *c == '░'), "got: {plain:?}");
    }

    #[test]
    fn bar_for_one_is_all_filled() {
        let s = bar_for(1.0);
        let plain = strip_ansi(&s);
        let body: String = plain.chars().skip(1).take(BAR_WIDTH).collect();
        assert!(body.chars().all(|c| c == '█'), "got: {plain:?}");
    }

    #[test]
    fn bar_for_half_is_half_filled() {
        let s = bar_for(0.5);
        let plain = strip_ansi(&s);
        let body: String = plain.chars().skip(1).take(BAR_WIDTH).collect();
        let filled_count = body.chars().filter(|c| *c == '█').count();
        // 0.5 * 10 = 5, exactly.
        assert_eq!(filled_count, 5, "got: {plain:?}");
    }

    #[test]
    fn section_rule_is_banner_width() {
        let rule = section_rule("EXAMPLE");
        assert_eq!(rule.chars().count(), BANNER_WIDTH);
    }

    #[test]
    fn soft_wrap_handles_basic_paragraph() {
        let lines = soft_wrap("the quick brown fox jumps over the lazy dog", 20);
        for line in &lines {
            assert!(line.chars().count() <= 20, "line too long: {line:?}");
        }
        // No word was truncated.
        let joined = lines.join(" ");
        assert_eq!(joined, "the quick brown fox jumps over the lazy dog");
    }

    #[test]
    fn format_duration_picks_readable_unit() {
        assert_eq!(format_duration(Duration::from_millis(42)), "42 ms");
        assert_eq!(format_duration(Duration::from_millis(999)), "999 ms");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.5 s");
    }

    /// Very basic ANSI-strip for test assertions. Handles CSI
    /// sequences; ignores the fancier modes we don't use.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    // Swallow until a letter (the sequence terminator).
                    for c2 in chars.by_ref() {
                        if c2.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}
