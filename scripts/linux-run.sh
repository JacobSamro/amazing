#!/usr/bin/env bash
# Run a lane's build or test command on a Linux build host (venus-*).
#
# win-001 is the reference platform and the only place merge gates and render
# sweeps count. Everything platform-independent -- crate and workspace nextest
# while iterating, full Test262, the WPT dom/ harness, the net corpora -- runs
# here instead, so it stops queueing behind win-001's three slots.
#
# The lane's working tree, including uncommitted edits, is rsynced to
# ~/amazing/wt/<lane> on its host, and the command runs there with the lane's
# own CARGO_TARGET_DIR. Heavy commands take a slot from that host's pool.
#
# Usage:  scripts/linux-run.sh <lane> [--host H] [--ref REF] [--no-slot] -- <command>
#         scripts/linux-run.sh <lane> --which   (prints the lane's host)
#   <command> is one bash string, run from the synced tree root. For example:
#     scripts/linux-run.sh css -- 'cargo nextest run -p amazing-css --locked'
#     scripts/linux-run.sh js-lang -- 'cargo build -p amazing-js --example test262_host --release --locked &&
#       python3 scripts/test262.py "$TEST262_ROOT" --binary "$CARGO_TARGET_DIR/release/examples/test262_host" --jobs "$JOBS" --timeout 60 --output ~/amazing/out/js-lang.json'
#   --ref REF   sync that exact commit (git archive) instead of the working tree
#   --no-slot   skip the slot queue; for crate-scoped iterations only
#
# Environment on the host: CARGO_TARGET_DIR, JOBS (per-slot parallelism),
# TEST262_ROOT (pinned 3655e746 checkout), toolchain 1.97.1.
# Results stay on the host; copy what you need back with
#   scp -o ControlPath=none <host>:<path> <local>

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"

LANE="${1:?usage: linux-run.sh <lane> [--host H] [--ref REF] [--no-slot] -- <command>}"
shift
HOST="" REF="" SLOT=1
if [ -e "$MAIN/.agents/.boat-only" ]; then HOST=boat; fi
while [ $# -gt 0 ]; do
  case "$1" in
    --host) HOST="$2"; shift 2 ;;
    --ref) REF="$2"; shift 2 ;;
    --no-slot) SLOT=0; shift ;;
    --which) WHICH=1; shift ;;
    --) shift; break ;;
    *) echo "linux-run: unknown option $1" >&2; exit 2 ;;
  esac
done
if [ -e "$MAIN/.agents/.boat-only" ] && [ "$HOST" != boat ]; then
  echo "linux-run: this fleet is configured for Boat only; use --host boat" >&2
  exit 2
fi
[ $# -gt 0 ] || [ "${WHICH:-0}" = 1 ] || { echo "linux-run: no command given" >&2; exit 2; }
CMD="$*"

# Lane homes. Since 2026-09-26 lanes run on pooled Boat sandboxes
# (scripts/boat-lane.py: related lanes share one sandbox from the amazing-builder
# template, stopped by boat-idle-reaper.py after 5 idle minutes). Outputs go to
# $OUT (~/amazing/out/<lane>) on the sandbox. The venus hosts remain
# available with --host venus-001 / venus-003.
if [ -z "$HOST" ]; then
  # Boat needs the amazing-builder template; without it (for example after
  # delete-on-stop data retention removed it) fall back to the venus hosts.
  # .agents/.boat-off forces the fallback: on 2026-09-26 the fleet used up
  # Boat's 200 sandbox starts per day and every resume was refused.
  # Since 2026-09-27 lanes build on the direct hosts below and Boat keeps only
  # what needs it: parallel Test262 shards and the shell lane's virtual desktop.
  # That keeps sandbox starts well under the daily quota.
  case "$LANE" in t262-shard-*|shell) boat_lane=1 ;; *) boat_lane=0 ;; esac
  if [ "$boat_lane" = 1 ] && [ ! -e "$MAIN/.agents/.boat-off" ] &&
     "$HOME/.ascii/bin/boat" snapshots --json 2>/dev/null | grep -q '"name":"amazing-builder"'; then
    HOST=boat
  else
    # venus-001 (32 threads) takes the Test262-heavy lanes and the gates;
    # venus-003 (16) the other JS lanes; worker-2 (16) layout/render/paint;
    # k-test (16) the DOM, HTML, net and platform lanes; worker-3 (4) ipc.
    case "$LANE" in
      js-lang|js-vm|js-binary|js-date|integrator-primary|t262-shard-*) HOST=venus-001 ;;
      js-runtime|js-stdlib|js-intl|wasm) HOST=venus-003 ;;
      layout|layout-flexgrid|css|text|shell|replaced-content|compositor) HOST=worker-2 ;;
      dom-bindings|html|net|storage|a11y|devtools|integrator-assist) HOST=k-test ;;
      ipc) HOST=worker-3 ;;
      *) HOST=venus-001 ;;
    esac
    [ "$boat_lane" = 1 ] && [ ! -e "$MAIN/.agents/.boat-off" ] && echo "linux-run: Boat template missing, using $HOST" >&2
  fi
fi
# SSH is the ssh destination; the worker hosts are root logins keyed by the
# agent's id_rsa (no ~/.ssh/config entries).
case "$HOST" in
  boat) SLOTS=0 JOBS=0 SSH=boat ;;
  venus-001) SLOTS=3 JOBS=10 SSH=venus-001 ;;
  venus-003) SLOTS=2 JOBS=8 SSH=venus-003 ;;
  worker-2) SLOTS=2 JOBS=8 SSH=root@46.224.130.223 ;;
  k-test) SLOTS=2 JOBS=8 SSH=root@159.69.124.56 ;;
  worker-3) SLOTS=1 JOBS=4 SSH=root@178.104.111.133 ;;
  *) echo "linux-run: unknown host $HOST" >&2; exit 2 ;;
esac
# --which: print the ssh destination this lane runs on ("boat" for Boat), for copy-backs.
[ "${WHICH:-0}" = 1 ] && { echo "$SSH"; exit 0; }

# One run per lane tree at a time. Two runs of the same lane share the remote
# tree and output paths; on 2026-09-24 a second comparison synced a different
# commit into the tree mid-build and made a Test262 baseline measure the wrong
# source. Held for the whole sync + run.
LOCKDIR="$MAIN/.agents/.remote-locks"
mkdir -p "$LOCKDIR"
LANE_LOCK="$LOCKDIR/$(basename "$0" .sh)-$LANE"
until mkdir "$LANE_LOCK" 2>/dev/null; do
  holder="$(cat "$LANE_LOCK/pid" 2>/dev/null || true)"
  if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then rm -rf "$LANE_LOCK"; continue; fi
  # A holder killed between mkdir and writing its pid leaves a pid-less lock
  # that no waiter could ever reclaim (2026-09-27: compositor and
  # replaced-content hung behind one for an hour); reclaim it after 2 minutes.
  if [ -z "$holder" ] && [ -n "$(find "$LANE_LOCK" -maxdepth 0 -mmin +2 2>/dev/null)" ]; then rm -rf "$LANE_LOCK"; continue; fi
  sleep 5
done
echo "$$" > "$LANE_LOCK/pid"

trap 'rm -rf "$LANE_LOCK"' EXIT
# Source: the lane's worktree (with uncommitted edits), or an exact commit.
if [ -n "$REF" ]; then
  COMMIT="$(git -C "$MAIN" rev-parse "$REF")" || exit 2
  SRC="$(mktemp -d)"; SRC_TMP="$SRC"
  trap 'rm -rf "$SRC" "$LANE_LOCK"' EXIT
  git -C "$MAIN" archive "$COMMIT" | tar x -C "$SRC"
  echo "linux-run[$LANE]: $HOST at $REF ($COMMIT)" >&2
else
  if [ "$LANE" = integrator-primary ]; then SRC="$MAIN"; else SRC="$(dirname "$MAIN")/amazing-wt/$LANE"; fi
  [ -d "$SRC" ] || { echo "linux-run: no worktree at $SRC" >&2; exit 2; }
  echo "linux-run[$LANE]: $HOST from $SRC ($(git -C "$SRC" rev-parse --short HEAD)$(git -C "$SRC" diff --quiet HEAD || echo ', with uncommitted edits'))" >&2
fi

if [ "$HOST" = boat ]; then
  # Resumed sandboxes serve /home/user from a lazy FUSE filesystem that can die
  # under load ("Transport endpoint is not connected"); retry the setup up to
  # three times, restarting a broken sandbox in between (boat-lane.py heal).
  # A lease file marks the sandbox busy for boat-idle-reaper.py from before the
  # sync until the command exits; without it the reaper stopped a sandbox
  # mid-sync (low load, no cargo yet) and killed the run.
  # A new lane tree is seeded on the sandbox from its own ~/amazing copy, so
  # rsync sends only the lane's differences instead of the whole 85 MB tree.
  # Pooled sandboxes host several lanes: each syncs into ~/amazing/wt/<lane>
  # and builds into its own target dir (boat-lane.py target_of: a shared one
  # linked another lane's stale artifacts).
  LEASE=".amazing-busy/$LANE.$$"
  ok=0
  for attempt in 1 2 3; do
    if [ "$attempt" = 1 ]; then how=ensure; else how=heal; sleep 10; fi
    read -r BOAT_HOST BOAT_PORT BOAT_ID BOAT_POOL BOAT_TARGET < <("$MAIN/scripts/boat-lane.py" "$how" "$LANE") || continue
    [ "$attempt" = 1 ] && echo "linux-run[$LANE]: boat sandbox $BOAT_ID ($BOAT_POOL)" >&2
    BSSH="ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ServerAliveInterval=30 -i $HOME/.ssh/ascii_box_ed25519 -p $BOAT_PORT"
    # Drop the lease if we exit before the remote command's own trap takes
    # over (failed or interrupted sync); a stale lease keeps the sandbox billed.
    trap '$BSSH "$BOAT_HOST" "rm -f $LEASE" 2>/dev/null; rm -rf "${SRC_TMP:-}" "$LANE_LOCK"' EXIT
    $BSSH "$BOAT_HOST" "mkdir -p amazing/wt/$LANE amazing/out/$LANE .amazing-busy && touch $LEASE &&
      { [ -e amazing/wt/$LANE/Cargo.toml ] || [ ! -e amazing/Cargo.toml ] ||
        tar -C amazing -cf - --exclude=./target --exclude='./target-*' --exclude=./wt --exclude=./out --exclude=./corpora --exclude=./.git --exclude=./.agents . | tar -C amazing/wt/$LANE -xf -; }" &&
    rsync -rlpDz --checksum --delete --exclude .git --exclude /target --exclude '/target-*' --exclude /out --exclude /corpora \
      --exclude /.agents --exclude /.opencode --exclude .DS_Store -e "$BSSH" "$SRC/" "$BOAT_HOST:amazing/wt/$LANE/" &&
    { ok=1; break; }
    echo "linux-run[$LANE]: setup attempt $attempt on $BOAT_ID failed" >&2
  done
  [ "$ok" = 1 ] || { echo "linux-run: could not sync to Boat for $LANE after 3 attempts" >&2; exit 3; }
  REMOTE="trap 'rm -f ~/$LEASE' EXIT
. ~/.amazing-env
cd ~/amazing/wt/$LANE || exit 3
export CARGO_TARGET_DIR=\$HOME/$BOAT_TARGET JOBS=\$(nproc) CARGO_BUILD_JOBS=\$(nproc)
export TEST262_ROOT=\$HOME/corpora/test262 RUSTUP_TOOLCHAIN=1.97.1 OUT=\$HOME/amazing/out/$LANE
ln -sfn ~/corpora ~/amazing/wt/$LANE/corpora
# Snapshot-backed HOME produces stale/missing rlibs and failed append writes.
# Keep synced source and compilation on the VM's native filesystem.
mkdir -p /tmp/amazing-native-src-$LANE || exit 3
rsync -a --delete --exclude /target --exclude '/target-*' --exclude /out --exclude /corpora ./ /tmp/amazing-native-src-$LANE/ || exit 3
ln -sfn \$HOME/corpora /tmp/amazing-native-src-$LANE/corpora
cd /tmp/amazing-native-src-$LANE || exit 3
export CARGO_TARGET_DIR=/tmp/amazing-native-target-$LANE CARGO_INCREMENTAL=0
# nextest is also snapshot-backed in HOME. A late mmap fault can kill its
# controller after every test passes; execute a native copy instead.
# Publish atomically so sibling lanes never truncate a running executable.
mkdir -p /tmp/amazing-native-tools || exit 3
if [ ! -x /tmp/amazing-native-tools/cargo-nextest ]; then
  nextest_source=\$(command -v cargo-nextest) || exit 3
  nextest_copy=\$(mktemp /tmp/amazing-native-tools/.cargo-nextest.XXXXXX) || exit 3
  cp \"\$nextest_source\" \"\$nextest_copy\" && chmod +x \"\$nextest_copy\" &&
    mv -f \"\$nextest_copy\" /tmp/amazing-native-tools/cargo-nextest || { rm -f \"\$nextest_copy\"; exit 3; }
fi
export PATH=/tmp/amazing-native-tools:\$PATH
free=\$(df -Pk ~ | awk 'NR==2{print int(\$4/1048576)}'); if [ \"\$free\" -lt 10 ]; then rm -rf \"\$CARGO_TARGET_DIR\"/*/incremental; find \"\$OUT\" -type f -mmin +120 -size +1M -delete 2>/dev/null; echo \"linux-run: only \${free} GB free; cleared this lane's incremental cache and outputs older than 2 h\" >&2; fi
$CMD"
  ENCODED="$(printf '%s' "$REMOTE" | base64 | tr -d '\n')"
  T0=$(date +%s)
  $BSSH "$BOAT_HOST" "echo $ENCODED | base64 -d | bash"
  rc=$?
  # Start/end per run, for `boat-lane.py pool-report` (busy and overlap per pool).
  echo "$T0 $(date +%s) $LANE $BOAT_POOL $rc" >> "$MAIN/.agents/.boat-runs.log"
  exit $rc
fi

# Bulk transfers get their own connection; a stalled one on the shared master
# blocks every other lane's ssh.
#
# No -t: modification times are NOT preserved. `git archive` stamps every file
# with the commit time, which is older than a previous build's artifacts, so
# cargo reused a stale binary after a --ref switch (JSRT-6's Test262 check ran
# the 7c6824b host). --checksum rewrites only files whose content changed, and
# a rewritten file gets the current time, so cargo always sees real changes.
#
# The worker hosts (root@ logins) are reached from the Mac at ~10 KB/s, so their
# trees are relayed: Mac -> venus-001 (fast) -> worker (venus-001's
# ~/.ssh/amazing_relay key is authorized there; ~4.5 MB/s).
EXCL=(--exclude .git --exclude /target --exclude /.agents --exclude /corpora --exclude /out --exclude .DS_Store)
case "$SSH" in root@*) RELAY=venus-001 ;; *) RELAY="" ;; esac
if [ -n "$RELAY" ]; then
  ssh -o ControlPath=none "$RELAY" "mkdir -p amazing/relay/$LANE" &&
  rsync -rlpDz --checksum --delete "${EXCL[@]}" -e "ssh -o ControlPath=none" "$SRC/" "$RELAY:amazing/relay/$LANE/" &&
  scp -q -o ControlPath=none "$MAIN/scripts/ensure-wpt.sh" "$RELAY:amazing/relay/$LANE/scripts/ensure-wpt.sh" &&
  ssh -o ControlPath=none "$RELAY" "rsync -rlpDz --checksum --delete ${EXCL[*]} \
    -e 'ssh -i ~/.ssh/amazing_relay -o BatchMode=yes -o StrictHostKeyChecking=accept-new' \
    amazing/relay/$LANE/ $SSH:amazing/wt/$LANE/" || {
    echo "linux-run: relay sync to $HOST via $RELAY failed" >&2; exit 3; }
else
  rsync -rlpDz --checksum --delete "${EXCL[@]}" -e "ssh -o ControlPath=none" "$SRC/" "$SSH:amazing/wt/$LANE/" || {
    echo "linux-run: rsync to $HOST failed" >&2; exit 3; }
  # Lane trees may predate ensure-wpt.sh's git-less (tarball) path; use main's.
  scp -q -o ControlPath=none "$MAIN/scripts/ensure-wpt.sh" "$SSH:amazing/wt/$LANE/scripts/ensure-wpt.sh" 2>/dev/null
fi

REMOTE=". ~/.amazing-env
cd ~/amazing/wt/$LANE || exit 3
export CARGO_TARGET_DIR=\$HOME/amazing/target/$LANE JOBS=$JOBS CARGO_BUILD_JOBS=$JOBS
export TEST262_ROOT=\$HOME/amazing/corpora/test262 RUSTUP_TOOLCHAIN=1.97.1 OUT=\$HOME/amazing/out/$LANE
mkdir -p \"\$OUT\"
# Same layout as a Boat sandbox: ~/corpora/{test262,wpt} and a corpora link in the tree.
ln -sfn ~/corpora ~/amazing/wt/$LANE/corpora
$CMD"
ENCODED="$(printf '%s' "$REMOTE" | base64 | tr -d '\n')"
run() { ssh "$SSH" "echo $ENCODED | base64 -d | bash" 2>&1 | grep -v -i -E 'post-quantum|store now|pq\.html'; return "${PIPESTATUS[0]}"; }

if [ "$SLOT" = 1 ]; then
  BUILD_POOL="$HOST" BUILD_SLOTS="$SLOTS" "$REPO/scripts/with-build-slot.sh" "$LANE" -- bash -c "$(declare -f run); ENCODED='$ENCODED' SSH='$SSH' run"
else
  run
fi
