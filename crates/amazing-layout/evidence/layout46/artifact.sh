#!/usr/bin/env bash
# Additional production-rendered artifact; reuses the exact completed gate binary.
set -euo pipefail
RESULT=$OUT/layout46/artifact
mkdir -p "$RESULT"
cp crates/amazing-layout/evidence/layout46/artifact/* "$RESULT/"
EXE=/tmp/amazing-layout36-target/release/amazing
sha256sum "$EXE" > "$RESULT/renderer.sha256"
"$EXE" render-wpt "$RESULT/actual.html" "$RESULT/actual.ppm" 60 60 > "$RESULT/actual.stdout.txt" 2> "$RESULT/actual.stderr.txt"
"$EXE" render-wpt "$RESULT/reference.html" "$RESULT/reference.ppm" 60 60 > "$RESULT/reference.stdout.txt" 2> "$RESULT/reference.stderr.txt"
python3 - "$RESULT" <<'PY'
import sys,json,importlib.util
from pathlib import Path
spec=importlib.util.spec_from_file_location('c',Path('scripts/wpt_render_compare.py'));c=importlib.util.module_from_spec(spec);spec.loader.exec_module(c)
r=Path(sys.argv[1]);a=c.read_ppm(r/'actual.ppm');b=c.read_ppm(r/'reference.ppm');assert a==b,'production decoded-PPM nine-piece artifact differs'
assert not (r/'actual.stderr.txt').read_text().strip(),'production resource warning'
(r/'summary.json').write_text(json.dumps({'production_ppm_artifact':'pass','viewport':[60,60],'renderer':(r/'renderer.sha256').read_text().split()[0]},indent=2))
print('Production decoded-PPM nine-piece artifact PASS')
PY
convert "$RESULT/actual.ppm" "$RESULT/actual.png"
