#!/usr/bin/env bash
# Linux counterpart of scripts/run-render-wpt.ps1 + scripts/inventory-render-wpt.ps1,
# for lanes that want a WPT reftest baseline without a win-001 render sweep.
#
# Builds the headless `amazing` binary (NOT the GPUI shell) on a lane's Boat
# sandbox, discovers self-contained static reftests under the given WPT
# subdirectory against the pinned checkout at ~/corpora/wpt, renders each
# test/reference pair with `amazing render-wpt`, pixel-compares them with the
# test's <meta name="fuzzy"> tolerance, and writes one report.json (schema
# documented in scripts/wpt_render_compare.py) classifying every test as
# pass/mismatch/error/timeout/unsupported/crash.
#
# Usage: scripts/linux-render-wpt.sh <lane> <wpt-subdir> [--width W] [--height H]
#          [--timeout S] [--limit N] [--release]
#   <wpt-subdir> is relative to the WPT checkout root, e.g. css/css-backgrounds.
#   Writes .agents/<lane>/wpt-render/<slug>/report.json on the Mac. Full
#   per-test artifacts (actual/reference/diff PPM, stdout/stderr logs) stay on
#   the sandbox under $OUT/wpt-render/<slug>/<run>/ — durable evidence is the
#   committed report.json + the counts in your lane's report, not the PPMs.
#
# Example: scripts/linux-render-wpt.sh css css/css-backgrounds
#          scripts/linux-render-wpt.sh text css/css-fonts --limit 30

set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"

LANE="${1:?usage: linux-render-wpt.sh <lane> <wpt-subdir> [--width W] [--height H] [--timeout S] [--limit N] [--release]}"
SUBDIR="${2:?usage: linux-render-wpt.sh <lane> <wpt-subdir> [--width W] [--height H] [--timeout S] [--limit N] [--release]}"
shift 2

WIDTH=800 HEIGHT=600 TIMEOUT=20 LIMIT=0 PROFILE=debug
while [ $# -gt 0 ]; do
  case "$1" in
    --width) WIDTH="$2"; shift 2 ;;
    --height) HEIGHT="$2"; shift 2 ;;
    --timeout) TIMEOUT="$2"; shift 2 ;;
    --limit) LIMIT="$2"; shift 2 ;;
    --release) PROFILE=release; shift ;;
    *) echo "linux-render-wpt: unknown option $1" >&2; exit 2 ;;
  esac
done
[ "$PROFILE" = release ] && BUILD_FLAG=--release || BUILD_FLAG=
SLUG="${SUBDIR//\//-}"

"$REPO/scripts/linux-run.sh" "$LANE" -- "
set -u
bash scripts/ensure-wpt.sh >&2
cargo build -p amazing --locked $BUILD_FLAG 2>&1 | grep -E '^(error|warning: unused)' -A6 | head -60
EXE=\$CARGO_TARGET_DIR/$PROFILE/amazing
[ -x \"\$EXE\" ] || { echo 'linux-render-wpt: build failed' >&2; exit 4; }
mkdir -p \$OUT/wpt-render/$SLUG
python3 scripts/wpt_render_compare.py \
  --wpt-root \$HOME/corpora/wpt \
  --subdir '$SUBDIR' \
  --exe \"\$EXE\" \
  --out \$OUT/wpt-render/$SLUG \
  --width $WIDTH --height $HEIGHT --timeout $TIMEOUT --limit $LIMIT
" || exit $?

read -r H P _ < <("$REPO/scripts/boat-lane.py" ensure "$LANE")
DEST="$MAIN/.agents/$LANE/wpt-render/$SLUG"; mkdir -p "$DEST"
scp -q -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
  -i "$HOME/.ssh/ascii_box_ed25519" -P "$P" "$H:amazing/out/$LANE/wpt-render/$SLUG/report.json" "$DEST/" \
  || { echo "linux-render-wpt: could not copy report.json back (venus host? copy manually with scp -o ControlPath=none)" >&2; exit 0; }
echo "linux-render-wpt: $DEST/report.json"
python3 - "$DEST/report.json" <<'PYEOF'
import json, sys
report = json.load(open(sys.argv[1]))
counts = report["counts"]
print("Counts: " + " ".join(f"{k}={v}" for k, v in counts.items()))
PYEOF
