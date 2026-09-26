"""Run the entire pinned WebIDL exception directory on Linux, with lane-local outputs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

REVISION = 'c2a2fce69e5c2509d9dc28440ec8b42532e5c293'
DIRECTORY = 'webidl/ecmascript-binding/es-exceptions'
parser = argparse.ArgumentParser()
parser.add_argument('label')
parser.add_argument('source')
args = parser.parse_args()
output = Path(os.environ['OUT']) / ('dom-exception-' + args.label)
output.mkdir(parents=True, exist_ok=True)
# Freeze the executable before other pooled lanes build into the shared target.
binary = output / 'amazing'
shutil.copy2(Path(os.environ['CARGO_TARGET_DIR']) / 'debug/amazing', binary)
root = Path(os.environ['OUT']) / 'dom-exception-wpt'
if not (root / '.git').exists():
    subprocess.run(['git', 'clone', '--shared', '--no-checkout', str(Path.home() / 'corpora/wpt'), str(root)], check=True)
    subprocess.run(['git', '-C', str(root), 'sparse-checkout', 'set', 'resources', DIRECTORY], check=True)
    subprocess.run(['git', '-C', str(root), 'checkout', '--detach', REVISION], check=True)
assert subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip() == REVISION
selection = sorted(p.relative_to(root).as_posix() for p in (root / DIRECTORY).rglob('*')
                   if p.is_file() and (p.suffix in ('.html', '.htm') or p.name.endswith(('.any.js', '.window.js'))))
assert selection, 'empty corpus'
results = []
for number, test in enumerate(selection):
    with tempfile.TemporaryDirectory(prefix='dom-exception-storage-') as storage:
        run = subprocess.run([str(binary), 'wpt-harness', str(root), test, '800', '600', '20000'],
                             env={**os.environ, 'AMAZING_STORAGE_DIR': storage},
                             capture_output=True, text=True, timeout=45)
    (output / f'{number}.json').write_text(run.stdout)
    (output / f'{number}.stderr').write_text(run.stderr)
    result = json.loads(run.stdout)
    results.append(result)
    print(test, result['status'], result['counts'], result.get('detail'), flush=True)
report = dict(source=args.source, platform='Linux Boat pooled dom-bindings', wpt_revision=REVISION,
              directory=DIRECTORY, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), results=results)
(output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
print('RESULTS', output / 'results.json', flush=True)
