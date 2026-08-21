#!/bin/sh
# Amazing CI — minimal gate required by Phase 0 exit criteria.
# Runs on any machine with cargo (canonical execution host: win-001).
set -e
cd "$(dirname "$0")/.."
echo "== amazing ci: build =="
cargo build --workspace --all-targets
echo "== amazing ci: test =="
cargo test --workspace
echo "== amazing ci: ok =="
