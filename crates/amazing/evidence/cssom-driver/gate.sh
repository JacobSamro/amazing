#!/usr/bin/env bash
set -euo pipefail
label=${1:?before/after label required}
baseline=${2:-}
export CARGO_TARGET_DIR=/tmp/amazing-target-integrator-cssom CARGO_INCREMENTAL=0
mkdir -p /tmp/amazing-src-integrator-cssom
rsync -a --delete --exclude /target --exclude '/target-*' --exclude /corpora --exclude /out ./ /tmp/amazing-src-integrator-cssom/
ln -sfn "$HOME/corpora" /tmp/amazing-src-integrator-cssom/corpora
cd /tmp/amazing-src-integrator-cssom
cargo build -p amazing --release --locked -j "$JOBS" 2>&1 | tee "$OUT/cssom-$label-build.log"
native_result=0
doc_result=0
corpus_result=0
cargo nextest run --workspace --locked --no-fail-fast --test-threads 4 2>&1 | tee "$OUT/cssom-$label-nextest.log" || native_result=$?
cargo test --workspace --locked --doc 2>&1 | tee "$OUT/cssom-$label-doc.log" || doc_result=$?
bash scripts/ensure-wpt.sh
comparison=()
if [ -n "$baseline" ]; then comparison=(--baseline "$baseline" --recheck-timeouts); fi
python3 crates/amazing/evidence/cssom-driver/run-corpus.py "$label" --jobs 8 --timeout-ms 5000 "${comparison[@]}" 2>&1 | tee "$OUT/cssom-$label-corpus.log" || corpus_result=$?
sha256sum "$CARGO_TARGET_DIR/release/amazing" | tee "$OUT/cssom-$label-binary.sha256"
if [ "$native_result" -ne 0 ] || [ "$doc_result" -ne 0 ] || [ "$corpus_result" -ne 0 ]; then exit 1; fi
