//! solver-cli — dev harness.
//!
//! Subcommands:
//!   solve       — solve a spot and print JSON
//!   bench       — run a bench battery and print numbers
//!   validate    — diff our solver against TexasSolver on canonical spots
//!   precompute  — solve a grid of spots and write cache files
//!
//! This binary is NEVER shipped to Poker Panel users — strictly a
//! development tool. Runs on the Mac for interactive work, runs on
//! Colab for overnight precompute jobs.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "solver-cli", version, about = "Poker Solver dev harness")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Solve a single spot and print JSON to stdout.
    Solve {
        /// Board string, e.g., "AhKh2s".
        #[arg(long)]
        board: String,
        /// Hero range, e.g., "AA,KK,AKs".
        #[arg(long)]
        hero_range: String,
        /// Villain range.
        #[arg(long)]
        villain_range: String,
        /// Pot size in chips.
        #[arg(long, default_value = "100")]
        pot: u32,
        /// Effective stack in chips.
        #[arg(long, default_value = "1000")]
        stack: u32,
        /// Iteration count.
        #[arg(long, default_value = "1000")]
        iterations: u32,
    },

    /// Validate our solver against TexasSolver on a fixture.
    Validate {
        /// Path to fixture JSON (see tests/fixtures/).
        #[arg(long)]
        spot: String,
    },

    /// Precompute a batch of spots (used by Colab).
    Precompute {
        /// Input: grid specification JSON.
        #[arg(long)]
        grid: String,
        /// Output directory.
        #[arg(long)]
        output: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Solve { .. } => {
            // TODO (Day 2, agent A5): dispatch to solver.
            anyhow::bail!("not yet implemented");
        }
        Cmd::Validate { .. } => {
            // TODO (Day 6, agent A1): diff vs TexasSolver JSON output.
            anyhow::bail!("not yet implemented");
        }
        Cmd::Precompute { .. } => {
            // TODO (Day 5, agent A5): grid-solve for Colab.
            anyhow::bail!("not yet implemented");
        }
    }
}
