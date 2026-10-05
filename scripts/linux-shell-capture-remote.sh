#!/usr/bin/env bash
# Runs inside supported linux-run; configuration is supplied by the caller.
capture_owned_window() {
set -euo pipefail
ROOT="$PWD"
PROOF_TOOL=$(mktemp /tmp/window-proof.XXXXXX.py)
printf '%s' "$HELPER" | base64 -d > "$PROOF_TOOL"
APID= XPID=
stop_owned_child() {
  local pid="$1"
  [ -n "$pid" ] || return 0
  kill "$pid" 2>/dev/null || true
  for _ in {1..50}; do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
  done
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}
cleanup_capture() {
  local status=$?
  trap - EXIT
  # Shell loses its X/AT-SPI peers only after its own exit has been reaped.
  stop_owned_child "${APID:-}"
  stop_owned_child "${XPID:-}"
  rm -f "$PROOF_TOOL"
  exit "$status"
}
trap cleanup_capture EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
cd crates/amazing-shell-gpui
# A storage-managed caller supplies its admitted native target. Ordinary
# captures keep the existing dedicated /tmp target.
export CARGO_TARGET_DIR="${CAPTURE_CARGO_TARGET_DIR:-/tmp/amazing-shell-target-$LANE}" CARGO_INCREMENTAL=0
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
WINDOW_SIZE="$SIZE"
DISP=:$((90 + $$ % 9))
Xvfb "$DISP" -screen 0 "${SIZE}x24" -nolisten tcp >/dev/null 2>&1 & XPID=$!
sleep 1
export DISPLAY="$DISP" WAYLAND_DISPLAY=
if [ -n "$DEVICE_SCALE" ]; then export GPUI_X11_SCALE_FACTOR="$DEVICE_SCALE"; fi
export VK_ICD_FILENAMES="$(find /usr/share/vulkan/icd.d -maxdepth 1 -name 'lvp_icd*.json' | head -1)"
start_shell() {
  "$EXE" "$@" --size "$SIZE" > "$OUTDIR/shell.log" 2>&1 & APID=$!
  sleep "$WAIT"
  WID=$(xdotool search --pid "$APID" 2>/dev/null | tail -1) || true
  [ -n "$WID" ] || { echo "no actual Amazing window; raw log remains $OUTDIR/shell.log" >&2; return 5; }
  # Configure the genuine X window to the requested product viewport, then
  # place it wholly on this dedicated display. ImageMagick clips off-screen
  # window pixels, so a centered/outside window cannot certify a full shot.
  xdotool windowsize --sync "$WID" "${SIZE%x*}" "${SIZE#*x}"
  xdotool windowmove --sync "$WID" 0 0
}
shot() {
  NATIVE_OWNER=(); [[ "$OUT" != /tmp/* ]] || NATIVE_OWNER=(--native-output-root "$OUT")
  METRICS=(); [ -z "$DEVICE_SCALE" ] || METRICS=(--engine-log "$OUTDIR/shell.log" --device-scale "$DEVICE_SCALE")
  [[ "$1" =~ ^[A-Za-z0-9_-]+$ ]] || { echo 'invalid shot name' >&2; return 2; }
  [ ! -e "$OUTDIR/$1.png" ] || { echo 'duplicate shot' >&2; return 2; }
  python3 "$PROOF_TOOL" capture --lane "$LANE" --manifest "$MANIFEST" --image "$OUTDIR/$1.png" \
    --exe "$EXE" --window "$WID" --pid "$APID" --source-commit "$SOURCE" --fixture "$2" --size "$SIZE" "${METRICS[@]}" "${NATIVE_OWNER[@]}"
}
if [ "$REAL_PAGES" = 1 ]; then
  [ ${#SHELL_ARGS[@]} = 0 ] || { echo 'real-pages does not accept shell args' >&2; exit 2; }
  if [ "$BASELINE" = auto ]; then BASELINE=crates/amazing-shell-gpui/evidence/real-pages; fi
  while IFS=$'\t' read -r slug page category title; do
    url="file://$ROOT/tests/real-pages/$page"
    HAD_WINDOW=0
    if start_shell --url "$url"; then shot "$slug" "$url"; HAD_WINDOW=1; fi
    stop_owned_child "$APID"; APID=
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
        resize)
          [[ "$rest" =~ ^[1-9][0-9]*x[1-9][0-9]*$ ]] || { echo 'invalid resize action' >&2; exit 2; }
          [ "${rest%x*}" -le "${SIZE%x*}" ] && [ "${rest#*x}" -le "${SIZE#*x}" ] || { echo 'resize exceeds proof display' >&2; exit 2; }
          xdotool windowsize --sync "$WID" "${rest%x*}" "${rest#*x}"
          WINDOW_SIZE="$rest";;
        click)
          [[ "$rest" =~ ^[0-9]+[[:space:]][0-9]+$ ]] || { echo 'invalid click coordinates' >&2; exit 2; }
          read -r x y <<< "$rest"
          [ "$x" -lt "${WINDOW_SIZE%x*}" ] && [ "$y" -lt "${WINDOW_SIZE#*x}" ] || { echo 'click lies outside owned window' >&2; exit 2; }
          xdotool mousemove --sync --window "$WID" "$x" "$y"; xdotool click 1;;
        shot) shot "$NAME-$rest" "$FIXTURE actions=$ACTIONS shot=$rest";;
        *) echo "unknown action verb: $verb" >&2; exit 2;;
      esac
    done < "$ACTIONS"
  else shot "$NAME" "$FIXTURE scroll=$SCROLL"; fi
  STATUS=0
fi
python3 "$PROOF_TOOL" validate "$MANIFEST" "$LANE" >/dev/null
exit "$STATUS"
}

# Serialize the actual capture configuration (including argv) without lossy
# shell interpolation; the runner owns a fresh bus until all capture cleanup.
SESSION_BODY=$(mktemp /tmp/amazing-window-session.XXXXXX.sh)
SESSION_TOOL=$(mktemp /tmp/amazing-session-custodian.XXXXXX.py)
trap 'rm -f "$SESSION_BODY" "$SESSION_TOOL"' EXIT
printf '%s' "$SESSION_HELPER" | base64 -d > "$SESSION_TOOL"
{
  declare -p LANE SOURCE SIZE WAIT NAME PROFILE SCROLL REAL_PAGES LIMIT ONLY BASELINE FUZZ ACTIONS HELPER SHELL_ARGS
  for optional in DEVICE_SCALE CAPTURE_CARGO_TARGET_DIR; do
    if declare -p "$optional" >/dev/null 2>&1; then declare -p "$optional"; fi
  done
  declare -f capture_owned_window
  printf '%s\n' 'capture_owned_window'
} > "$SESSION_BODY"
if [ "$REAL_PAGES" = 1 ]; then SESSION_SUBDIR=real-pages; else SESSION_SUBDIR=captures; fi
SOURCE="$SOURCE" python3 "$SESSION_TOOL" --receipt "$OUT/$SESSION_SUBDIR/$NAME.session.manifest.json" -- bash "$SESSION_BODY"
