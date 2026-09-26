#!/usr/bin/env bash
# Full Test262 on N Boat sandboxes in parallel, merged and compared locally.
#
# One default sandbox runs the whole suite in ~13.6 minutes; four shards take
# ~3.5. Each shard is its own lane (t262-shard-K) so the runs never share a
# tree; each shard stops its sandbox as soon as its result is copied back.
#
# Usage: scripts/test262-boat.sh <ref> <tag> [--shards N] [--baseline BASE.json]
#   Writes .agents/test262/t262-<shortsha>-<tag>.json (merged) and prints lost/gained
#   against the baseline. Exit 1 when a baseline pass was lost.

set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
REF="${1:?usage: test262-boat.sh <ref> <tag> [--shards N] [--baseline BASE.json]}"; TAG="${2:?tag}"; shift 2
N=4 BASE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --shards) N="$2"; shift 2 ;;
    --baseline) BASE="$2"; shift 2 ;;
    *) echo "test262-boat: unknown option $1" >&2; exit 2 ;;
  esac
done
SHA="$(git -C "$MAIN" rev-parse --short=8 "$REF")" || exit 2
OUTDIR="$MAIN/.agents/test262"; mkdir -p "$OUTDIR"
OUT="$OUTDIR/t262-$SHA-$TAG.json"
T0=$(date +%s)

# Always run the current runner (with --shard), whatever the ref ships.
RUNNER_B64="$(base64 < "$REPO/scripts/test262.py" | tr -d '\n')"

pids=()
for K in $(seq 0 $((N - 1))); do
  (
    "$REPO/scripts/linux-run.sh" "t262-shard-$K" --ref "$SHA" -- "
      cargo build -p amazing-js --example test262_host --release --locked 2>&1 | grep -E '^error' -A7 | head -20
      echo $RUNNER_B64 | base64 -d > /tmp/t262runner.py
      python3 /tmp/t262runner.py \"\$TEST262_ROOT\" --binary \"\$CARGO_TARGET_DIR/release/examples/test262_host\" \
        --jobs \$JOBS --timeout 60 --shard $K/$N --output \$OUT/shard-$SHA-$TAG.json >/dev/null 2>&1
      echo shard-$K-exit=\$?" > "$OUTDIR/.shard-$SHA-$TAG-$K.log" 2>&1
    D="$(mktemp -d)"
    "$MAIN/scripts/lane-fetch.sh" "t262-shard-$K" "$D" "amazing/out/t262-shard-$K/shard-$SHA-$TAG.json" &&
      mv "$D/shard-$SHA-$TAG.json" "$OUTDIR/.shard-$SHA-$TAG-$K.json"; rm -rf "$D"
    # Shards are single-use: stop now instead of billing 5 idle minutes.
    # ...unless another run already holds this shard (its lane lock), which a
    # stop would kill mid-run.
    [ "$("$MAIN/scripts/linux-run.sh" "t262-shard-$K" --which)" = boat ] &&
      [ ! -d "$MAIN/.agents/.remote-locks/linux-run-t262-shard-$K" ] &&
      "$MAIN/scripts/boat-lane.py" stop "t262-shard-$K" >/dev/null 2>&1
  ) &
  pids+=($!)
done
for p in "${pids[@]}"; do wait "$p"; done

SHARDS=()
for K in $(seq 0 $((N - 1))); do
  f="$OUTDIR/.shard-$SHA-$TAG-$K.json"
  [ -s "$f" ] || { echo "test262-boat: shard $K produced no output; see ${f%.json}.log" >&2; tail -5 "${f%.json}.log" >&2; exit 3; }
  SHARDS+=("$f")
done
echo "test262-boat: $SHA $TAG, $N shards, $(( $(date +%s) - T0 ))s"
if [ -n "$BASE" ]; then
  python3 "$REPO/scripts/test262-merge.py" "$OUT" "${SHARDS[@]}" --baseline "$BASE"
else
  python3 "$REPO/scripts/test262-merge.py" "$OUT" "${SHARDS[@]}"
fi
