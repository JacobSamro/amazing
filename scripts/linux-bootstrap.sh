#!/usr/bin/env bash
# Sourced on Boat before user commands. Each filesystem/tool stage is finite.
BOOTSTRAP_NATIVE=$(mktemp -d /tmp/amazing-bootstrap.XXXXXX) || exit 3
bootstrap_stage() {
  local label="$1" seconds="$2"; shift 2
  local log="$BOOTSTRAP_NATIVE/$label.log" receipt="$BOOTSTRAP_NATIVE/$label.manifest.json" rc publication=0
  timeout --signal=TERM --kill-after=5 "$seconds" "$@" >"$log" 2>&1
  rc=$?
  timeout --kill-after=5 20 python3 - "$label" "$rc" "$log" "$receipt" "${COMMIT:-working-tree}" <<'PY'
import hashlib,json,pathlib,sys
label,code,log,receipt,source=sys.argv[1:]; p=pathlib.Path(log)
r={'schema':1,'phase':'bootstrap.'+label,'actual_exit':int(code),'source_commit':source,
   'raw':{'path':str(p),'bytes':p.stat().st_size,'sha256':hashlib.file_digest(p.open('rb'),'sha256').hexdigest()}}
with p.open('rb') as f:
 f.seek(max(0,p.stat().st_size-4096));r['diagnostic_tail']=f.read().decode(errors='replace').splitlines()[-8:]
text=json.dumps(r,separators=(',',':'))+'\n';pathlib.Path(receipt).write_text(text);print(text,end='')
PY
  publication=$?
  # Durable publication is separately bounded; its failure never hides stage exit.
  if [ "$publication" = 0 ]; then
    timeout --kill-after=5 20 cp "$receipt" "$OUT/bootstrap-$label.manifest.json"
    publication=$?
  fi
  [ "$publication" = 0 ] || echo "bootstrap receipt publication failed: phase=$label exit=$publication native=$receipt" >&2
  [ "$rc" = 0 ] && [ "$publication" = 0 ] || exit 3
}
bootstrap_clean_python_environment() {
local name
for name in "${!PYTHON@}" "${!_PYTHON@}"; do
 [ -z "$name" ] || unset "$name"
done
}

bootstrap_native_setup() {
local destination="$BOOTSTRAP_NATIVE/source"
bootstrap_stage native-source-extract 600 /usr/bin/python3 -s -S -P "$BOOTSTRAP_NATIVE/native-inputs.py" \
 --receipt "$BOOTSTRAP_NATIVE/source-input.manifest.json" source \
 "$NATIVE_SOURCE_ARCHIVE" "$NATIVE_SOURCE_SHA" "$destination"
cd "$destination" || exit 3
}
bootstrap_native_tools() {
bootstrap_stage native-tools-directory 30 mkdir -p /tmp/amazing-native-tools
bootstrap_stage native-nextest 120 bash -c '
 if [ -x /tmp/amazing-native-tools/cargo-nextest ]; then exit 0; fi
 source=$(command -v cargo-nextest) || exit 3
 copy=$(mktemp /tmp/amazing-native-tools/.cargo-nextest.XXXXXX) || exit 3
 trap '\''rm -f "$copy"'\'' EXIT
 cp "$source" "$copy" && chmod +x "$copy" && mv -f "$copy" /tmp/amazing-native-tools/cargo-nextest
'
export PATH=/tmp/amazing-native-tools:$PATH
}

bootstrap_native_cargo() {
# Native targets alone are insufficient: C/C++ build scripts also read their
# registry sources. Snapshot-backed HOME registry reads have failed with
# ENOTCONN during OpenH264 and Opus compilation. Let Cargo download and unpack
# locked packages directly on the native filesystem; never copy the lazy tree.
local cargo_native="/tmp/amazing-native-cargo-$LANE"
bootstrap_stage native-cargo-directory 30 python3 - "$cargo_native" <<'PY'
import os,pathlib,sys
p=pathlib.Path(sys.argv[1])
p.mkdir(exist_ok=True)
if p.is_symlink() or p.resolve()!=p or p.stat().st_dev!=pathlib.Path('/tmp').stat().st_dev:
 raise SystemExit('Cargo home must be an actual native directory')
for name in ('registry','git'):
 q=p/name
 if q.exists() and (q.is_symlink() or q.stat().st_dev!=p.stat().st_dev):
  raise SystemExit('Cargo dependency storage must remain native')
print(str(p))
PY
export CARGO_HOME="$cargo_native"
}

bootstrap_native_inputs() {
# No ambient PyYAML C module is imported until its authentic installed bytes
# have been copied to native storage. System stdlib remains the interpreter's.
bootstrap_stage native-python-input 120 /usr/bin/python3 -s -S -P "$BOOTSTRAP_NATIVE/native-inputs.py" \
 --receipt "$BOOTSTRAP_NATIVE/python-input.manifest.json" python "$BOOTSTRAP_NATIVE/python"
unset PYTHONHOME PYTHONUSERBASE
export PYTHONPATH="$BOOTSTRAP_NATIVE/python" PYTHONNOUSERSITE=1
export PATH="$BOOTSTRAP_NATIVE/python:$PATH"
bootstrap_stage native-test262-input 1200 python3 -s -S -P "$BOOTSTRAP_NATIVE/native-inputs.py" \
 --receipt "$BOOTSTRAP_NATIVE/test262-input.manifest.json" corpus /tmp/amazing-test262-3655e746
export TEST262_ROOT=/tmp/amazing-test262-3655e746
export NATIVE_CORPORA_ROOT="$BOOTSTRAP_NATIVE/corpora"
bootstrap_stage native-corpora-directory 30 mkdir -p "$NATIVE_CORPORA_ROOT"
bootstrap_stage native-corpora-links 30 bash -c '
 ln -s "$1" "$2/test262" && ln -s "$3/wpt" "$2/wpt"
' bash "$TEST262_ROOT" "$BOOTSTRAP_NATIVE/corpora" "$HOME/corpora"
if [ -z "${NATIVE_MISSION_DIR:-}" ]; then
 bootstrap_stage native-source-corpora 30 ln -s "$BOOTSTRAP_NATIVE/corpora" corpora
fi
}
