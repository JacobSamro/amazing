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
#
# --real-pages (SHELL-L6): batch mode. Instead of one capture, opens every
# page in tests/real-pages/manifest.json (or a --limit/--only subset) through
# --url file://, one shell process at a time on one Xvfb display, and
# pixel-diffs each capture against a committed baseline PNG with
# scripts/real_pages_capture_compare.py (ImageMagick `compare -metric AE`).
#   scripts/linux-shell-capture.sh <lane> --real-pages [--ref REF]
#     [--size WxH] [--wait S] [--limit N] [--only SLUG]
#     [--baseline auto|<dir>] [--fuzz PCT] [--release]
#   --baseline auto (the default when --real-pages is given) resolves each
#   page's baseline against the checked-in
#   crates/amazing-shell-gpui/evidence/real-pages/<slug>.png; a <dir> rehomes
#   the lookup, and omitting a baseline entirely captures/reports without
#   gating. A page with no baseline yet is classified "new", not a failure --
#   promote its capture into the evidence directory to establish one.
#   --fuzz (default 1) is passed straight to ImageMagick's -fuzz percentage;
#   validated at 1% with zero false positives across all 50 gated fixtures on
#   a same-build second pass.
#   Two fixtures (news-live, app-calendar) render Date.now() and are always
#   classified "excluded", never gated -- see EXCLUDE_SLUGS in
#   scripts/real_pages_capture_compare.py.
#   Writes .agents/<lane>/real-pages/{report.json,pages.jsonl,<slug>.png,
#   <slug>-diff.png (mismatches only)} on the Mac. Exits 2 if any page is
#   mismatch/crash/error (excluded and new do not fail the run).

set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
USAGE="usage: linux-shell-capture.sh <lane> [--ref REF] [--size WxH] [--wait S] [--name N] [--release] [--scroll N] [-- shell args] | linux-shell-capture.sh <lane> --real-pages [--ref REF] [--size WxH] [--wait S] [--limit N] [--only SLUG] [--baseline auto|<dir>] [--fuzz PCT] [--release]"
LANE="${1:?$USAGE}"
shift
REF_ARGS=() SIZE=1280x800 WAIT=8 NAME="capture-$(date -u +%Y%m%dT%H%M%SZ)" PROFILE=debug SCROLL=0
REAL_PAGES=0 LIMIT=0 ONLY="" BASELINE="" FUZZ=1
while [ $# -gt 0 ]; do
  case "$1" in
    --ref) REF_ARGS=(--ref "$2"); shift 2 ;;
    --size) SIZE="$2"; shift 2 ;;
    --wait) WAIT="$2"; shift 2 ;;
    --name) NAME="$2"; shift 2 ;;
    --release) PROFILE=release; shift ;;
    --scroll) SCROLL="$2"; shift 2 ;;
    --real-pages) REAL_PAGES=1; BASELINE=auto; shift ;;
    --limit) LIMIT="$2"; shift 2 ;;
    --only) ONLY="$2"; shift 2 ;;
    --baseline) BASELINE="$2"; shift 2 ;;
    --fuzz) FUZZ="$2"; shift 2 ;;
    --) shift; break ;;
    *) echo "linux-shell-capture: unknown option $1" >&2; exit 2 ;;
  esac
done
SHELL_ARGS="$*"
[ "$PROFILE" = release ] && BUILD_FLAG=--release || BUILD_FLAG=
SIZE_W="${SIZE%x*}"; SIZE_H="${SIZE#*x}"
CENTER_X=$(( SIZE_W / 2 )); CENTER_Y=$(( SIZE_H / 2 ))

if [ "$REAL_PAGES" = 1 ]; then
  [ -z "$SHELL_ARGS" ] || { echo "linux-shell-capture: --real-pages drives its own --url per page, no shell args" >&2; exit 2; }

  "$REPO/scripts/linux-run.sh" "$LANE" ${REF_ARGS[@]+"${REF_ARGS[@]}"} -- "
set -u
ROOT=\"\$PWD\"
cd crates/amazing-shell-gpui || exit 3
export CARGO_TARGET_DIR=\$HOME/amazing/target-shell-$LANE
cargo build --locked $BUILD_FLAG --bin amazing-shell 2>&1 | grep -E '^(error|warning: unused)' -A6 | head -40
EXE=\$CARGO_TARGET_DIR/$PROFILE/amazing-shell
[ -x \"\$EXE\" ] || { echo 'linux-shell-capture: build failed' >&2; exit 4; }
cd \"\$ROOT\"
OUTDIR=\$OUT/real-pages
mkdir -p \"\$OUTDIR\"
rm -f \"\$OUTDIR/pages.jsonl\"
if [ '$BASELINE' = auto ]; then BASELINE_ROOT=crates/amazing-shell-gpui/evidence/real-pages
elif [ -n '$BASELINE' ] && [ -d '$BASELINE' ]; then BASELINE_ROOT='$BASELINE'
else BASELINE_ROOT=''; fi
DISP=:\$(( 90 + \$\$ % 9 ))
Xvfb \$DISP -screen 0 ${SIZE}x24 -nolisten tcp >/dev/null 2>&1 & XPID=\$!
trap 'kill \$APID \$XPID 2>/dev/null' EXIT
sleep 1
while IFS=\$'\t' read -r slug relpage category title; do
  [ -n \"\$slug\" ] || continue
  url=\"file://\$ROOT/tests/real-pages/\$relpage\"
  DISPLAY=\$DISP WAYLAND_DISPLAY= VK_ICD_FILENAMES=\$(ls /usr/share/vulkan/icd.d/lvp_icd*.json 2>/dev/null | head -1) \
    \"\$EXE\" --url \"\$url\" --size $SIZE >\"\$OUTDIR/\$slug.log\" 2>&1 & APID=\$!
  sleep $WAIT
  WID=\$(DISPLAY=\$DISP xdotool search --pid \$APID 2>/dev/null | tail -1)
  HAD_WINDOW=0
  if [ -n \"\$WID\" ]; then
    DISPLAY=\$DISP import -window \$WID \"\$OUTDIR/\$slug.png\" && HAD_WINDOW=1
  fi
  kill -9 \$APID 2>/dev/null; wait \$APID 2>/dev/null
  if [ -n \"\$BASELINE_ROOT\" ]; then baseline_file=\"\$BASELINE_ROOT/\$slug.png\"; else baseline_file=''; fi
  python3 scripts/real_pages_capture_compare.py record --out-dir \"\$OUTDIR\" --slug \"\$slug\" \
    --category \"\$category\" --title \"\$title\" --url \"\$url\" --baseline \"\$baseline_file\" \
    --fuzz $FUZZ --had-window \$HAD_WINDOW
done < <(python3 -c \"
import json
d = json.load(open('tests/real-pages/manifest.json'))
only = '$ONLY'
limit = $LIMIT
n = 0
for p in d['pages']:
    slug = p['page'].rsplit('/', 1)[-1].rsplit('.', 1)[0]
    if only and slug != only:
        continue
    if limit and n >= limit:
        break
    n += 1
    print(slug + '\t' + p['page'] + '\t' + p.get('category', '') + '\t' + p.get('title', '').replace('\t', ' '))
\")
python3 scripts/real_pages_capture_compare.py finalize --out-dir \"\$OUTDIR\" --lane $LANE --captured-on \"\$(date -u +%F)\" --fuzz $FUZZ
FINALIZE_STATUS=\$?
ls \"\$OUTDIR\" | wc -l
exit \$FINALIZE_STATUS
" || REMOTE_STATUS=$?
  REMOTE_STATUS="${REMOTE_STATUS:-0}"

  read -r H P _ < <("$MAIN/scripts/boat-lane.py" ensure "$LANE")
  DEST="$MAIN/.agents/$LANE/real-pages"; mkdir -p "$DEST"
  scp -q -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
    -i "$HOME/.ssh/ascii_box_ed25519" -P "$P" "$H:amazing/out/$LANE/real-pages/*" "$DEST/" 2>/dev/null \
    || echo "linux-shell-capture: could not copy real-pages evidence back (venus host? copy manually with scp -o ControlPath=none)" >&2
  echo "linux-shell-capture: $DEST/report.json"
  [ -f "$DEST/report.json" ] && python3 -c "
import json
r = json.load(open('$DEST/report.json'))
print('Counts:', ' '.join(f'{k}={v}' for k, v in sorted(r['counts'].items())))
"
  exit "$REMOTE_STATUS"
fi

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
