#!/usr/bin/env bash
# Build the GPUI shell on a lane's Boat sandbox, run it on a virtual X display,
# and copy a PNG of its window back to the Mac.
#
# Boat sandboxes are Ubuntu 24.04 with a GNOME desktop (open it with
# `boat desktop <id>`) but no GPU, so the shell renders through Mesa's software
# Vulkan (lavapipe). Captures use their own Xvfb display so they never depend on
# a desktop session being open. This is the Linux counterpart of
# scripts/capture-gpui-shell.ps1 on win-001.
#
# Usage: scripts/linux-shell-capture.sh <lane> [--ref REF] [--size 1280x800]
#          [--wait 8] [--name capture] [--release] [--scroll N] -- [shell args...]
#   Writes .agents/<lane>/captures/<name>.png and <name>.json (executable
#   sha256, commit, shell args, display size, capture time) on the Mac.
#   Shell args go to amazing-shell unchanged (for example `--url file:///...`
#   once the shell accepts it).
#   --scroll N sends N scroll-wheel-down clicks (xdotool button 5) at the
#   window center after --wait and before capturing, for scrolled-page
#   regression shots.

set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
LANE="${1:?usage: linux-shell-capture.sh <lane> [--ref REF] [--size WxH] [--wait S] [--name N] [--release] [--scroll N] [-- shell args]}"
shift
REF_ARGS=() SIZE=1280x800 WAIT=8 NAME="capture-$(date -u +%Y%m%dT%H%M%SZ)" PROFILE=debug SCROLL=0
while [ $# -gt 0 ]; do
  case "$1" in
    --ref) REF_ARGS=(--ref "$2"); shift 2 ;;
    --size) SIZE="$2"; shift 2 ;;
    --wait) WAIT="$2"; shift 2 ;;
    --name) NAME="$2"; shift 2 ;;
    --release) PROFILE=release; shift ;;
    --scroll) SCROLL="$2"; shift 2 ;;
    --) shift; break ;;
    *) echo "linux-shell-capture: unknown option $1" >&2; exit 2 ;;
  esac
done
SHELL_ARGS="$*"
[ "$PROFILE" = release ] && BUILD_FLAG=--release || BUILD_FLAG=
SIZE_W="${SIZE%x*}"; SIZE_H="${SIZE#*x}"
CENTER_X=$(( SIZE_W / 2 )); CENTER_Y=$(( SIZE_H / 2 ))

# The shell has its own lockfile and GPUI dependency graph, so it builds into
# its own target dir next to the lane's.
"$REPO/scripts/linux-run.sh" "$LANE" ${REF_ARGS[@]+"${REF_ARGS[@]}"} -- "
set -u
cd crates/amazing-shell-gpui || exit 3
export CARGO_TARGET_DIR=\$HOME/amazing/target-shell-$LANE
cargo build --locked $BUILD_FLAG --bin amazing-shell 2>&1 | grep -E '^(error|warning: unused)' -A6 | head -40
EXE=\$CARGO_TARGET_DIR/$PROFILE/amazing-shell
[ -x \"\$EXE\" ] || { echo 'linux-shell-capture: build failed' >&2; exit 4; }
mkdir -p \$OUT/captures
DISP=:\$(( 90 + \$\$ % 9 ))
Xvfb \$DISP -screen 0 ${SIZE}x24 -nolisten tcp >/dev/null 2>&1 & XPID=\$!
trap 'kill \$APID \$XPID 2>/dev/null' EXIT
sleep 1
DISPLAY=\$DISP WAYLAND_DISPLAY= VK_ICD_FILENAMES=\$(ls /usr/share/vulkan/icd.d/lvp_icd*.json 2>/dev/null | head -1) \
  \"\$EXE\" $SHELL_ARGS >\$OUT/captures/$NAME.log 2>&1 & APID=\$!
sleep $WAIT
WID=\$(DISPLAY=\$DISP xdotool search --pid \$APID 2>/dev/null | tail -1)
if [ -n \"\$WID\" ] && [ $SCROLL -gt 0 ]; then
  # xdotool's --window form sends synthetic XSendEvent input, which GPUI's
  # XInput2 raw-event handling never sees. Move the real (only) pointer over
  # the window's content and click with XTest instead, which raw input does
  # see. This Xvfb display exists only for this one, undecorated,
  # display-filling window, so a display-center point always lands on it.
  DISPLAY=\$DISP xdotool mousemove --sync $CENTER_X $CENTER_Y
  DISPLAY=\$DISP xdotool click --repeat $SCROLL --delay 80 5
  sleep 1
fi
if [ -n \"\$WID\" ]; then DISPLAY=\$DISP import -window \$WID \$OUT/captures/$NAME.png; else
  echo 'linux-shell-capture: no shell window; capturing the whole display' >&2
  DISPLAY=\$DISP import -window root \$OUT/captures/$NAME.png; fi
kill -0 \$APID 2>/dev/null || { echo 'linux-shell-capture: shell exited early; log:' >&2; tail -20 \$OUT/captures/$NAME.log >&2; }
printf '{\"exe\":\"%s\",\"sha256\":\"%s\",\"commit_tree\":\"%s\",\"args\":\"%s\",\"display\":\"$SIZE\",\"window\":\"%s\",\"captured\":\"%s\"}\n' \
  \"\$EXE\" \"\$(sha256sum \$EXE | cut -d' ' -f1)\" \"\$PWD\" \"$SHELL_ARGS\" \"\$WID\" \"\$(date -u +%FT%TZ)\" > \$OUT/captures/$NAME.json
ls -la \$OUT/captures/$NAME.png
" || exit $?

read -r H P _ < <("$MAIN/scripts/boat-lane.py" ensure "$LANE")
DEST="$MAIN/.agents/$LANE/captures"; mkdir -p "$DEST"
for f in png json log; do
  scp -q -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
    -i "$HOME/.ssh/ascii_box_ed25519" -P "$P" "$H:amazing/out/$LANE/captures/$NAME.$f" "$DEST/" 2>/dev/null
done
echo "linux-shell-capture: $DEST/$NAME.png"
