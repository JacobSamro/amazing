#!/usr/bin/env bash
# Full Test262 on N Boat sandboxes in parallel, merged and compared locally.
#
# One default sandbox runs the whole suite in ~13.6 minutes; four shards take
# ~3.5. Each shard is its own lane (t262-shard-K) so the runs never share a
# tree; each shard stops its sandbox as soon as its result is copied back.
#
# Usage: scripts/test262-boat.sh <ref> <tag> [--shards N] [--baseline BASE.json]
#                              [--resume] [--lane-prefix NAME] [--runner-ref REF]
#   Writes .agents/test262/t262-<shortsha>-<tag>.json (merged) and prints lost/gained
#   against the baseline. Exit 1 when a baseline pass was lost.

set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
REF="${1:?usage: test262-boat.sh <ref> <tag> [--shards N] [--baseline BASE.json]}"; TAG="${2:?tag}"; shift 2
N=4 BASE="" RESUME=0 LANE_PREFIX=t262-shard RUNNER_REF=""
while [ $# -gt 0 ]; do
  case "$1" in
    --shards) N="$2"; shift 2 ;;
    --baseline) BASE="$2"; shift 2 ;;
    --resume) RESUME=1; shift ;;
    --lane-prefix) LANE_PREFIX="$2"; shift 2 ;;
    --runner-ref) RUNNER_REF="$2"; shift 2 ;;
    *) echo "test262-boat: unknown option $1" >&2; exit 2 ;;
  esac
done
[[ "$LANE_PREFIX" =~ ^[a-z0-9-]+$ ]] || { echo "invalid lane prefix" >&2; exit 2; }
SHA="$(git -C "$MAIN" rev-parse --short=8 "$REF")" || exit 2
OUTDIR="$MAIN/.agents/test262"; mkdir -p "$OUTDIR"
OUT="$OUTDIR/t262-$SHA-$TAG.json"
T0=$(date +%s)

# Always run the current runner (with --shard), whatever the ref ships.
if [ -n "$RUNNER_REF" ]; then
  RUNNER_B64="$(git -C "$MAIN" show "$RUNNER_REF:scripts/test262.py" | base64 | tr -d '\n')" || exit 2
else
  RUNNER_B64="$(base64 < "$REPO/scripts/test262.py" | tr -d '\n')"
fi

pids=()
for K in $(seq 0 $((N - 1))); do
  LANE="$LANE_PREFIX-$K"
  if [ "$RESUME" = 1 ] && [ -s "$OUTDIR/.shard-$SHA-$TAG-$K.json" ]; then
    echo "test262-boat: retaining completed shard $K"
    continue
  fi
  (
    "$REPO/scripts/linux-run.sh" "$LANE" --ref "$SHA" -- "
      set -euo pipefail
      export CARGO_TARGET_DIR=/tmp/amazing-target-t262-shard-$K CARGO_INCREMENTAL=0
      mkdir -p /tmp/amazing-src-t262-shard-$K
      rsync -a --delete --exclude /target --exclude '/target-*' --exclude /corpora --exclude /out ./ /tmp/amazing-src-t262-shard-$K/
      ln -sfn \"\$HOME/corpora\" /tmp/amazing-src-t262-shard-$K/corpora
      cd /tmp/amazing-src-t262-shard-$K
      cargo build -p amazing-js --example test262_host --release --locked >\"\$OUT/shard-$SHA-$TAG-build.log\" 2>&1 || {
        tail -40 \"\$OUT/shard-$SHA-$TAG-build.log\"; exit 1;
      }
      echo $RUNNER_B64 | base64 -d > /tmp/t262runner.py
      set +e
      python3 /tmp/t262runner.py \"\$TEST262_ROOT\" --binary \"\$CARGO_TARGET_DIR/release/examples/test262_host\" \
        --jobs \$JOBS --timeout 60 --shard $K/$N --output /tmp/shard-$SHA-$TAG-$K.json >/tmp/shard-$SHA-$TAG-$K-runner.log 2>&1
      result=\$?
      set -e
      echo shard-$K-exit=\$result
      if ! test -s /tmp/shard-$SHA-$TAG-$K.json; then
        tail -60 /tmp/shard-$SHA-$TAG-$K-runner.log
        exit 1
      fi
      cp /tmp/shard-$SHA-$TAG-$K.json \"\$OUT/shard-$SHA-$TAG.json\"
      test -s \"\$OUT/shard-$SHA-$TAG.json\"" > "$OUTDIR/.shard-$SHA-$TAG-$K.log" 2>&1
    D="$(mktemp -d)"
    "$MAIN/scripts/lane-fetch.sh" "$LANE" "$D" "amazing/out/$LANE/shard-$SHA-$TAG.json" &&
      python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["shard"] == sys.argv[2]; assert d["records"] and d["host_sha256"] and d["input_sha256"]' "$D/shard-$SHA-$TAG.json" "$K/$N" &&
      mv "$D/shard-$SHA-$TAG.json" "$OUTDIR/.shard-$SHA-$TAG-$K.json"
    if [ ! -s "$OUTDIR/.shard-$SHA-$TAG-$K.json" ]; then
      echo "test262-boat: shard $K fetch incomplete; keeping $LANE running for recovery" >&2
      rm -rf "$D"
      exit 1
    fi
    rm -rf "$D"
    # Shards are single-use: stop now instead of billing 5 idle minutes.
    # ...unless another run already holds this shard (its lane lock), which a
    # stop would kill mid-run.
    [ "$("$MAIN/scripts/linux-run.sh" "$LANE" --which)" = boat ] &&
      [ ! -d "$MAIN/.agents/.remote-locks/linux-run-$LANE" ] &&
      "$MAIN/scripts/boat-lane.py" stop "$LANE" >/dev/null 2>&1
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
