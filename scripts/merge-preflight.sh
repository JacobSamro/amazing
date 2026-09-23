#!/usr/bin/env bash
# Verify a lane branch is safe to merge, before the integrator reads a diff.
#
# Checks, in order:
#   1. the branch is rebased on the current integration branch
#   2. every changed path is inside the lane's owned paths or its recorded grants
#   3. any granted file outside the lane's crates is ADDITIONS ONLY, since every
#      grant issued so far was for added match arms, an added dispatch line or an
#      added entry point -- a deletion there means the grant was exceeded
#   4. the lane wrote a report and marked itself done
#
# It deliberately does not run the build. Gate numbers come from the lane's
# report and from the integrator's own authoritative gate on the merged head,
# because a gate run on the lane branch proves nothing about the merge.
#
# Usage:  scripts/merge-preflight.sh <lane> [integration-branch]

set -o pipefail

LANE="${1:?usage: merge-preflight.sh <lane> [integration-branch]}"
BASE="${2:-dev}"
BRANCH="${PREFLIGHT_BRANCH:-lane/$LANE}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

fail=0
note() { printf '  %-4s %s\n' "$1" "$2"; }

echo "preflight: $BRANCH onto $BASE"

git rev-parse --verify --quiet "$BRANCH" >/dev/null || {
  note FAIL "no such branch $BRANCH"; exit 1; }

# A lane may split a ticket across several branches. That has already caused a
# merge to ship a message claiming work that was sitting on a sibling branch,
# so surface every unmerged branch belonging to this lane rather than assuming
# lane/<lane> is the whole story.
siblings=$(git branch --format='%(refname:short)' |
  grep -E "^(lane|checkpoint)/${LANE}([-/].+)?$" | grep -v "^${BRANCH}$" || true)
for b in $siblings; do
  ahead=$(git rev-list --count "$BASE..$b" 2>/dev/null || echo 0)
  if [ "$ahead" -gt 0 ]; then
    note WARN "sibling branch $b has $ahead unmerged commit(s) -- is it part of this ticket?"
  fi
done

# 1. Rebased?
behind=$(git rev-list --count "$BRANCH..$BASE")
if [ "$behind" -eq 0 ]; then
  note OK "rebased on $BASE"
else
  note WARN "$behind commit(s) on $BASE not in branch -- ask the lane to rebase"
  fail=1
fi

# 2/3. Ownership, using the same ledger tmuxctl enforces.
# No mapfile here: macOS ships bash 3.2, which does not have it.
n_changed=$(git diff --name-only "$BASE...$BRANCH" | wc -l | tr -d ' ')
if [ "$n_changed" -eq 0 ]; then
  note WARN "no changes"
else
  note OK "$n_changed changed path(s)"
fi

python3 - "$LANE" "$BASE" "$BRANCH" <<'PY'
import subprocess, sys, pathlib
sys.path.insert(0, str(pathlib.Path("scripts")))
import tmuxctl

lane, base, branch = sys.argv[1], sys.argv[2], sys.argv[3]
owned = tmuxctl.owned_paths(lane)
core = tmuxctl.LANE_PATHS.get(lane, [])
changed = subprocess.run(
    ["git", "diff", "--name-only", "-z", f"{base}...{branch}"],
    capture_output=True, text=True).stdout.split("\0")
changed = [path for path in changed if path]

stray, grant_edits = [], []
for p in changed:
    if any(p.startswith(o) for o in owned):
        if not any(p.startswith(c) for c in core) and not p.startswith(
                (f".agents/{lane}", f"docs/lanes/{lane}", f"tests/wpt-{lane}")):
            grant_edits.append(p)
    else:
        stray.append(p)

rc = 0
for p in stray:
    print(f"  FAIL out-of-lane: {p}")
    rc = 1

# A grant covered added arms / an added entry point. Deletions mean more.
for p in grant_edits:
    ns = subprocess.run(["git", "diff", "--numstat", f"{base}...{branch}", "--", p],
                        capture_output=True, text=True).stdout.split()
    added, removed = (ns[0], ns[1]) if len(ns) >= 2 else ("?", "?")
    if tmuxctl.grant_is_rewrite(lane, p):
        print(f"  REVIEW rewrite grant, read this diff (+{added} -{removed}): {p}")
    elif removed not in ("0", "?"):
        print(f"  FAIL additive grant has deletions (+{added} -{removed}): {p}")
        rc = 1
    else:
        print(f"  OK   granted, additions only (+{added} -{removed}): {p}")
sys.exit(rc)
PY
[ $? -ne 0 ] && fail=1

# 4. Did the lane actually report?
status=".agents/$LANE/status.json"
report=".agents/$LANE/report.md"
if [ -s "$report" ]; then note OK "report.md present"; else note WARN "no report.md"; fail=1; fi
if grep -q '"state"[[:space:]]*:[[:space:]]*"done"' "$status" 2>/dev/null; then
  note OK "status.json state=done"
else
  note WARN "status.json not marked done"
fi

echo
if [ "$fail" -eq 0 ]; then
  echo "PREFLIGHT PASS -- read the report's measured numbers, then merge."
else
  echo "PREFLIGHT ISSUES -- resolve above before merging."
fi
exit "$fail"
