#!/usr/bin/env bash
# Bootstrap a fresh Colab runtime: install Rust, clone the repo, build solver-cli.
#
# Called from the first cell of every Colab notebook in this directory. Safe
# to re-run — idempotent for both the rustup install and the git clone.
set -euo pipefail

# Install Rust if not already present. Cargo ships to ~/.cargo/bin, which
# isn't on Colab's default PATH, so callers must `source ~/.cargo/env`
# (handled below) or set it explicitly in their Python cell.
if ! command -v cargo >/dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
# shellcheck disable=SC1091
source "$HOME/.cargo/env"

# Clone or update the repo under /content (Colab's working dir).
cd /content
if [[ ! -d poker-solver ]]; then
    git clone https://github.com/HenrySchlesinger/poker-solver.git
fi
cd poker-solver
git pull --ff-only

# Build the dev harness. --release matters — debug builds are ~30x slower on
# the hot CFR loop and the whole point of Colab is overnight precompute.
cargo build --release -p solver-cli

echo "setup.sh: ready. solver-cli is at $(pwd)/target/release/solver-cli"
