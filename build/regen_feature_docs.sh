#!/usr/bin/env bash
set -euo pipefail

# Thin compatibility entrypoint; the Rust command owns validation and output.
exec cargo run -p selene-db-profile --bin selene-profile -- "$@"
