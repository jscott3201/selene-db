#!/usr/bin/env bash
# External consumer of extracted .crate contents, never workspace source paths.
# Exact registry version requirements remain in the normalized crate manifests.
# Local patches select ONLY these unpublished candidate archives for rehearsal.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
archives="${1:-$root/target/package}"
bash "$root/.github/scripts/check-package-release.sh" --archives "$archives" "$root"
metadata="$(cargo metadata --no-deps --locked --offline --format-version 1)"
version="$(jq -r '.packages[] | select(.name == "selene-db") | .version' <<< "$metadata")"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/consumer/tests" "$scratch/unpacked"
manifest="$scratch/consumer/Cargo.toml"
cat > "$manifest" <<'TOML'
[package]
name = "selene-artifact-consumer"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
TOML
while IFS= read -r name; do
  tar -xzf "$archives/$name-$version.crate" -C "$scratch/unpacked"
  package="$scratch/unpacked/$name-$version"
  # Cargo's normalized artifact must contain registry constraints, not paths.
  cargo metadata --manifest-path "$package/Cargo.toml" --no-deps --locked --offline --format-version 1 |
    jq -e 'all(.packages[].dependencies[]; .path == null and .req != "*")' >/dev/null
  printf '%s = "=%s"\n' "$name" "$version" >> "$manifest"
done < "$root/.github/scripts/public-crates.txt"
printf '\n[dev-dependencies]\ntempfile = "=3.27.0"\n\n[features]\nall-engine-features = [' >> "$manifest"
jq -r '.packages[] | select(.publish != []) | .name as $name | .features | keys[] | "\"\($name)/\(.)\","' \
  <<< "$metadata" >> "$manifest"
printf ']\n\n[patch.crates-io]\n' >> "$manifest"
while IFS= read -r name; do
  printf '%s = { path = "%s/unpacked/%s-%s" }\n' "$name" "$scratch" "$name" "$version" >> "$manifest"
done < "$root/.github/scripts/public-crates.txt"
cp "$root/docs/v2/roadmap/examples/facade_smoke.rs" "$scratch/consumer/tests/"
cp "$root/docs/v2/roadmap/examples/facade_release.rs" "$scratch/consumer/tests/"

# The consumer gets its own lock and target; no inherited workspace feature
# unification, package target cache, or path shortcut can make this pass.
unset CARGO_TARGET_DIR
cargo generate-lockfile --manifest-path "$manifest"
cargo metadata --manifest-path "$manifest" --locked --format-version 1 |
  jq -e --arg prefix "$scratch/unpacked/" --arg version "$version" '
    [.packages[] | select(.name | startswith("selene-db"))] as $engine |
    ($engine | length == 8) and
    all($engine[]; .version == $version and (.manifest_path | startswith($prefix)))
  ' >/dev/null
cargo test --manifest-path "$manifest" --locked
cargo test --manifest-path "$manifest" --locked --all-features
echo "OK: external default/all-feature consumer used all eight extracted candidate crates"
