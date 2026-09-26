#!/usr/bin/env bash
# Linux counterpart of scripts/run-render-wpt.ps1 + scripts/inventory-render-wpt.ps1,
# for lanes that want a WPT reftest baseline without a win-001 render sweep.
#
# Builds the headless `amazing` binary (NOT the GPUI shell) on a lane's Boat
# sandbox, discovers self-contained static reftests under one or more WPT
# subdirectories against the pinned checkout at ~/corpora/wpt, renders each
# test/reference pair with `amazing render-wpt`, pixel-compares them with the
# test's <meta name="fuzzy"> tolerance, and writes one report.json per subdir
# (schema documented in scripts/wpt_render_compare.py) classifying every test
# as pass/mismatch/error/timeout/unsupported/crash, plus a clusters.json
# grouping mismatches by what visibly differs (scripts/wpt_render_cluster.py).
#
# Usage: scripts/linux-render-wpt.sh <lane> <wpt-subdir> [<wpt-subdir> ...]
#          [--width W] [--height H] [--timeout S] [--limit N] [--jobs N]
#          [--baseline PATH|DIR|auto] [--compact] [--release]
#   Each <wpt-subdir> is relative to the WPT checkout root, e.g.
#   css/css-backgrounds. Multiple subdirs in one call share a single build +
#   linux-run.sh round trip -- batch a lane's whole sweep into as few calls
#   as practical; each is still a separate report.
#   Writes .agents/<lane>/wpt-render/<slug>/{report,clusters}.json on the Mac
#   (compact.json instead of report.json when --compact is given).
#   Full per-test artifacts (actual/reference/diff PPM, stdout/stderr logs)
#   stay on the sandbox under $OUT/wpt-render/<slug>/<run>/ -- durable
#   evidence is the committed report.json/compact.json + clusters.json, not
#   the PPMs, and this script deletes them right after clusters.json exists.
#   --jobs defaults to the sandbox's own $JOBS (nproc-derived); pass 1 to
#   force serial rendering.
#
#   --baseline turns the run into a zero-loss gate (SHELL-L5): prints
#   gained/lost/changed/unchanged/new/removed test-id counts per subdir and
#   exits non-zero if any subdir lost a previously-passing test.
#     --baseline auto   per subdir, use the checked-in
#                        crates/amazing-shell-gpui/evidence/wpt-render/<slug>/
#                        {compact,report}.json already in the synced worktree
#                        (whichever exists; skips the compare if neither does)
#     --baseline DIR     same lookup, rooted at DIR instead
#     --baseline FILE    that exact file, for every subdir in this call
#                        (only makes sense with a single <wpt-subdir>)
#   --compact writes/commits compact.json (id -> classification, no per-test
#   detail) instead of the full report.json, so evidence stays small when a
#   category is gated on every run rather than baselined once.
#
# Example: scripts/linux-render-wpt.sh css css/css-backgrounds css/css-color css/css-values
#          scripts/linux-render-wpt.sh text css/css-fonts --limit 30 --jobs 8
#          scripts/linux-render-wpt.sh css css/css-backgrounds --baseline auto --compact

set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"

USAGE="usage: linux-render-wpt.sh <lane> <wpt-subdir> [<wpt-subdir> ...] [--width W] [--height H] [--timeout S] [--limit N] [--jobs N] [--baseline PATH|DIR|auto] [--compact] [--release]"
LANE="${1:?$USAGE}"
shift
[ $# -gt 0 ] || { echo "linux-render-wpt: at least one <wpt-subdir> is required" >&2; exit 2; }

SUBDIRS=()
while [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; do
  SUBDIRS+=("$1")
  shift
done
[ "${#SUBDIRS[@]}" -gt 0 ] || { echo "linux-render-wpt: at least one <wpt-subdir> is required" >&2; exit 2; }

WIDTH=800 HEIGHT=600 TIMEOUT=20 LIMIT=0 JOBS_ARG="" PROFILE=debug BASELINE="" COMPACT=0
while [ $# -gt 0 ]; do
  case "$1" in
    --width) WIDTH="$2"; shift 2 ;;
    --height) HEIGHT="$2"; shift 2 ;;
    --timeout) TIMEOUT="$2"; shift 2 ;;
    --limit) LIMIT="$2"; shift 2 ;;
    --jobs) JOBS_ARG="$2"; shift 2 ;;
    --baseline) BASELINE="$2"; shift 2 ;;
    --compact) COMPACT=1; shift ;;
    --release) PROFILE=release; shift ;;
    *) echo "linux-render-wpt: unknown option $1" >&2; exit 2 ;;
  esac
done
[ "$PROFILE" = release ] && BUILD_FLAG=--release || BUILD_FLAG=
[ "$COMPACT" = 1 ] && ARTIFACT=compact.json || ARTIFACT=report.json

# One `for subdir` loop inside a single remote command: the build (cargo build
# -p amazing) happens once and every subdir's discovery + render + cluster
# runs against the warm target dir, instead of paying a linux-run.sh round
# trip (sync + slot queue + ssh) per category.
REMOTE_SUBDIRS=""
for subdir in "${SUBDIRS[@]}"; do
  REMOTE_SUBDIRS="$REMOTE_SUBDIRS $(printf '%q' "$subdir")"
done

"$REPO/scripts/linux-run.sh" "$LANE" -- "
set -u
bash scripts/ensure-wpt.sh >&2
cargo build -p amazing --locked $BUILD_FLAG 2>&1 | grep -E '^(error|warning: unused)' -A6 | head -60
EXE=\$CARGO_TARGET_DIR/$PROFILE/amazing
[ -x \"\$EXE\" ] || { echo 'linux-render-wpt: build failed' >&2; exit 4; }
JOBS_VALUE='$JOBS_ARG'
[ -n \"\$JOBS_VALUE\" ] || JOBS_VALUE=\"\${JOBS:-1}\"
BATCH_FAILED=0
for subdir in$REMOTE_SUBDIRS; do
  slug=\"\${subdir//\//-}\"
  mkdir -p \"\$OUT/wpt-render/\$slug\"
  echo \"=== \$subdir ===\" >&2
  BASELINE_ARGS=()
  if [ -n '$BASELINE' ]; then
    if [ '$BASELINE' = auto ]; then baseline_root=\"crates/amazing-shell-gpui/evidence/wpt-render\"
    elif [ -d '$BASELINE' ]; then baseline_root='$BASELINE'
    else baseline_root=''; fi
    if [ -n \"\$baseline_root\" ]; then
      baseline_file=\"\$baseline_root/\$slug/compact.json\"
      [ -f \"\$baseline_file\" ] || baseline_file=\"\$baseline_root/\$slug/report.json\"
    else
      baseline_file='$BASELINE'
    fi
    if [ -f \"\$baseline_file\" ]; then BASELINE_ARGS=(--baseline \"\$baseline_file\")
    else echo \"linux-render-wpt: no baseline found for \$slug at \$baseline_file, skipping compare\" >&2; fi
  fi
  python3 scripts/wpt_render_compare.py \
    --wpt-root \$HOME/corpora/wpt \
    --subdir \"\$subdir\" \
    --exe \"\$EXE\" \
    --out \"\$OUT/wpt-render/\$slug\" \
    --width $WIDTH --height $HEIGHT --timeout $TIMEOUT --limit $LIMIT --jobs \"\$JOBS_VALUE\" \
    $([ "$COMPACT" = 1 ] && echo --compact) \
    \"\${BASELINE_ARGS[@]}\"
  [ \$? -eq 0 ] || BATCH_FAILED=1
  report=\$(ls -t \"\$OUT/wpt-render/\$slug\"/run-*/report.json | head -1)
  python3 scripts/wpt_render_cluster.py \"\$report\"
  # wpt_render_compare.py already copies report.json/compact.json flat into
  # the slug directory for convenience; do the same for clusters.json so all
  # of them live next to each other for the single scp pass below.
  cp \"\$(dirname \"\$report\")/clusters.json\" \"\$OUT/wpt-render/\$slug/clusters.json\"
  # The actual/reference/diff PPMs are what filled the sandbox disk to 98%
  # full mid-sweep (a few MB per test x thousands of tests across categories)
  # and are disposable once clusters.json exists (durable evidence is
  # report.json/compact.json + clusters.json, not the PPMs -- same policy as
  # win-001's disk hygiene rule in AGENTS.md). Drop the whole run's per-test
  # PPM tree right away instead of letting it accumulate across batches.
  rm -rf \"\$(dirname \"\$report\")\"
done
exit \$BATCH_FAILED
"
REMOTE_STATUS=$?

for subdir in "${SUBDIRS[@]}"; do
  slug="${subdir//\//-}"
  DEST="$MAIN/.agents/$LANE/wpt-render/$slug"; mkdir -p "$DEST"
  "$MAIN/scripts/lane-fetch.sh" "$LANE" "$DEST" "amazing/out/$LANE/wpt-render/$slug/$ARTIFACT" \
    "amazing/out/$LANE/wpt-render/$slug/clusters.json" \
    || echo "linux-render-wpt: could not copy $slug results back" >&2
  echo "linux-render-wpt: $DEST/$ARTIFACT"
  python3 - "$DEST/$ARTIFACT" <<'PYEOF'
import json, sys
report = json.load(open(sys.argv[1]))
counts = report["counts"]
print("Counts: " + " ".join(f"{k}={v}" for k, v in counts.items()))
compare = report.get("baseline_compare")
if compare:
    print("Baseline compare: " + " ".join(f"{k}={len(v)}" for k, v in compare.items() if k != "baseline"))
    if compare.get("lost"):
        print("LOST: " + ", ".join(compare["lost"]))
PYEOF
done
exit $REMOTE_STATUS
