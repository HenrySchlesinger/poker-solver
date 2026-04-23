#!/usr/bin/env bash
# Install the poker-solver pre-commit hook into .git/hooks/ via symlink.
# Symlinking (not copying) means any future edit to .githooks/pre-commit
# takes effect on the next commit — no re-install needed.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

chmod +x .githooks/pre-commit

# -s symlink, -f force-replace any existing hook (.sample files or a stale copy).
# Relative target so the link keeps working if the repo moves.
ln -sf ../../.githooks/pre-commit .git/hooks/pre-commit

echo "pre-commit hook installed -> .git/hooks/pre-commit -> .githooks/pre-commit"
echo "Bypass on a one-off basis with: git commit --no-verify"
