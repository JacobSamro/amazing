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
bootstrap_native_setup() {
# Do not mutate snapshot-backed HOME corpora links. Only the native tree needs one.
bootstrap_stage native-source-directory 30 mkdir -p "/tmp/amazing-native-src-$LANE"
bootstrap_stage native-source-sync 600 rsync -a --delete --exclude /target --exclude '/target-*' --exclude /out --exclude /corpora ./ "/tmp/amazing-native-src-$LANE/"
bootstrap_stage native-corpora-link 30 bash -c 'dest="$1/corpora"; expected="$2"; if [ -L "$dest" ] && [ "$(readlink "$dest")" = "$expected" ]; then exit 0; fi; ln -sfn "$expected" "$dest"' bash "/tmp/amazing-native-src-$LANE" "$HOME/corpora"
cd "/tmp/amazing-native-src-$LANE" || exit 3
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
