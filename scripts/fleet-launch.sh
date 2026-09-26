#!/usr/bin/env bash
# (Re)launch lane agents in tmux sessions named amz-<lane> and hand each its
# resume instructions. Survives Mac restarts: nothing depends on /tmp.
#
# Usage: scripts/fleet-launch.sh codex|opencode <lane>...
#   codex    -> tmuxctl.AGENT_LAUNCH (Codex gpt-6-sol medium)
#   opencode -> opencode --auto (OpenCode 2, DeepSeek V4.1 Flash)
# FLEET_MSG_FILE overrides the resume message (LANE is replaced by the lane).

set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
KIND="${1:?usage: fleet-launch.sh codex|opencode <lane>...}"; shift
case "$KIND" in
  codex) LAUNCH="$(python3 -c "import sys;sys.path.insert(0,'scripts');import tmuxctl;print(tmuxctl.AGENT_LAUNCH)")" ;;
  opencode) LAUNCH="opencode --auto" ;;
  *) echo "fleet-launch: unknown agent kind $KIND" >&2; exit 2 ;;
esac

MSG='RESUME, lane LANE. The previous agent stopped when the Mac restarted; your worktree and commits are intact. Read AGENTS.md (Linux builds run on your own Boat sandbox through scripts/linux-run.sh LANE -- '"'"'<cmd>'"'"'; Windows gates through scripts/win-run.sh, which falls back to win-001; never run boat commands yourself), then .agents/LANE/brief.md, .agents/LANE/report.md, docs/lanes/LANE_TODO.md, .agents/LANE/status.json and your open items in .agents/LANE/questions.md. Check git status and git log -5 for uncommitted or recent work, merge local branch dev if you are behind, and continue your current ticket in bulk. Update .agents/LANE/status.json with the real UTC time (date -u) at least every 15 minutes. When a ticket is ready set state done with the SHA and measured numbers, then continue with the next item without waiting.'
[ -n "${FLEET_MSG_FILE:-}" ] && MSG="$(cat "$FLEET_MSG_FILE")"

for l in "$@"; do
  wt="$(python3 -c "import json;print(json.load(open('.agents/agents.json'))['$l']['worktree'])")"
  s="amz-$l"
  tmux kill-session -t "$s" 2>/dev/null
  tmux new-session -d -s "$s" -x 200 -y 50 -c "$wt"
  pane="$(tmux display-message -p -t "$s" '#{pane_id}')"
  python3 - "$l" "$s" "$pane" <<'PY'
import json, sys
p = '.agents/agents.json'; d = json.load(open(p)); l, s, pane = sys.argv[1:]
d[l]['session'] = s; d[l]['pane'] = pane
json.dump(d, open(p, 'w'), indent=1)
PY
  tmux send-keys -t "$s" -l "cd $wt && $LAUNCH"; tmux send-keys -t "$s" Enter
done
sleep 20
for l in "$@"; do
  for _ in $(seq 1 12); do
    tmux capture-pane -p -t "amz-$l" | grep -qE "Ask Codex|Ask anything|ctrl\+p commands" && break
    sleep 5
  done
  python3 scripts/tmuxctl.py send "$l" "${MSG//LANE/$l}"
done
