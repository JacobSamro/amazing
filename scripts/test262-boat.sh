#!/usr/bin/env bash
# Full Test262 on 16 Boat shards; fetch compact classifications only.
#
# One default sandbox runs the whole suite in ~13.6 minutes; four shards take
# ~3.5. Each shard is its own lane (t262-shard-K) so the runs never share a
# tree. Sandbox lifecycle remains with the orchestrator.
#
# Usage: scripts/test262-boat.sh <ref> <tag> [--shards N] [--baseline BASE.json]
#                              [--lane-prefix NAME] [--runner-ref REF]
#   Writes .agents/test262/t262-<shortsha>-<tag>.json (merged) and prints lost/gained
#   against the baseline. Exit 1 when a baseline pass was lost.

set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
REF="${1:?usage: test262-boat.sh <ref> <tag> [--shards N] [--baseline BASE.json]}"; TAG="${2:?tag}"; shift 2
N=16 BASE="" LANE_PREFIX=t262-shard RUNNER_REF=""
while [ $# -gt 0 ]; do
  case "$1" in
    --shards) N="$2"; shift 2 ;;
    --baseline) BASE="$2"; shift 2 ;;
    --lane-prefix) LANE_PREFIX="$2"; shift 2 ;;
    --runner-ref) RUNNER_REF="$2"; shift 2 ;;
    *) echo "test262-boat: unknown option $1" >&2; exit 2 ;;
  esac
done
[[ "$N" =~ ^[1-9][0-9]*$ ]] && [ "$N" -le 16 ] || { echo "shards must be 1..16" >&2; exit 2; }
[[ "$TAG" =~ ^[a-zA-Z0-9_-]+$ ]] || { echo "invalid tag" >&2; exit 2; }
[[ "$LANE_PREFIX" =~ ^[a-z0-9-]+$ ]] || { echo "invalid lane prefix" >&2; exit 2; }
COMMIT="$(git -C "$MAIN" rev-parse "$REF^{commit}")" || exit 2
SHA="${COMMIT:0:8}" || exit 2
OUTDIR="$MAIN/.agents/test262"; mkdir -p "$OUTDIR"
OUT="$OUTDIR/t262-$SHA-$TAG.json"
T0=$(date +%s)

# Always run the current runner (with --shard), whatever the ref ships.
if [ -n "$RUNNER_REF" ]; then
  RUNNER_B64="$(git -C "$MAIN" show "$RUNNER_REF:scripts/test262.py" | base64 | tr -d '\n')" || exit 2
else
  RUNNER_B64="$(base64 < "$REPO/scripts/test262.py" | tr -d '\n')"
fi

COMPACT_B64="$(base64 < "$REPO/scripts/gate-summary.py" | tr -d '\n')"
pids=()
for K in $(seq 0 $((N - 1))); do
  LANE="$LANE_PREFIX-$K"
  (
    if "$REPO/scripts/linux-run.sh" "$LANE" --ref "$COMMIT" -- "
      set -euo pipefail
      echo $COMPACT_B64 | base64 -d > /tmp/t262compact-$K.py
      phase=setup
      trap 'rc=\$?; trap - EXIT; if [ \$rc -ne 0 ]; then python3 /tmp/t262compact-$K.py failure --source-commit $COMMIT --shard $K/$N \"\$OUT/shard-$SHA-$TAG.failure.manifest.json\" \"\$phase\" \$rc \"\$OUT/shard-$SHA-$TAG-build.log\" /tmp/shard-$SHA-$TAG-$K-runner.log /tmp/shard-$SHA-$TAG-$K.json \"\$OUT/shard-$SHA-$TAG-packaging.log\" || echo compact-failure-manifest-unavailable; fi; exit \$rc' EXIT
      export CARGO_TARGET_DIR=/tmp/amazing-target-t262-shard-$K CARGO_INCREMENTAL=0
      mkdir -p /tmp/amazing-src-t262-shard-$K
      rsync -a --delete --exclude /target --exclude '/target-*' --exclude /corpora --exclude /out ./ /tmp/amazing-src-t262-shard-$K/
      ln -sfn \"\$HOME/corpora\" /tmp/amazing-src-t262-shard-$K/corpora
      cd /tmp/amazing-src-t262-shard-$K
      phase=build
      set +e
      cargo build -p amazing-js --example test262_host --release --locked >\"\$OUT/shard-$SHA-$TAG-build.log\" 2>&1
      build_rc=\$?
      set -e
      [ \$build_rc = 0 ] || exit \$build_rc
      echo $RUNNER_B64 | base64 -d > /tmp/t262runner.py
      phase=runner
      set +e
      python3 /tmp/t262runner.py \"\$TEST262_ROOT\" --binary \"\$CARGO_TARGET_DIR/release/examples/test262_host\" \
        --jobs \$JOBS --timeout 60 --shard $K/$N --output /tmp/shard-$SHA-$TAG-$K.json >/tmp/shard-$SHA-$TAG-$K-runner.log 2>&1
      result=\$?
      set -e
      [ \$result = 0 ] || [ \$result = 1 ] || exit \$result
      phase=packaging
      cp /tmp/shard-$SHA-$TAG-$K.json \"\$OUT/shard-$SHA-$TAG.json\"
      cp /tmp/shard-$SHA-$TAG-$K-runner.log \"\$OUT/shard-$SHA-$TAG-runner.log\"
      python3 /tmp/t262compact-$K.py test262 --source-commit $COMMIT \"\$OUT/shard-$SHA-$TAG.json\" \"\$OUT/shard-$SHA-$TAG.compact.json\" \$result >\"\$OUT/shard-$SHA-$TAG-packaging.log\" 2>&1
      echo shard-$K-exit=\$result
      :" > "$OUTDIR/.shard-$SHA-$TAG-$K.log" 2>&1; then
      : # SSH job completed successfully; only now collect the compact receipt.
    else
      job_rc=$?
      echo "test262-boat: shard $K job exit $job_rc; raw stays on Boat" >&2
      D="$(mktemp -d)"
      if bash "$REPO/scripts/lane-fetch.sh" "$LANE" "$D" "amazing/out/$LANE/shard-$SHA-$TAG.failure.manifest.json"; then
        mv "$D/shard-$SHA-$TAG.failure.manifest.json" "$OUTDIR/.shard-$SHA-$TAG-$K.failure.manifest.json"
      else
        echo "test262-boat: shard $K phase receipt unavailable; wrapper exit remains $job_rc" >&2
      fi
      rmdir "$D" 2>/dev/null || true
      exit "$job_rc"
    fi
    rm -f "$OUTDIR/.shard-$SHA-$TAG-$K.json"
    D="$(mktemp -d)"
    bash "$REPO/scripts/lane-fetch.sh" "$LANE" "$D" "amazing/out/$LANE/shard-$SHA-$TAG.compact.json" &&
      python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["shard"] == sys.argv[2]; assert d["job_exit"] == 0 and d["compact_records"] and d["host_sha256"] and d["input_sha256"]' "$D/shard-$SHA-$TAG.compact.json" "$K/$N" &&
      mv "$D/shard-$SHA-$TAG.compact.json" "$OUTDIR/.shard-$SHA-$TAG-$K.json"
    if [ ! -s "$OUTDIR/.shard-$SHA-$TAG-$K.json" ]; then
      echo "test262-boat: shard $K compact fetch failed; raw remains on Boat" >&2
      rm -rf "$D"
      exit 1
    fi
    rm -rf "$D"
  ) &
  pids+=($!)
done
failed=0
for p in "${pids[@]}"; do wait "$p" || failed=1; done
[ "$failed" = 0 ] || { echo "test262-boat: a shard failed; no stale output qualifies the run" >&2; exit 3; }

SHARDS=()
bytes=0
for K in $(seq 0 $((N - 1))); do
  f="$OUTDIR/.shard-$SHA-$TAG-$K.json"
  [ -s "$f" ] || { echo "test262-boat: shard $K produced no output; see ${f%.json}.log" >&2; tail -5 "${f%.json}.log" >&2; exit 3; }
  bytes=$((bytes + $(wc -c < "$f")))
  [ "$bytes" -le 2000000 ] || { echo "test262-boat: compact shards exceed ticket budget" >&2; exit 3; }
  SHARDS+=("$f")
done
echo "test262-boat: $SHA $TAG, $N shards, $(( $(date +%s) - T0 ))s"
if [ -n "$BASE" ]; then
  python3 "$REPO/scripts/test262-merge.py" "$OUT" "${SHARDS[@]}" --baseline "$BASE"
else
  python3 "$REPO/scripts/test262-merge.py" "$OUT" "${SHARDS[@]}"
fi
