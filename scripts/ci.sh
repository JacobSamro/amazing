#!/bin/sh
# Ferric CI — minimal gate required by Phase 0 exit criteria.
# Runs on any machine with cargo (canonical execution host: win-001).
set -e
cd "$(dirname "$0")/.."
echo "== ferric ci: build =="
cargo build --workspace --all-targets
echo "== ferric ci: test =="
cargo test --workspace
echo "== ferric ci: ok =="
