#!/bin/bash
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ACTION=${1:-verify}

case "$ACTION" in
  capture|install|verify)
    exec python3 -B "$ROOT/scripts/v2_baseline.py" "$ACTION" --root "$ROOT"
    ;;
  *)
    printf 'usage: %s {capture|install|verify}\n' "$0" >&2
    exit 2
    ;;
esac
