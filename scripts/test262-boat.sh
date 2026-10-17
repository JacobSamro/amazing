#!/usr/bin/env bash
# Full Test262 on 16 Boat shards; fetch compact classifications only.
#
# One default sandbox runs the whole suite in ~13.6 minutes; four shards take
# ~3.5. Each shard is its own lane (t262-shard-K) so the runs never share a
# tree. Sandbox lifecycle remains with the orchestrator.
#
# Usage: scripts/test262-boat.sh <ref> <tag> [--shards N] [--baseline BASE.json]
#                              [--lane-prefix NAME] [--runner-ref REF]
#                              [--boxes N] (optional shared-build placement)
#   Writes .agents/test262/t262-<shortsha>-<tag>.json (merged) and prints lost/gained
#   against the baseline. Exit 1 when a baseline pass was lost.

set -uo pipefail
ORIGINAL_ARGS=("$@")
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
REF="${1:?usage: test262-boat.sh <ref> <tag> [--shards N] [--baseline BASE.json]}"; TAG="${2:?tag}"; shift 2
N=16 BASE="" LANE_PREFIX=t262-shard RUNNER_REF="" BOXES=""
while [ $# -gt 0 ]; do
  case "$1" in
    --shards) N="$2"; shift 2 ;;
    --baseline) BASE="$2"; shift 2 ;;
    --lane-prefix) LANE_PREFIX="$2"; shift 2 ;;
    --runner-ref) RUNNER_REF="$2"; shift 2 ;;
    --boxes) BOXES="$2"; shift 2 ;;
    *) echo "test262-boat: unknown option $1" >&2; exit 2 ;;
  esac
done
[[ "$N" =~ ^[1-9][0-9]*$ ]] && [ "$N" -le 16 ] || { echo "shards must be 1..16" >&2; exit 2; }
[[ "$TAG" =~ ^[a-zA-Z0-9_-]+$ ]] || { echo "invalid tag" >&2; exit 2; }
[[ "$LANE_PREFIX" =~ ^[a-z0-9-]+$ ]] || { echo "invalid lane prefix" >&2; exit 2; }
if [ -n "$BOXES" ]; then
  exec python3 "$REPO/scripts/test262-pooled.py" "${ORIGINAL_ARGS[@]}"
fi
COMMIT="$(git -C "$MAIN" rev-parse "$REF^{commit}")" || exit 2
SHA="${COMMIT:0:8}" || exit 2
OUTDIR="$MAIN/.agents/test262"; mkdir -p "$OUTDIR"
OUT="$OUTDIR/t262-$SHA-$TAG.json"
T0=$(date +%s)
RUN_ID="$TAG-$(date -u +%Y%m%dT%H%M%S)-$$"
SHARDROOT="$OUTDIR/runs/$RUN_ID"; mkdir -p "$SHARDROOT"

# Always run the current runner (with --shard), whatever the ref ships.
if [ -n "$RUNNER_REF" ]; then
  RUNNER_B64="$(git -C "$MAIN" show "$RUNNER_REF:scripts/test262.py" | base64 | tr -d '\n')" || exit 2
else
  RUNNER_B64="$(base64 < "$REPO/scripts/test262.py" | tr -d '\n')"
fi

COMPACT_B64="$(base64 < "$REPO/scripts/gate-summary.py" | tr -d '\n')"
COMMAND_B64="$(base64 < "$REPO/scripts/gate-command.py" | tr -d '\n')"
pids=()
for K in $(seq 0 $((N - 1))); do
  LANE="$LANE_PREFIX-$K"
  (
    remote_root="amazing/out/$LANE/test262-$RUN_ID/$COMMIT"
    endpoint_receipt="$OUTDIR/endpoints/$RUN_ID/$K/endpoint.manifest.json"
    setup_receipt="$SHARDROOT/shard-$K.setup.manifest.json"
    observer_rc=0
    if "$REPO/scripts/linux-run.sh" "$LANE" --ref "$COMMIT" --endpoint-receipt "$endpoint_receipt" --compact-root "$remote_root" --setup-summary "$setup_receipt" -- "
      set -euo pipefail
      export OUT=\"\$OUT/test262-$RUN_ID/$COMMIT\"
      mkdir -p \"\$OUT\"
      ${T262_EXTRA_ENV:+export $T262_EXTRA_ENV}
      echo $COMPACT_B64 | base64 -d > /tmp/t262compact-$K.py
      echo $COMMAND_B64 | base64 -d > /tmp/t262command-$K.py
      phase=setup
      phase_receipt=
      trap 'rc=\$?; trap - EXIT; if [ \$rc -ne 0 ]; then python3 /tmp/t262compact-$K.py failure --source-commit $COMMIT --shard $K/$N --phase-receipt \"\$phase_receipt\" \"\$OUT/shard-$SHA-$TAG.failure.manifest.json\" \"\$phase\" \$rc \"\$OUT/shard-$SHA-$TAG-build.log\" \"\$OUT/shard-$SHA-$TAG-runner.log\" /tmp/shard-$SHA-$RUN_ID-$K.json \"\$OUT/shard-$SHA-$TAG-packaging.log\" || echo compact-failure-manifest-unavailable; fi; exit \$rc' EXIT
      export CARGO_TARGET_DIR=/tmp/amazing-target-t262-shard-$K-$COMMIT CARGO_INCREMENTAL=0
      mkdir -p /tmp/amazing-src-t262-shard-$K-$COMMIT
      rsync -a --delete --exclude /target --exclude '/target-*' --exclude /corpora --exclude /out ./ /tmp/amazing-src-t262-shard-$K-$COMMIT/
      ln -sfn \"\$NATIVE_CORPORA_ROOT\" /tmp/amazing-src-t262-shard-$K-$COMMIT/corpora
      cd /tmp/amazing-src-t262-shard-$K-$COMMIT
      phase=build
      phase_receipt=\"\$OUT/shard-$SHA-$TAG-build.phase.manifest.json\"
      set +e
      python3 /tmp/t262command-$K.py --phase build --source-commit $COMMIT --output \"\$OUT/shard-$SHA-$TAG-build.log\" --receipt \"\$phase_receipt\" -- cargo build -p amazing-js --example test262_host --release --locked
      build_rc=\$?
      set -e
      [ \$build_rc = 0 ] || exit \$build_rc
      echo $RUNNER_B64 | base64 -d > /tmp/t262runner.py
      phase=runner
      phase_receipt=\"\$OUT/shard-$SHA-$TAG-runner.phase.manifest.json\"
      set +e
      python3 /tmp/t262command-$K.py --phase runner --source-commit $COMMIT --output \"\$OUT/shard-$SHA-$TAG-runner.log\" --receipt \"\$phase_receipt\" --copy /tmp/shard-$SHA-$RUN_ID-$K.json=\"\$OUT/shard-$SHA-$TAG.json\" -- python3 /tmp/t262runner.py \"\$TEST262_ROOT\" --binary \"\$CARGO_TARGET_DIR/release/examples/test262_host\" \
        --jobs \$JOBS --timeout 60 --shard $K/$N --output /tmp/shard-$SHA-$RUN_ID-$K.json
      result=\$?
      set -e
      [ \$result = 0 ] || [ \$result = 1 ] || exit \$result
      python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d[\"output_copy_exit\"] == 0 and not d.get(\"receipt_copy_error\")' \"\$phase_receipt\"
      phase=packaging
      phase_receipt=\"\$OUT/shard-$SHA-$TAG-packaging.phase.manifest.json\"
      python3 /tmp/t262command-$K.py --phase packaging --source-commit $COMMIT --output \"\$OUT/shard-$SHA-$TAG-packaging.log\" --receipt \"\$phase_receipt\" -- python3 /tmp/t262compact-$K.py test262 --source-commit $COMMIT \"\$OUT/shard-$SHA-$TAG.json\" \"\$OUT/shard-$SHA-$TAG.compact.json\" \$result
      echo shard-$K-exit=\$result
      :" > "$SHARDROOT/shard-$K.log" 2>&1; then
      :
    else
      observer_rc=$?
    fi
    native_receipt="$SHARDROOT/shard-$K.native.summary.json"
    python3 "$REPO/scripts/test262-job-state.py" --receipt "$endpoint_receipt" \
      --lane "$LANE" --source "$COMMIT" --observer-exit "$observer_rc" \
      --seconds 3600 --output "$native_receipt" --questions "$SHARDROOT/shard-$K.questions.md"
    native_rc=$?
    if ! python3 - "$native_receipt" <<'PYNATIVE'
import json,sys
row=json.load(open(sys.argv[1]))
raise SystemExit(0 if row.get('compact_fetch_allowed') is True and row.get('state')=='TERMINAL' and type(row.get('actual_exit')) is int and type(row.get('runner_exit')) is int else 1)
PYNATIVE
    then
      echo "test262-boat: shard $K native state unresolved; $native_receipt; questions in $SHARDROOT/shard-$K.questions.md" >&2
      exit 3
    fi
    if [ "$native_rc" != 0 ]; then
      echo "test262-boat: shard $K authenticated native exit $native_rc; observer exit $observer_rc; raw stays on Boat" >&2
      D="$(mktemp -d)"
      if bash "$REPO/scripts/lane-fetch.sh" "$LANE" "$D" --endpoint-receipt "$endpoint_receipt" --source "$COMMIT" "$remote_root/shard-$SHA-$TAG.failure.manifest.json"; then
        mv "$D/shard-$SHA-$TAG.failure.manifest.json" "$SHARDROOT/shard-$K.failure.manifest.json"
      else
        echo "test262-boat: shard $K compact phase receipt unavailable; authenticated native exit remains $native_rc" >&2
      fi
      rmdir "$D" 2>/dev/null || true
      exit "$native_rc"
    fi
    D="$(mktemp -d)"
    bash "$REPO/scripts/lane-fetch.sh" "$LANE" "$D" --endpoint-receipt "$endpoint_receipt" --source "$COMMIT" "$remote_root/shard-$SHA-$TAG.compact.json" &&
      python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["shard"] == sys.argv[2]; assert d["job_exit"] == 0 and d["compact_records"] and d["host_sha256"] and d["input_sha256"]' "$D/shard-$SHA-$TAG.compact.json" "$K/$N" &&
      mv "$D/shard-$SHA-$TAG.compact.json" "$SHARDROOT/shard-$K.compact.json"
    if [ ! -s "$SHARDROOT/shard-$K.compact.json" ]; then
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
[ "$failed" = 0 ] || {
  echo "test262-boat: a shard failed; no stale output qualifies the run; fresh receipts/questions $SHARDROOT" >&2
  exit 3
}

SHARDS=()
bytes=0
for K in $(seq 0 $((N - 1))); do
  f="$SHARDROOT/shard-$K.compact.json"
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
