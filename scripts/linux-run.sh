#!/usr/bin/env bash
# Run a lane's build or test command on its configured Linux host.
#
# The current fleet is Boat-only (.agents/.boat-only), including final Linux
# gates and render sweeps. Windows gates are deferred by AGENTS.md; if requested,
# Windows-only work uses server@win-001. Legacy hosts below remain opt-in for
# configurations without the Boat-only marker.
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
#   --mission-package PATH --mission-sha256 SHA256
#               Boat-only reviewed mission transport with its own source guards;
#               requires --ref matching plan.json and stages without unpacking locally
#   --existing-box
#               guarded mission only: use its live endpoint without lifecycle changes
#   --setup-summary PATH  compact local setup-failure receipt; no remote raw fetch
#   --endpoint-receipt PATH --compact-root REMOTE_DIR
#               capture one Boat job endpoint and its completed compact catalog
#   --command-file PATH   exact command bytes from a file, avoiding argv limits
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
HOST="" REF="" SLOT=1 MISSION_PACKAGE="" MISSION_SHA256="" EXISTING_BOX=0 SETUP_SUMMARY="" ENDPOINT_RECEIPT="" COMPACT_ROOT="" COMMAND_FILE=""
if [ -e "$MAIN/.agents/.boat-only" ]; then HOST=boat; fi
while [ $# -gt 0 ]; do
  case "$1" in
    --host) HOST="$2"; shift 2 ;;
    --ref) REF="$2"; shift 2 ;;
    --mission-package) MISSION_PACKAGE="$2"; shift 2 ;;
    --mission-sha256) MISSION_SHA256="$2"; shift 2 ;;
    --existing-box) EXISTING_BOX=1; shift ;;
    --setup-summary) SETUP_SUMMARY="$2"; shift 2 ;;
    --endpoint-receipt) ENDPOINT_RECEIPT="$2"; shift 2 ;;
    --compact-root) COMPACT_ROOT="$2"; shift 2 ;;
    --command-file) COMMAND_FILE="$2"; shift 2 ;;
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
[ $# -gt 0 ] || [ -n "$COMMAND_FILE" ] || [ "${WHICH:-0}" = 1 ] || { echo "linux-run: no command given" >&2; exit 2; }
if [ -n "$COMMAND_FILE" ]; then
  [ $# = 0 ] && [ -f "$COMMAND_FILE" ] || { echo 'linux-run: command file must replace command arguments' >&2; exit 2; }
  CMD=""; IFS= read -r -d '' CMD < "$COMMAND_FILE" || true
else
  CMD="$*"
fi
if [ -n "$ENDPOINT_RECEIPT$COMPACT_ROOT" ]; then
  [ -n "$ENDPOINT_RECEIPT" ] && [ -n "$COMPACT_ROOT" ] && [ -n "$REF" ] && [ ! -e "$ENDPOINT_RECEIPT" ] || {
    echo 'linux-run: endpoint capture requires a fresh receipt, exact ref and compact root' >&2; exit 2;
  }
  # Reuse the receiver contract before locks, source preparation or allocation.
  python3 "$REPO/scripts/completed-job.py" validate-request --lane "$LANE" --root "$COMPACT_ROOT" || exit 2
fi

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
if [ -n "$ENDPOINT_RECEIPT" ] && [ "$HOST" != boat ]; then
  echo 'linux-run: completed endpoint capture is supported on Boat only' >&2; exit 2
fi

# A reviewed mission can carry its own exact source archives and byte guards.
# Stage a hard link to its immutable transport instead of expanding a second
# full checkout on the Mac. The mission validates reconstructed sources before
# building; this wrapper authenticates transport/ref before any billed setup.
if [ -n "$MISSION_PACKAGE$MISSION_SHA256" ]; then
  [ "$HOST" = boat ] && [ -n "$REF" ] && [ -n "$MISSION_PACKAGE" ] && [ -n "$MISSION_SHA256" ] || {
    echo "linux-run: mission transport requires Boat, an explicit ref, package and SHA256" >&2; exit 2;
  }
fi
if [ "$EXISTING_BOX" = 1 ] && [ -z "$MISSION_PACKAGE" ]; then
  echo "linux-run: --existing-box requires guarded mission transport" >&2; exit 2
fi

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
  case "$REF" in
    HEAD|HEAD~*|HEAD^*)
      echo "linux-run: ambiguous ref $REF; use the explicit source commit SHA (HEAD belongs to the main worktree)" >&2
      exit 2 ;;
  esac
  COMMIT="$(git -C "$MAIN" rev-parse --verify "${REF}^{commit}")" || exit 2
  SRC="$(mktemp -d)"; SRC_TMP="$SRC"
  trap 'rm -rf "$SRC" "$LANE_LOCK"' EXIT
  if [ -n "$MISSION_PACKAGE" ]; then
    python3 - "$MISSION_PACKAGE" "$MISSION_SHA256" "$COMMIT" <<'PY' || exit 2
import hashlib, json, pathlib, sys, tarfile
package, expected, commit = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
with package.open('rb') as stream:
    if hashlib.file_digest(stream, 'sha256').hexdigest() != expected:
        raise SystemExit('linux-run: mission package SHA256 mismatch')
with tarfile.open(package) as archive:
    plans = [m for m in archive.getmembers() if m.isfile() and m.name in ('plan.json', './plan.json')]
    if len(plans) != 1:
        raise SystemExit('linux-run: mission requires one top-level plan.json')
    plan = json.load(archive.extractfile(plans[0]))
    if commit not in (plan.get('before'), plan.get('after')):
        raise SystemExit('linux-run: mission plan does not contain the exact ref')
PY
    ln "$MISSION_PACKAGE" "$SRC/mission.tar.gz" || {
      echo "linux-run: cannot hard-link mission transport; no sandbox started" >&2; exit 2;
    }
    echo "linux-run[$LANE]: guarded mission transport $MISSION_SHA256" >&2
  else
    python3 "$REPO/scripts/gate-source.py" "$MAIN" "$COMMIT" "$SRC" || exit 2
  fi
  echo "linux-run[$LANE]: $HOST at $REF ($COMMIT)" >&2
else
  if [ "$LANE" = integrator-primary ]; then SRC="$MAIN"; else SRC="$(dirname "$MAIN")/amazing-wt/$LANE"; fi
  [ -d "$SRC" ] || { echo "linux-run: no worktree at $SRC" >&2; exit 2; }
  echo "linux-run[$LANE]: $HOST from $SRC ($(git -C "$SRC" rev-parse --short HEAD)$(git -C "$SRC" diff --quiet HEAD || echo ', with uncommitted edits'))" >&2
fi

# Authenticate a complete canonical source packet before any remote job. Avoid
# sending file-by-file writes through snapshot-backed HOME.
SOURCE_ARCHIVE=""
SOURCE_ARCHIVE_SHA=""
if [ "$HOST" = boat ] && [ -z "$MISSION_PACKAGE" ]; then
  SOURCE_ARCHIVE=$(mktemp /tmp/amazing-source-transport.XXXXXX) || exit 3
  trap 'rm -f "${SOURCE_ARCHIVE:-}"; rm -rf "${SRC_TMP:-}" "$LANE_LOCK"' EXIT
  tar -czf "$SOURCE_ARCHIVE" --exclude=./.git --exclude=./target --exclude='./target-*' \
    --exclude=./.agents --exclude=./out --exclude=./corpora -C "$SRC" . || exit 3
  SOURCE_ARCHIVE_SHA=$(python3 - "$SOURCE_ARCHIVE" <<'PYHASH'
import hashlib,sys
with open(sys.argv[1],'rb') as stream: print(hashlib.file_digest(stream,'sha256').hexdigest())
PYHASH
  ) || exit 3
fi

if [ "$HOST" = boat ]; then
  # Resumed sandboxes serve /home/user from a lazy FUSE filesystem that can die
  # under load ("Transport endpoint is not connected"). Fail once; the caller
  # may authorize one clean rerun. Never heal or restart as recovery.
  # A lease file marks the sandbox busy for boat-idle-reaper.py from before the
  # sync until the command exits; without it the reaper stopped a sandbox
  # mid-sync (low load, no cargo yet) and killed the run.
  # Source archives stage directly on native storage. Pooled sandboxes host
  # several lanes, each with a separate source namespace and native target.
  LEASE=".amazing-busy/$LANE.$$"
  JOB_CAPTURED=0 JOB_TERMINAL=0
  ok=0
  ATTEMPTS="1"
  [ -n "$MISSION_PACKAGE" ] && ATTEMPTS="1"
  for attempt in $ATTEMPTS; do
    if [ "$EXISTING_BOX" = 1 ]; then how=endpoint
    else how=ensure; fi
    read -r BOAT_HOST BOAT_PORT BOAT_ID BOAT_POOL BOAT_TARGET < <("$MAIN/scripts/boat-lane.py" "$how" "$LANE") || continue
    [ "$attempt" = 1 ] && echo "linux-run[$LANE]: boat sandbox $BOAT_ID ($BOAT_POOL)" >&2
    BSSH="ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=30 -o ServerAliveInterval=30 -o ServerAliveCountMax=2 -i $HOME/.ssh/ascii_box_ed25519 -p $BOAT_PORT"
    # Drop the lease if we exit before the remote command's own trap takes
    # over (failed or interrupted sync); a stale lease keeps the sandbox billed.
    trap 'if [ "${JOB_CAPTURED:-0}" = 0 ] || [ "${JOB_TERMINAL:-0}" = 1 ]; then $BSSH "$BOAT_HOST" "rm -f $LEASE" 2>/dev/null; fi; rm -rf "${SRC_TMP:-}" "$LANE_LOCK"; rm -f "${SOURCE_ARCHIVE:-}"' EXIT
    # Seed only from the canonical local source below. Snapshot-backed template
    # directories can change metadata during tar reads and fail fresh shard
    # setup; a partial template seed must never become build input.
    if [ -n "$MISSION_PACKAGE" ]; then
      # Large opaque packets must not traverse the lazy HOME filesystem.
      # Upload directly to a run-private native directory; the source archives
      # are reconstructed and guarded by the reviewed mission controller.
      NATIVE_MISSION_DIR="/tmp/amazing-mission-transport-$LANE.$$"
      $BSSH "$BOAT_HOST" "mkdir -p amazing/out/$LANE .amazing-busy $NATIVE_MISSION_DIR && touch $LEASE" &&
      scp -O -q -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
        -i "$HOME/.ssh/ascii_box_ed25519" -P "$BOAT_PORT" "$MISSION_PACKAGE" \
        "$BOAT_HOST:$NATIVE_MISSION_DIR/mission.tar.gz" &&
      $BSSH "$BOAT_HOST" "echo '$MISSION_SHA256  $NATIVE_MISSION_DIR/mission.tar.gz' | sha256sum -c -" &&
      { ok=1; break; }
      echo "linux-run[$LANE]: native mission transport failed; no controller started" >&2
      break
    fi
    NATIVE_SOURCE_DIR="/tmp/amazing-source-transport-$LANE.$$"
    setup_phase=native-source-directory
    $BSSH "$BOAT_HOST" "timeout --kill-after=5 60 bash -c 'mkdir -p amazing/out/$LANE .amazing-busy $NATIVE_SOURCE_DIR && touch $LEASE'"
    setup_rc=$?
    if [ "$setup_rc" = 0 ]; then
      setup_phase=native-source-upload
      COMMIT="${COMMIT:-working-tree}" python3 "$REPO/scripts/native-inputs.py" --receipt "$SRC/source-transfer.manifest.json" transfer --seconds 900 -- \
        scp -O -q -o ConnectTimeout=30 -o ServerAliveInterval=30 -o ServerAliveCountMax=2 \
        -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
        -i "$HOME/.ssh/ascii_box_ed25519" -P "$BOAT_PORT" "$SOURCE_ARCHIVE" \
        "$BOAT_HOST:$NATIVE_SOURCE_DIR/source.tar.gz"
      setup_rc=$?
    fi
    if [ "$setup_rc" = 0 ]; then
      setup_phase=native-source-sha256
      $BSSH "$BOAT_HOST" "timeout --kill-after=5 60 bash -c 'printf \"%s  %s\\n\" $SOURCE_ARCHIVE_SHA $NATIVE_SOURCE_DIR/source.tar.gz | sha256sum -c -'"
      setup_rc=$?
    fi
    if [ "$setup_rc" = 0 ]; then ok=1; break; fi
    echo "linux-run[$LANE]: setup $setup_phase exited $setup_rc on $BOAT_ID; no job started" >&2
    if [ -n "$SETUP_SUMMARY" ]; then
      mkdir -p "$(dirname "$SETUP_SUMMARY")" &&
        python3 "$REPO/scripts/gate-summary.py" failure --source-commit "${COMMIT:-working-tree}" \
          --shard "$LANE" --wrapper-exit 3 "$SETUP_SUMMARY" "$setup_phase" "$setup_rc" || \
          echo "linux-run: compact setup receipt could not be written" >&2
    fi
  done
  [ "$ok" = 1 ] || { echo "linux-run: could not sync to Boat for $LANE after the allowed setup attempts" >&2; exit 3; }
  NATIVE_INPUTS_B64="$(base64 < "$REPO/scripts/native-inputs.py" | tr -d '\n')"
  BOOTSTRAP_B64="$(base64 < "$REPO/scripts/linux-bootstrap.sh" | tr -d '\n')"
  REMOTE="trap 'timeout --kill-after=5 15 rm -f ~/$LEASE' EXIT
export LANE='$LANE' COMMIT='${COMMIT:-working-tree}' OUT=\$HOME/amazing/out/$LANE
bootstrap=\$(mktemp /tmp/amazing-bootstrap-script.XXXXXX) || exit 3
printf '%s' '$BOOTSTRAP_B64' | base64 -d > \"\$bootstrap\" || exit 3
. \"\$bootstrap\"
rm -f \"\$bootstrap\"
bootstrap_clean_python_environment
printf '%s' '$NATIVE_INPUTS_B64' | base64 -d > \"\$BOOTSTRAP_NATIVE/native-inputs.py\" || exit 3
export NATIVE_SOURCE_ARCHIVE='${NATIVE_SOURCE_DIR:-}/source.tar.gz' NATIVE_SOURCE_SHA='$SOURCE_ARCHIVE_SHA'
env_native=\$BOOTSTRAP_NATIVE/environment
bootstrap_stage environment-copy 30 cp ~/.amazing-env \"\$env_native\"
bootstrap_stage environment-load 30 bash -c '. \"\$1\" && export -p > \"\$2\"' bash \"\$env_native\" \"\$env_native.exports\"
. \"\$env_native.exports\" || exit 3
bootstrap_clean_python_environment
bootstrap_native_cargo
export JOBS=\$(nproc) CARGO_BUILD_JOBS=\$(nproc)
export TEST262_ROOT=\$HOME/corpora/test262 RUSTUP_TOOLCHAIN=1.97.1 OUT=\$HOME/amazing/out/$LANE
if [ -n '${NATIVE_MISSION_DIR:-}' ]; then
  cd '${NATIVE_MISSION_DIR:-}' || exit 3
else
bootstrap_native_setup
fi
bootstrap_native_tools
bootstrap_native_inputs
export CARGO_TARGET_DIR=/tmp/amazing-native-target-$LANE-${COMMIT:-working-tree} CARGO_INCREMENTAL=0
export AMAZING_SOURCE_COMMIT='${COMMIT:-working-tree}'
$CMD"
  if [ -n "$ENDPOINT_RECEIPT" ]; then
    # Supervise the entire native bootstrap, not merely the last gate command.
    # Isolated system Python can record ownership before template Python/env
    # setup; every bootstrap failure then has the same actual-exit contract.
    # Capture digests from files: commands may exceed Linux's per-argument
    # limit even when the outer caller can pass them on another platform.
    command_file=$(mktemp) || exit 3
    execution_file=$(mktemp) || { rm -f "$command_file"; exit 3; }
    printf '%s' "$CMD" > "$command_file"
    printf '%s' "$REMOTE" > "$execution_file"
    JOB_ROW=$(python3 "$REPO/scripts/completed-job.py" capture --receipt "$ENDPOINT_RECEIPT" --lane "$LANE" --source "$COMMIT" --box "$BOAT_ID" --host "$BOAT_HOST" --port "$BOAT_PORT" --root "$COMPACT_ROOT" --command-file "$command_file" --execution-file "$execution_file" --source-archive-sha256 "${SOURCE_ARCHIVE_SHA:-$MISSION_SHA256}")
    capture_rc=$?
    if [ "$capture_rc" = 0 ]; then
      REMOTE=$(python3 "$REPO/scripts/completed-job.py" launch "$JOB_ROW" --execution-file "$execution_file")
      capture_rc=$?
    fi
    rm -f "$command_file" "$execution_file"
    [ "$capture_rc" = 0 ] || exit 3
    JOB_CAPTURED=1
  fi
  T0=$(date +%s)
  printf '%s' "$REMOTE" | $BSSH "$BOAT_HOST" "bash -s"
  rc=$?
  if [ -n "$ENDPOINT_RECEIPT" ]; then
    observation="$ENDPOINT_RECEIPT.observation.summary.json"
    python3 "$REPO/scripts/completed-job.py" wait --receipt "$ENDPOINT_RECEIPT" --lane "$LANE" --source "$COMMIT" --seconds 60 --output "$observation"
    observed_rc=$?
    if python3 - "$observation" <<'PYJOB'
import json,sys
row=json.load(open(sys.argv[1]))
raise SystemExit(0 if row.get('state')=='TERMINAL' and type(row.get('runner_exit')) is int else 1)
PYJOB
    then
      JOB_TERMINAL=1
      rc=$observed_rc
    else
      echo "linux-run[$LANE]: native job UNKNOWN/RUNNING; wrapper observation exit $rc; captured lease preserved" >&2
      [ "$rc" != 0 ] || rc=3
    fi
  fi
  # Start/end per run, for `boat-lane.py pool-report` (busy and overlap per pool).
  python3 "$REPO/scripts/boat-lane.py" record-run "$T0" "$(date +%s)" "$LANE" "$BOAT_POOL" "$rc" || \
    echo "linux-run: activity recording failed; wrapper/status exit remains $rc" >&2
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
