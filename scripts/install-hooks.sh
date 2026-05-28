#!/usr/bin/env bash
# One-time local setup: point git at the tracked .githooks/ directory.
# Run once per clone:  bash scripts/install-hooks.sh
#
# .githooks/ is version-controlled (unlike .git/hooks/), so the team shares
# the same gates. Mirrors the CI split:
#   pre-commit -> cargo fmt --check + file-size + no-secret  (fast)
#   pre-push   -> cargo clippy -D warnings + nextest + doctests (the gate
#                 moved off PR->development CI)
#
# Escape hatches: `git commit/push --no-verify` (once) or
# `export SELENE_SKIP_HOOKS=1` (whole shell session).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

git config core.hooksPath .githooks
chmod +x .githooks/pre-commit .githooks/pre-push 2>/dev/null || true

echo "core.hooksPath -> .githooks"
echo "  pre-commit: cargo fmt --check + file-size cap + no-secret scan"
echo "  pre-push:   cargo clippy -D warnings + nextest + doctests"
echo "Skip once: --no-verify   |   skip session: export SELENE_SKIP_HOOKS=1"
echo
echo "LSP: rust-analyzer is editor-side (not a git hook). Enable clippy-on-save"
echo "in your editor (VS Code: \"rust-analyzer.check.command\": \"clippy\")."
