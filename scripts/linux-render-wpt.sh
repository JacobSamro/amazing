#!/usr/bin/env bash
# Exact-ref workspace/docs + static render gate. Raw artifacts stay on Boat.
# --baseline-ref runs the same selection on that ref first, then compares candidate.
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
LANE="${1:?usage: linux-render-wpt.sh LANE DIR... --ref SHA [--workspace] [--baseline-ref SHA] [--compact]}"; shift
SUBDIRS=(); ARGS=(); REF= BASE_REF= TAG="render-$(date -u +%Y%m%dT%H%M%S)-$$"
while [ $# -gt 0 ]; do
 case "$1" in
  --ref) REF="$2";shift 2;;
  --baseline-ref) BASE_REF="$2";shift 2;;
  --tag) TAG="$2";shift 2;;
  --workspace|--release) ARGS+=("$1");shift;;
  --compact) shift;; # Always compact; retained for compatible callers.
  --width|--height|--timeout|--jobs|--limit|--baseline) ARGS+=("$1" "$2");shift 2;;
  --*) echo "unknown option $1" >&2;exit 2;;
  *) SUBDIRS+=("$1");shift;;
 esac
done
[ -n "$REF" ] && [ ${#SUBDIRS[@]} -gt 0 ] || { echo 'explicit --ref and directories required' >&2;exit 2; }
[[ "$LANE" =~ ^[A-Za-z0-9_-]+$ && "$TAG" =~ ^[A-Za-z0-9_-]+$ ]] || exit 2
for dir in "${SUBDIRS[@]}";do [[ "$dir" =~ ^[A-Za-z0-9_-]+(/[A-Za-z0-9_-]+)*$ ]] || exit 2;done
REF=$(git -C "$MAIN" rev-parse "$REF^{commit}") || exit 2
[ -z "$BASE_REF" ] || BASE_REF=$(git -C "$MAIN" rev-parse "$BASE_REF^{commit}") || exit 2
# Inject current supported tooling; engine/fixtures are always the recorded exact ref.
HELPERS=$(python3 - "$REPO" <<'PY'
import base64,json,pathlib,sys
root=pathlib.Path(sys.argv[1])/'scripts'
print(base64.b64encode(json.dumps({n:base64.b64encode((root/n).read_bytes()).decode() for n in ('render-gate.py','gate-command.py','gate-summary.py')}).encode()).decode())
PY
)
run_side() {
 local source="$1" baseline="$2" side="${3:-after}" argq="" rc fetchrc=0 slug
 local side_tag="$TAG-$side"
 for arg in ${ARGS[@]+"${ARGS[@]}"} "${SUBDIRS[@]}";do argq="$argq $(printf '%q' "$arg")";done
 local cmd="tools=\$(mktemp -d /tmp/amazing-render-tools.XXXXXX) || exit 3
 python3 - \"\$tools\" '$HELPERS' <<'PY'
import base64,json,pathlib,sys
root=pathlib.Path(sys.argv[1])
for name,data in json.loads(base64.b64decode(sys.argv[2])).items():(root/name).write_bytes(base64.b64decode(data))
PY
 python3 \"\$tools/render-gate.py\" --source '$source' --tag '$side_tag' ${baseline:+--baseline-ref '$baseline' --baseline-tag '$TAG-before'} $argq"
 local endpoint="$MAIN/.agents/$LANE/render-gate/$side_tag/$source/endpoint.manifest.json"
 bash "$REPO/scripts/linux-run.sh" "$LANE" --ref "$source" --endpoint-receipt "$endpoint" --compact-root "amazing/out/$LANE/render-gate/$side_tag/$source" --setup-summary "$MAIN/.agents/$LANE/render-gate/$side_tag/$source/setup.manifest.json" -- "$cmd"
 rc=$?
 local dest="$MAIN/.agents/$LANE/render-gate/$side_tag/$source"
 bash "$REPO/scripts/lane-fetch.sh" "$LANE" "$dest" --endpoint-receipt "$endpoint" --source "$source" "amazing/out/$LANE/render-gate/$side_tag/$source/gate.summary.json" || fetchrc=3
 if [ "$fetchrc" = 0 ];then
  while IFS= read -r map;do
   bash "$REPO/scripts/lane-fetch.sh" "$LANE" "$dest" --endpoint-receipt "$endpoint" --source "$source" "amazing/out/$LANE/render-gate/$side_tag/$source/$map" </dev/null || fetchrc=3
  done < <(python3 - "$dest/gate.summary.json" <<'PYMAP'
import json,sys
for slug,row in json.load(open(sys.argv[1]))['renders'].items():
 if 'raw_compact' in row:print(slug+'.compact.json')
PYMAP
)
  python3 - "$dest" <<'PYBUDGET' || fetchrc=3
import pathlib,sys
root=pathlib.Path(sys.argv[1]);files=list(root.glob('*.json'))
if sum(p.stat().st_size for p in files)>2097152:raise SystemExit('render gate compact ticket exceeds 2MiB')
PYBUDGET
 fi
 [ "$fetchrc" = 0 ] || return "$fetchrc"
 return "$rc"
}
if [ -n "$BASE_REF" ];then run_side "$BASE_REF" "" before || exit $?;fi
run_side "$REF" "$BASE_REF" after
