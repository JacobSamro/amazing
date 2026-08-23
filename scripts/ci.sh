#!/bin/sh
# Amazing CI — minimal gate required by Phase 0 exit criteria.
# Runs on any machine with cargo (canonical execution host: win-001).
set -e
cd "$(dirname "$0")/.."
echo "== amazing ci: build =="
cargo build --workspace --all-targets --locked
echo "== amazing ci: test =="
cargo test --workspace --locked
echo "== amazing ci: ok =="
