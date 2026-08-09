#!/bin/sh
# Abbey Bot gate — fmt + clippy + tests. Mirrors the sibling Rust projects.
#
# Note the deliberate absence of a pipe anywhere below: `cmd | tail` reports
# tail's exit status, not cmd's, which is how a red suite reads as green.
set -eu
cd "$(dirname "$0")"

echo "== fmt =="
cargo fmt --all -- --check

echo "== clippy =="
cargo clippy --all-targets -- -D warnings

echo "== test =="
cargo test

echo "== ok =="
