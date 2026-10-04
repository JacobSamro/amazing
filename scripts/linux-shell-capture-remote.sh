#!/usr/bin/env bash
# Runs inside supported linux-run; configuration is supplied by the caller.
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
  # Configure the genuine X window to the requested product viewport, then
  # place it wholly on this dedicated display. ImageMagick clips off-screen
  # window pixels, so a centered/outside window cannot certify a full shot.
  xdotool windowsize --sync "$WID" "${SIZE%x*}" "${SIZE#*x}"
  xdotool windowmove --sync "$WID" 0 0
}
shot() {
  [[ "$1" =~ ^[A-Za-z0-9_-]+$ ]] || { echo 'invalid shot name' >&2; return 2; }
  [ ! -e "$OUTDIR/$1.png" ] || { echo 'duplicate shot' >&2; return 2; }
  python3 "$PROOF_TOOL" capture --lane "$LANE" --manifest "$MANIFEST" --image "$OUTDIR/$1.png" \
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
