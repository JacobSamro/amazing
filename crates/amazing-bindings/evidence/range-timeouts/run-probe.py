"""Run only on a remote build host. Curated diagnostic, not upstream WPT."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / 'corpora/wpt'
binary = Path(os.environ['CARGO_TARGET_DIR']) / 'debug' / ('amazing.exe' if os.name == 'nt' else 'amazing')
source = Path(__file__).with_name('probe.html')
with tempfile.TemporaryDirectory(prefix='dom-range-probe-') as scratch:
    scratch_root = Path(scratch)
    shutil.copyfile(root / 'resources/testharness.js', scratch_root / 'scaffold-harness.js')
    for resource in ['dom/common.js', 'dom/ranges/Range-test-iframe.html']:
        target = scratch_root / resource
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(root / resource, target)
    fixture = Path(scratch) / 'probe.html'
    shutil.copyfile(source, fixture)
    with tempfile.TemporaryDirectory(prefix='dom-range-storage-') as storage:
        result = subprocess.run([str(binary), 'wpt-harness', '--scaffold', str(scratch_root),
                                 str(fixture.relative_to(scratch_root)), '800', '600', '3000'],
                                env={**os.environ, 'AMAZING_STORAGE_DIR': storage},
                                capture_output=True, text=True, timeout=30)
    output = Path(sys.argv[2]) if len(sys.argv) > 2 else Path.home() / 'amazing/out/range-probe.json'
    output.write_text(result.stdout)
    report = json.loads(result.stdout)
    print(json.dumps(report, indent=2))
