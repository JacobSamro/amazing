#!/usr/bin/env bash
# One fixed-source gate phase; stdout/stderr and raw JSON remain below remote OUT.
# The paired source caller stages this same reviewed script and corpus controller
# under OUT for BEFORE and AFTER; the baseline engine never gains candidate code.
set -uo pipefail
phase=${1:?before or after}
source=${2:?exact source}
base=${3:?exact base}
controller=${4:?staged corpus controller path}
run="$OUT/inline-csp/$phase"
mkdir -p "$run/compact"
phases=()
last_phase_exit=0
run_phase() {
  local name=$1
  shift
  "$@" >"$run/$name.log" 2>&1
  local result=$?
  last_phase_exit=$result
  phases+=("$name:$result")
  echo "$phase $name exit=$result"
}
run_phase workspace cargo nextest run --workspace --locked --no-fail-fast
run_phase docs cargo test --workspace --locked --doc
run_phase build cargo build -p amazing --locked
build_exit=$last_phase_exit
run_phase wpt-setup bash scripts/ensure-wpt.sh
wpt_exit=$last_phase_exit
baseline=()
[ "$phase" = before ] || baseline=(--baseline "$OUT/inline-csp/before/corpus/compact.json")
if [ "$build_exit" = 0 ] && [ "$wpt_exit" = 0 ] && [ -x "$CARGO_TARGET_DIR/debug/amazing" ]; then
  run_phase corpus python3 "$controller" --binary "$CARGO_TARGET_DIR/debug/amazing" \
    --wpt-root "$HOME/corpora/wpt" --output "$run/corpus" --source "$source" --base "$base" \
    --jobs "${JOBS:-4}" "${baseline[@]}"
else
  phases+=("corpus:127")
fi
python3 - "$run" "$source" "$base" "${phases[@]}" <<'PY'
import hashlib,json,re,sys,pathlib
run=pathlib.Path(sys.argv[1]);phases=dict(item.split(':') for item in sys.argv[4:])
summary={'source':sys.argv[2],'base':sys.argv[3],'remote_run':str(run),'phases':{},'raw':{}}
for name,status in phases.items():
 p=run/(name+'.log');text=p.read_text(errors='replace') if p.exists() else ''
 summary['phases'][name]={'exit':int(status)}
 errors=[]
 lines=text.splitlines()
 for index,line in enumerate(lines):
  if line.startswith('error:') or line.startswith('error['):
   errors.append('\n'.join(lines[index:index+5]))
 if errors:summary['phases'][name]['compiler_errors']=errors[:32]
 for line in text.splitlines():
  if ('tests run:' in line and ('passed' in line or 'failed' in line)) or line.startswith('test result:'):
   summary['phases'][name].setdefault('counts',[]).append(line.strip())
 if p.exists():summary['raw'][name]={'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
(run/'compact'/'gate.summary.json').write_text(json.dumps(summary,sort_keys=True,separators=(',',':'))+'\n')
p=run/'corpus'/'compact.json'
if p.exists():(run/'compact'/'corpus.compact.json').write_bytes(p.read_bytes())
PY
result=0
for phase_status in "${phases[@]}"; do [ "${phase_status#*:}" = 0 ] || result=1; done
exit "$result"
