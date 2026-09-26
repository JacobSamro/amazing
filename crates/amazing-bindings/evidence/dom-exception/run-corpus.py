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

def sparse_checkout(root, mode, paths):
    """`git sparse-checkout set/add`, but never on a checkout this script did
    not create itself. `root` below is always a private clone freshly made
    under $OUT; guard anyway in case it is ever pointed at a shared checkout
    such as ~/corpora/wpt on a pooled Boat sandbox -- narrowing that shared
    tree hid other lanes' directories on 2026-09-26. See scripts/ensure-wpt.sh."""
    if root.resolve() == (Path.home() / 'corpora' / 'wpt').resolve():
        return
    subprocess.run(['git', '-C', str(root), 'sparse-checkout', mode, *paths], check=True)

parser = argparse.ArgumentParser()
parser.add_argument('label')
parser.add_argument('source')
parser.add_argument('--reuse-binary', action='store_true', help='Use the frozen executable from an interrupted corpus run')
args = parser.parse_args()
output = Path(os.environ['OUT']) / ('dom-exception-' + args.label)
output.mkdir(parents=True, exist_ok=True)
# Freeze the executable before other pooled lanes build into the shared target.
binary = output / 'amazing'
if not args.reuse_binary:
    shutil.copy2(Path(os.environ['CARGO_TARGET_DIR']) / 'debug/amazing', binary)
assert binary.is_file(), 'missing frozen executable'
root = Path(os.environ['OUT']) / 'dom-exception-wpt'
if not (root / '.git').exists():
    subprocess.run(['git', 'init', str(root)], check=True)
    subprocess.run(['git', '-C', str(root), 'remote', 'add', 'origin', 'https://github.com/web-platform-tests/wpt.git'], check=True)
    sparse_checkout(root, 'set', ['resources', DIRECTORY])
if not (root / DIRECTORY).exists():
    subprocess.run(['git', '-C', str(root), 'fetch', '--depth=1', '--filter=blob:none', 'origin', REVISION], check=True)
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
