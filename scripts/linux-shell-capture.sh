#!/usr/bin/env bash
# Actual-window product proof, separate from compact conformance gate evidence.
# --ref is required: captures identify the exact built source commit.
# Usage: linux-shell-capture.sh LANE --ref REF [--size WxH] [--wait S]
#   [--name N] [--release] [--scroll N] [--actions FILE] -- [shell args...]
#   or: ... --real-pages [--limit N] [--only SLUG] [--baseline auto|DIR] [--fuzz PCT]
# Action lines: key TEXT / type TEXT / wait SECONDS / shot NAME.
# Each image <=512 KiB; the complete manifest and images <=2 MiB. Use --only
# or --limit for a bounded real-pages proof. Logs, diffs and full reports stay
# on Boat. Local artifacts are explicit PNGs and *.window.manifest.json only.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
LANE="${1:?lane required}"; shift
REF= SIZE=1280x800 WAIT=8 NAME="capture-$(date -u +%Y%m%dT%H%M%SZ)" PROFILE=debug SCROLL=0
REAL_PAGES=0 LIMIT=0 ONLY= BASELINE= FUZZ=1 ACTIONS=
while [ $# -gt 0 ]; do
  case "$1" in
    --ref) REF="$2"; shift 2;;
    --size) SIZE="$2"; shift 2;;
    --wait) WAIT="$2"; shift 2;;
    --name) NAME="$2"; shift 2;;
    --release) PROFILE=release; shift;;
    --scroll) SCROLL="$2"; shift 2;;
    --actions) ACTIONS="$2"; shift 2;;
    --real-pages) REAL_PAGES=1; BASELINE=auto; shift;;
    --limit) LIMIT="$2"; shift 2;;
    --only) ONLY="$2"; shift 2;;
    --baseline) BASELINE="$2"; shift 2;;
    --fuzz) FUZZ="$2"; shift 2;;
    --) shift; break;;
    *) echo "unknown capture option: $1" >&2; exit 2;;
  esac
done
[[ "$LANE" =~ ^[a-z0-9-]+$ && "$NAME" =~ ^[A-Za-z0-9_-]+$ && "$SIZE" =~ ^[1-9][0-9]*x[1-9][0-9]*$ ]] || { echo 'invalid lane/name/size' >&2; exit 2; }
[ -n "$REF" ] || { echo 'actual window proof requires --ref REF' >&2; exit 2; }
SOURCE="$(git -C "$REPO" rev-parse --verify "$REF^{commit}")"
# Quote every supplied argument before constructing the remote command.
ASSIGN="$(python3 - "$LANE" "$SOURCE" "$SIZE" "$WAIT" "$NAME" "$PROFILE" "$SCROLL" "$REAL_PAGES" "$LIMIT" "$ONLY" "$BASELINE" "$FUZZ" "$ACTIONS" "$@" <<'PY'
import shlex, sys
names = 'LANE SOURCE SIZE WAIT NAME PROFILE SCROLL REAL_PAGES LIMIT ONLY BASELINE FUZZ ACTIONS'.split()
for name, value in zip(names, sys.argv[1:14]): print(name + '=' + shlex.quote(value))
print('SHELL_ARGS=(' + shlex.join(sys.argv[14:]) + ')')
PY
)"
HELPER="$(base64 < "$REPO/scripts/window-proof.py" | tr -d '\n')"
REMOTE_BODY="$(cat <<'REMOTE'
(
set -euo pipefail
ROOT="$PWD"
PROOF_TOOL=$(mktemp /tmp/window-proof.XXXXXX.py)
printf '%s' "$HELPER" | base64 -d > "$PROOF_TOOL"
APID= XPID=
trap 'kill ${APID:-} ${XPID:-} 2>/dev/null || true; rm -f "$PROOF_TOOL"' EXIT
cd crates/amazing-shell-gpui
export CARGO_TARGET_DIR=/tmp/amazing-shell-target-$LANE CARGO_INCREMENTAL=0
BUILD_FLAG=(); [ "$PROFILE" != release ] || BUILD_FLAG=(--release)
cargo build --locked "${BUILD_FLAG[@]}" --bin amazing-shell
EXE="$CARGO_TARGET_DIR/$PROFILE/amazing-shell"
cd "$ROOT"
if [ "$REAL_PAGES" = 1 ]; then SUBDIR=real-pages; else SUBDIR=captures; fi
OUTDIR="$OUT/$SUBDIR/$NAME"
# A proof name is a unique run; never append to or overwrite an earlier proof.
mkdir -p "$OUT/$SUBDIR"
mkdir "$OUTDIR"
MANIFEST="$OUTDIR/$NAME.window.manifest.json"
DISP=:$((90 + $$ % 9))
Xvfb "$DISP" -screen 0 "${SIZE}x24" -nolisten tcp >/dev/null 2>&1 & XPID=$!
sleep 1
export DISPLAY="$DISP" WAYLAND_DISPLAY=
export VK_ICD_FILENAMES="$(find /usr/share/vulkan/icd.d -maxdepth 1 -name 'lvp_icd*.json' | head -1)"
start_shell() {
  "$EXE" "$@" --size "$SIZE" > "$OUTDIR/shell.log" 2>&1 & APID=$!
  sleep "$WAIT"
  WID=$(xdotool search --pid "$APID" 2>/dev/null | tail -1) || true
  [ -n "$WID" ] || { echo "no actual Amazing window; raw log remains $OUTDIR/shell.log" >&2; return 5; }
}
shot() {
  [[ "$1" =~ ^[A-Za-z0-9_-]+$ ]] || { echo 'invalid shot name' >&2; return 2; }
  [ ! -e "$OUTDIR/$1.png" ] || { echo 'duplicate shot' >&2; return 2; }
  import -window "$WID" "$OUTDIR/$1.png"
  python3 "$PROOF_TOOL" record --lane "$LANE" --manifest "$MANIFEST" --image "$OUTDIR/$1.png" \
    --exe "$EXE" --window "$WID" --pid "$APID" --source-commit "$SOURCE" --fixture "$2" --size "$SIZE"
}
if [ "$REAL_PAGES" = 1 ]; then
  [ ${#SHELL_ARGS[@]} = 0 ] || { echo 'real-pages does not accept shell args' >&2; exit 2; }
  if [ "$BASELINE" = auto ]; then BASELINE=crates/amazing-shell-gpui/evidence/real-pages; fi
  while IFS=$'\t' read -r slug page category title; do
    url="file://$ROOT/tests/real-pages/$page"
    HAD_WINDOW=0
    if start_shell --url "$url"; then shot "$slug" "$url"; HAD_WINDOW=1; fi
    kill "$APID" 2>/dev/null || true; wait "$APID" 2>/dev/null || true; APID=
    baseline=; [ -z "$BASELINE" ] || baseline="$BASELINE/$slug.png"
    python3 scripts/real_pages_capture_compare.py record --out-dir "$OUTDIR" --slug "$slug" \
      --category "$category" --title "$title" --url "$url" --baseline "$baseline" --fuzz "$FUZZ" --had-window "$HAD_WINDOW"
  done < <(python3 - "$ONLY" "$LIMIT" <<'PY'
import json, sys
pages = json.load(open('tests/real-pages/manifest.json'))['pages']
selected = [p for p in pages if not sys.argv[1] or p['page'].rsplit('/',1)[-1].rsplit('.',1)[0] == sys.argv[1]]
limit = int(sys.argv[2])
for p in selected[:limit or None]:
    print('\t'.join([p['page'].rsplit('/',1)[-1].rsplit('.',1)[0], p['page'], p.get('category',''), p.get('title','').replace('\t',' ')]))
PY
)
  set +e
  python3 scripts/real_pages_capture_compare.py finalize --out-dir "$OUTDIR" --lane "$LANE" --captured-on "$(date -u +%F)" --fuzz "$FUZZ"
  STATUS=$?
  set -e
  echo "comparison report remains on Boat: $OUTDIR/report.json (exit $STATUS)"
else
  start_shell "${SHELL_ARGS[@]}"
  FIXTURE=$(python3 - "${SHELL_ARGS[@]}" <<'PY'
import json,sys
print(json.dumps(sys.argv[1:]))
PY
)
  if [ "$SCROLL" -gt 0 ] || [ -n "$ACTIONS" ]; then
    xdotool mousemove --sync "$((${SIZE%x*}/2))" "$((${SIZE#*x}/2))"; xdotool click 1
  fi
  if [ "$SCROLL" -gt 0 ]; then xdotool click --repeat "$SCROLL" --delay 80 5; sleep 1; fi
  if [ -n "$ACTIONS" ]; then
    while IFS=' ' read -r verb rest || [ -n "${verb:-}" ]; do
      case "$verb" in
        ''|'#'*) continue;;
        key) xdotool key "$rest";;
        type) xdotool type --delay 30 -- "$rest";;
        wait) sleep "$rest";;
        shot) shot "$NAME-$rest" "$FIXTURE actions=$ACTIONS shot=$rest";;
        *) echo "unknown action verb: $verb" >&2; exit 2;;
      esac
    done < "$ACTIONS"
  else shot "$NAME" "$FIXTURE scroll=$SCROLL"; fi
  STATUS=0
fi
python3 "$PROOF_TOOL" validate "$MANIFEST" "$LANE" >/dev/null
exit "$STATUS"
)
REMOTE
)"
STATUS=0
bash "$REPO/scripts/linux-run.sh" "$LANE" --ref "$SOURCE" -- "$ASSIGN
HELPER='$HELPER'
$REMOTE_BODY" || STATUS=$?
if [ "$REAL_PAGES" = 1 ]; then SUBDIR=real-pages; else SUBDIR=captures; fi
# A comparator mismatch may still have valid actual-window artifacts; fetch
# those while preserving its real exit. Build/capture failures fail transfer.
FETCH_STATUS=0
bash "$REPO/scripts/lane-fetch-window.sh" "$LANE" "$MAIN/.agents/$LANE/$SUBDIR/$NAME" \
  "amazing/out/$LANE/$SUBDIR/$NAME/$NAME.window.manifest.json" || FETCH_STATUS=$?
[ "$STATUS" = 0 ] || exit "$STATUS"
exit "$FETCH_STATUS"
