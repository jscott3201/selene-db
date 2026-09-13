#!/usr/bin/env bash
# Negative regression cases for the release inventory and actual archive bytes.
set -euo pipefail
checker="$(pwd)/.github/scripts/check-package-release.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/.github/scripts" "$scratch/demo/src" "$scratch/packages"
cat > "$scratch/Cargo.toml" <<'TOML'
[workspace]
members = ["demo"]
resolver = "3"
TOML
cat > "$scratch/demo/Cargo.toml" <<'TOML'
[package]
name = "demo"
version = "0.1.0"
edition = "2024"
rust-version = "1.97.1"
description = "Package regression fixture"
repository = "https://example.invalid/demo"
license = "MIT OR Apache-2.0"
TOML
touch "$scratch/demo/src/lib.rs"
printf 'demo\n' > "$scratch/.github/scripts/public-crates.txt"
for text in LICENSE-MIT LICENSE-APACHE NOTICE THIRDPARTY.md; do
  printf 'fixture %s\n' "$text" > "$scratch/$text"
done
expect_failure() {
  if bash "$checker" "$@" > "$scratch/failure.log" 2>&1; then
    echo "FAIL: checker accepted $*" >&2
    exit 1
  fi
}
expect_failure --check "$scratch"
bash "$checker" --sync-licenses "$scratch"
bash "$checker" --check "$scratch"
printf 'drift\n' >> "$scratch/demo/NOTICE"
expect_failure --check "$scratch"
bash "$checker" --sync-licenses "$scratch"
printf 'demo\ndemo\n' > "$scratch/.github/scripts/public-crates.txt"
expect_failure --check "$scratch"
printf 'demo\n' > "$scratch/.github/scripts/public-crates.txt"
cp -R "$scratch/demo" "$scratch/demo-0.1.0"
tar -czf "$scratch/packages/demo-0.1.0.crate" -C "$scratch" demo-0.1.0
bash "$checker" --archives "$scratch/packages" "$scratch"
for text in LICENSE-MIT LICENSE-APACHE NOTICE THIRDPARTY.md; do
  rm "$scratch/demo-0.1.0/$text"
  tar -czf "$scratch/packages/demo-0.1.0.crate" -C "$scratch" demo-0.1.0
  expect_failure --archives "$scratch/packages" "$scratch"
  cp "$scratch/$text" "$scratch/demo-0.1.0/$text"
done
echo "OK: package checks reject missing/stale source texts, duplicate inventory and each missing archive text"
