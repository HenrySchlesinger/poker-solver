//! solver-cli — dev harness.
//!
//! Subcommands:
//!   solve              — solve a spot and print JSON
//!   validate           — diff our solver against TexasSolver on canonical spots
//!   precompute         — solve a grid of spots and write cache files
//!   seed-cache         — write the v0.1 format-only flop-cache seed binary
//!   translate-fixture  — convert a fixture JSON into a TexasSolver config
//!   demo               — render a polished 30-second demo (A30)
//!   md-to-ipynb        — convert a colab/*.md plan into a Jupyter .ipynb
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
mod md_to_ipynb;
mod seed_cache;
mod solve_cmd;
mod translate;

use demo::{run_demo, DemoArgs};
use md_to_ipynb::{run_md_to_ipynb, MdToIpynbArgs};
use seed_cache::{run_seed_cache, SeedCacheArgs};
use solve_cmd::{run_solve, SolveArgs, SolverKind};
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
        /// Solver implementation. `vector` (default) uses the post-A70
        /// combo-axis-SIMD walker — ~10× faster than `flat` on NLHE
        /// river spots. `flat` uses the A64 flat-array + SIMD regret
        /// matching path. `classic` uses the `HashMap<InfoSetId, _>`
        /// reference implementation; kept as an escape hatch.
        #[arg(long, default_value = "vector")]
        solver: String,
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

    /// Write the v0.1 format-only flop-cache seed binary.
    ///
    /// Produces a 36-entry placeholder cache (12 boards × 3 SPR buckets ×
    /// Srp) with hand-constructed strategies. This is NOT real GTO — it
    /// exists so the loader + format can be exercised end-to-end ahead
    /// of the Day-5 Colab precompute. See `src/seed_cache.rs` and
    /// `data/flop-cache/README.md`.
    SeedCache {
        /// Output binary path. Canonically
        /// `data/flop-cache/flop-cache-v0.1.bin`.
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

    /// Render a polished 30-second demo of a canonical GTO spot.
    ///
    /// The output is deterministic and uses hand-curated strategies
    /// (see `demo_spots.rs`). When the live NLHE subgame is fully
    /// wired, the underlying spot data can be swapped for real solver
    /// output without touching the renderer.
    Demo {
        /// Spot preset. One of `royal`, `coinflip`, `bluff_catch`, `all`.
        #[arg(long, default_value = "royal")]
        spot: String,
    },

    /// Convert a `colab/*.md` plan into a Jupyter `.ipynb` notebook.
    ///
    /// The `.md` files in `colab/` are the source-of-truth plans; this
    /// subcommand deterministically emits the matching `.ipynb` so we
    /// can ship "Open in Colab" badges that link to real notebooks on
    /// GitHub. See `src/md_to_ipynb.rs` for the nbformat v4.5 details.
    MdToIpynb {
        /// Path to the input `.md` file.
        #[arg(long)]
        input: String,
        /// Path to write the `.ipynb` to. `-` writes to stdout.
        #[arg(long)]
        output: String,
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
            solver,
        } => {
            let solver_kind = SolverKind::parse(&solver)?;
            let args = SolveArgs {
                board_raw: board,
                hero_range_raw: hero_range,
                villain_range_raw: villain_range,
                pot,
                stack,
                iterations,
                bet_tree,
                solver: solver_kind,
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
        Cmd::SeedCache { output } => {
            let args = SeedCacheArgs {
                output: output.into(),
            };
            run_seed_cache(&args)
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
        Cmd::Demo { spot } => {
            // `use_color: true` means "let `colored` decide" — it already
            // auto-detects `NO_COLOR` and TTY status, so this is the right
            // default for interactive CLI use. Tests pass `false` directly.
            let args = DemoArgs {
                spot,
                use_color: true,
            };
            let stdout = std::io::stdout();
            run_demo(&args, stdout.lock())
        }
        Cmd::MdToIpynb { input, output } => {
            let args = MdToIpynbArgs { input, output };
            run_md_to_ipynb(&args)
        }
    }
}
