#!/usr/bin/env bash
# Native-only. Caller enters through linux-capture-session.py's owned D-Bus session.
# Usage: bash navigation_windows.sh SOURCE EXE OUT [original|after] [LANE]
set -euo pipefail
SOURCE=${1:?source} EXE=${2:?native exe} SUITE_OUT=${3:?output} MODE=${4:-after} PROOF_LANE=${5:-product-responsive-navigation}
ROOT=$PWD
mkdir -p "$SUITE_OUT"
XPID= APID= FPID= NAVIGATION_XDG=
stop_child() {
  local pid=$1
  [ -n "$pid" ] || return 0
  kill "$pid" 2>/dev/null || true
  for _ in {1..50}; do kill -0 "$pid" 2>/dev/null || break; sleep .1; done
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}
cleanup() {
  local status=$?
  trap - EXIT
  stop_child "${APID:-}";stop_child "${FPID:-}";stop_child "${XPID:-}"
  [ -z "$NAVIGATION_XDG" ] || rmdir "$NAVIGATION_XDG" 2>/dev/null || true
  exit "$status"
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
DISPLAY=:$((110 + $$ % 40)); export DISPLAY WAYLAND_DISPLAY=
Xvfb "$DISPLAY" -screen 0 1280x800x24 -nolisten tcp > "$SUITE_OUT/xvfb.log" 2>&1 & XPID=$!
sleep 1
export VK_ICD_FILENAMES=$(find /usr/share/vulkan/icd.d -maxdepth 1 -name 'lvp_icd*.json' | head -1)
# Make activated AT-SPI/portal clients see this owned display and writable runtime.
# No accessibility backend is disabled by the fixture.
NAVIGATION_XDG=$(mktemp -d /tmp/amazing-navigation-xdg.XXXXXX)
chmod 700 "$NAVIGATION_XDG"
export XDG_RUNTIME_DIR="$NAVIGATION_XDG"
dbus-update-activation-environment DISPLAY WAYLAND_DISPLAY XDG_RUNTIME_DIR
export AMAZING_NAVIGATION_TRACE=1
SUITE_STATUS=0
for SCALE in 1 2; do
  export GPUI_X11_SCALE_FACTOR=$SCALE
  NAVIGATION_GROUPS=(product-smoke viewport-resize)
  if [ "$MODE" = after ]; then NAVIGATION_GROUPS+=(navigation font-display continuation); fi
  for GROUP in "${NAVIGATION_GROUPS[@]}"; do
    GROUP_OUT="$SUITE_OUT/$GROUP-scale$SCALE"
    mkdir "$GROUP_OUT"
    export LOCALAPPDATA="$GROUP_OUT/profile"
    URL=http://127.0.0.1:8087
    if [ "$GROUP" = navigation ] || [ "$GROUP" = font-display ] || [ "$GROUP" = continuation ]; then
      python3 crates/amazing-shell-gpui/smoke/navigation_fixture.py --output "$GROUP_OUT/fixture.jsonl" --ready "$GROUP_OUT/fixture-ready.json" > "$GROUP_OUT/fixture.log" 2>&1 & FPID=$!
      for _ in {1..100}; do [ ! -e "$GROUP_OUT/fixture-ready.json" ] || break; kill -0 "$FPID"; sleep .02; done
      [ -e "$GROUP_OUT/fixture-ready.json" ]
    else
      : > "$GROUP_OUT/fixture.jsonl"
    fi
    "$EXE" --size 1280x800 > "$GROUP_OUT/shell.log" 2>&1 & APID=$!
    WID=
    for _ in {1..2400}; do
      kill -0 "$APID"
      WID=$(xdotool search --onlyvisible --pid "$APID" 2>/dev/null | tail -1) || true
      [ -z "$WID" ] || break
      sleep .025
    done
    [ -n "$WID" ] || { echo "no mapped Amazing window; native log: $GROUP_OUT/shell.log" >&2; exit 5; }
    # The XID can exist before initial engine construction and mapping complete.
    # Require the real measured frame before input/capture, including baseline mode.
    python3 - "$GROUP_OUT/shell.log" "$APID" <<'FIRST_FRAME'
import os,pathlib,sys,time
log=pathlib.Path(sys.argv[1]);pid=int(sys.argv[2]);deadline=time.monotonic()+60
while time.monotonic()<deadline:
    os.kill(pid,0)
    records=[line for line in log.read_text(errors='strict').splitlines() if line.startswith(('AMAZING_VIEWPORT ','AMAZING_VIEWPORT_FAILURE '))]
    if records and records[-1].startswith('AMAZING_VIEWPORT '):break
    time.sleep(.025)
else:raise RuntimeError('mapped Amazing window never produced a measured engine frame')
FIRST_FRAME
    xdotool windowsize --sync "$WID" 1280 800
    xdotool windowmove --sync "$WID" 0 0
    xdotool windowfocus --sync "$WID"
    BASELINE_FLAG=()
    if [ "$MODE" = original ]; then BASELINE_FLAG=(--baseline); fi
    ACTION_STATUS=0
    python3 crates/amazing-shell-gpui/smoke/navigation_actions.py "${BASELINE_FLAG[@]}" --out "$GROUP_OUT/actions" \
      --log "$GROUP_OUT/shell.log" --fixture-log "$GROUP_OUT/fixture.jsonl" --proof scripts/window-proof.py \
      --lane "$PROOF_LANE" --native-output-root "$SUITE_OUT" --exe "$EXE" --window "$WID" --pid "$APID" --source "$SOURCE" \
      --scale "$SCALE" --url "$URL" --actions "crates/amazing-shell-gpui/smoke/$GROUP.actions" || ACTION_STATUS=$?
    python3 - "$GROUP_OUT" "$SOURCE" "$GROUP" "$SCALE" "$ACTION_STATUS" <<'GROUP_RESULT'
import json,pathlib,sys
pathlib.Path(sys.argv[1],"group-exit.json").write_text(json.dumps({"source":sys.argv[2],"group":sys.argv[3],"density":int(sys.argv[4]),"actual_action_exit":int(sys.argv[5])},separators=(",",":"))+"\n")
GROUP_RESULT
    if [ "$ACTION_STATUS" -ne 0 ]; then SUITE_STATUS=1; fi
    stop_child "$APID";APID=
    stop_child "${FPID:-}";FPID=
  done
done
exit "$SUITE_STATUS"
