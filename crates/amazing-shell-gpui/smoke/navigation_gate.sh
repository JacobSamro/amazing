#!/usr/bin/env bash
# Native Linux bulk gate. No provisioning/lifecycle; invoke through supported linux-run.
# Caller must provide exact SOURCE and admitted OUT/CARGO_TARGET_DIR.
set -euo pipefail
: "${SOURCE:?exact source}" "${OUT:?native raw output}" "${CARGO_TARGET_DIR:?native target}"
GATE_OUT="$OUT/product-navigation-$SOURCE"
mkdir "$GATE_OUT"
export CARGO_INCREMENTAL=0
cargo nextest run --workspace --locked --no-fail-fast > "$GATE_OUT/workspace.log" 2>&1
cargo test --workspace --locked --doc > "$GATE_OUT/doctests.log" 2>&1
ROOT=$PWD
SHELL_TARGET="$CARGO_TARGET_DIR"
PROOF_LANE=${PROOF_LANE:-product-responsive-navigation}
WINDOW_OUT="/tmp/amazing-navigation-window-$SOURCE/$PROOF_LANE"
(
  cd crates/amazing-shell-gpui
  export CARGO_TARGET_DIR="$SHELL_TARGET"
  cargo nextest run --locked --no-fail-fast > "$GATE_OUT/shell-tests.log" 2>&1
  # This standalone package has only binaries; workspace doctests above are the applicable gate.
  cargo build --locked --bins > "$GATE_OUT/shell-build.log" 2>&1
)
SOURCE="$SOURCE" python3 scripts/linux-capture-session.py --receipt "$GATE_OUT/session.manifest.json" -- \
  bash crates/amazing-shell-gpui/smoke/navigation_windows.sh "$SOURCE" "$SHELL_TARGET/debug/amazing-shell" "$WINDOW_OUT" after "$PROOF_LANE"
python3 - "$GATE_OUT" "$SOURCE" "$WINDOW_OUT" <<'PY'
import hashlib,json,re,sys
from pathlib import Path
root=Path(sys.argv[1]);windows=Path(sys.argv[3]);files=[p for owner in [root,windows] for p in owner.rglob('*') if p.is_file()]
raw={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
controls=[json.loads(p.read_text()) for p in windows.rglob('actions/compact.json')]
counts={}
for name in ['workspace','shell-tests']:
 text=(root/(name+'.log')).read_text(errors='replace')
 matches=re.findall(r'(\d+) tests run: (\d+) passed(?:, (\d+) failed)?(?:, (\d+) skipped)?',text)
 counts[name]=[{'run':int(run),'passed':int(passed),'failed':int(failed or 0),'skipped':int(skipped or 0)} for run,passed,failed,skipped in matches]
for name in ['doctests']:
 text=(root/(name+'.log')).read_text(errors='replace')
 matches=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',text)
 counts[name]={'passed':sum(int(passed) for passed,_,_ in matches),'failed':sum(int(failed) for _,failed,_ in matches),'ignored':sum(int(ignored) for _,_,ignored in matches)}
summary={'standalone_doctests':'not applicable: binary-only crate','counts':counts,'source':sys.argv[2],'base':'a6bdb2f64735be47470716935aaa86d60a8231c1','original_corpus_base':'3d49ae6d6f42a293c28720337c8be2ef09ba7f6c','commands_exit':0,
 'action_groups':controls,'raw_sha256':raw,'baseline_comparison':None,'note':'Compare genuine BEFORE pinned action identities before readiness; root full16 still required.'}
encoded=json.dumps(summary,separators=(',',':'))+'\n'
if len(encoded.encode())>512*1024:raise ValueError('compact evidence too large')
(root/'compact.json').write_text(encoded)
PY
