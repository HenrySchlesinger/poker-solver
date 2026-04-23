//! solver-cli — dev harness.
//!
//! Subcommands:
//!   solve              — solve a spot and print JSON
//!   validate           — diff our solver against TexasSolver on canonical spots
//!   precompute         — solve a grid of spots and write cache files
//!   translate-fixture  — convert a fixture JSON into a TexasSolver config
//!   demo               — render a polished 30-second demo (A30)
//!
//! This binary is NEVER shipped to Poker Panel users — strictly a
//! development tool. Runs on the Mac for interactive work, runs on
//! Colab for overnight precompute jobs. `demo` is the one "showable"
//! surface — what Henry pastes into a chat when a streamer asks
//! "cool, but what does this actually do?"
//!
//! See `src/solve_cmd.rs` for the `solve` implementation,
//! `src/translate.rs` for the fixture-translator, and `src/demo.rs`
//! for the 30-second demo renderer. `validate` and `precompute` are
//! scaffolded for later days of the sprint.

use clap::{Parser, Subcommand};

mod demo;
mod demo_spots;
mod solve_cmd;
mod translate;

use demo::{run_demo, DemoArgs};
use solve_cmd::{run_solve, SolveArgs};
use translate::{run_translate, TargetFormat, TranslateArgs};

/// Top-level CLI.
#[derive(Parser)]
#[command(name = "solver-cli", version, about = "Poker Solver dev harness")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Available subcommands.
#[derive(Subcommand)]
enum Cmd {
    /// Solve a single spot and print JSON to stdout.
    Solve {
        /// Board string, e.g. "AhKh2s". Empty string = preflop.
        #[arg(long)]
        board: String,
        /// Hero range, e.g. "AA,KK,AKs". See `solver-nlhe::Range::parse`.
        #[arg(long)]
        hero_range: String,
        /// Villain range, same syntax as `--hero-range`.
        #[arg(long)]
        villain_range: String,
        /// Pot size in chips.
        #[arg(long, default_value = "100")]
        pot: u32,
        /// Effective stack in chips.
        #[arg(long, default_value = "1000")]
        stack: u32,
        /// CFR iteration count.
        #[arg(long, default_value = "1000")]
        iterations: u32,
        /// Bet-tree profile. Only "default" is recognized in v0.1-wip.
        #[arg(long, default_value = "default")]
        bet_tree: String,
    },

    /// Validate our solver against TexasSolver on a JSON fixture.
    Validate {
        /// Path to the fixture JSON. See `tests/fixtures/` (Day 6).
        #[arg(long)]
        spot: String,
    },

    /// Precompute a batch of spots (used by Colab on Day 5).
    Precompute {
        /// Input: grid specification JSON.
        #[arg(long)]
        grid: String,
        /// Output directory for cache files.
        #[arg(long)]
        output: String,
    },

    /// Translate a fixture JSON (A15 schema) into a TexasSolver
    /// `.tsconfig` file for the A14 differential harness.
    ///
    /// See `src/translate.rs` for the schema→config mapping.
    TranslateFixture {
        /// Path to the input fixture JSON.
        #[arg(long)]
        input: String,
        /// Path to write the translated config. `-` = stdout.
        #[arg(long)]
        output: String,
        /// Target format. Only `"texassolver"` is recognized today.
        #[arg(long, default_value = "texassolver")]
        format: String,
        /// Path baked into the emitted `dump_result` line. TexasSolver
        /// writes its strategy JSON there at the end of `start_solve`.
        #[arg(long, default_value = "output_result.json")]
        dump_path: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Solve {
            board,
            hero_range,
            villain_range,
            pot,
            stack,
            iterations,
            bet_tree,
        } => {
            let args = SolveArgs {
                board_raw: board,
                hero_range_raw: hero_range,
                villain_range_raw: villain_range,
                pot,
                stack,
                iterations,
                bet_tree,
            };
            let stdout = std::io::stdout();
            run_solve(&args, stdout.lock())
        }
        Cmd::Validate { spot: _ } => {
            // TODO (Day 6, agent A1): diff vs TexasSolver JSON output.
            anyhow::bail!("validate: not-yet-implemented (scheduled Day 6)")
        }
        Cmd::Precompute { grid: _, output: _ } => {
            // TODO (Day 5, agent A5): grid-solve for Colab.
            anyhow::bail!("precompute: not-yet-implemented (scheduled Day 5)")
        }
        Cmd::TranslateFixture {
            input,
            output,
            format,
            dump_path,
        } => {
            let fmt = TargetFormat::parse(&format)?;
            let args = TranslateArgs {
                input,
                output,
                format: fmt,
                dump_path,
            };
            run_translate(&args)
        }
    }
}
