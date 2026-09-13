#!/usr/bin/env bash
# Keep the release inventory dependency-ordered and legal texts in every crate.
# Root texts are authoritative. --sync-licenses regenerates package-local copies;
# --archives also extracts and checks the actual Cargo artifacts (never uploads).
set -euo pipefail

mode="${1:---check}"
case "$mode" in
  --check|--sync-licenses) shift "$(( $# > 0 ? 1 : 0 ))" ;;
  --archives) archives="${2:?expected package directory}"; shift 2 ;;
  *) echo "usage: $0 [--check|--sync-licenses|--archives DIRECTORY] [ROOT]" >&2; exit 2 ;;
esac
root="${1:-.}"
metadata="$(cargo metadata --manifest-path "$root/Cargo.toml" --no-deps --locked --offline --format-version 1)"
root="$(jq -r .workspace_root <<< "$metadata")"
inventory="$root/.github/scripts/public-crates.txt"

# Compare the complete publishable set, then require every local non-dev edge to
# have a registry version and an earlier entry. This also rejects unpublished
# runtime/build dependencies, duplicates, omissions and stale inventory entries.
jq -e --rawfile inventory "$inventory" '
  ($inventory | split("\n") | map(select(length > 0))) as $order |
  [.packages[] | select(.publish != [])] as $public |
  ($order | length > 0) and
  (($order | sort) == ([$public[].name] | sort)) and
  all($public[];
    . as $p |
    (.description | type == "string" and length > 0) and
    .license == "MIT OR Apache-2.0" and
    (.repository | type == "string" and length > 0) and
    (.rust_version | type == "string" and length > 0) and
    all(.dependencies[] | select(.kind != "dev" and .path != null);
      . as $d | .req != "*" and
      ($order | index($d.name)) != null and
      (($order | index($d.name)) < ($order | index($p.name)))))
' <<< "$metadata" >/dev/null || {
  echo "FAIL: publication metadata, runtime closure or dependency order: $inventory" >&2
  exit 1
}

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
jq -r '.packages[] | select(.publish != []) | [.name, .version, .manifest_path] | @tsv' \
  <<< "$metadata" > "$scratch/packages"
while IFS=$'\t' read -r name version manifest; do
  directory="${manifest%/Cargo.toml}"
  for text in LICENSE-MIT LICENSE-APACHE NOTICE THIRDPARTY.md; do
    if [ "$mode" = --sync-licenses ]; then
      cp "$root/$text" "$directory/$text"
    fi
    if ! cmp -s "$root/$text" "$directory/$text"; then
      echo "FAIL: $name/$text missing or stale; run $0 --sync-licenses" >&2
      exit 1
    fi
  done
  if [ "$mode" = --archives ]; then
    tar -xzf "$archives/$name-$version.crate" -C "$scratch"
    for text in LICENSE-MIT LICENSE-APACHE NOTICE THIRDPARTY.md; do
      if ! cmp -s "$root/$text" "$scratch/$name-$version/$text"; then
        echo "FAIL: packaged $name/$text missing or stale" >&2
        exit 1
      fi
    done
    echo "OK: extracted $name-$version.crate legal texts"
  fi
done < "$scratch/packages"
echo "OK: publication closure/order and package legal texts ($mode)"
