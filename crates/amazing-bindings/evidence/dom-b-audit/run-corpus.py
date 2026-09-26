"""Run the pinned node, traversal, event and collection testharness directories on Linux, with lane-local outputs."""
from concurrent.futures import ThreadPoolExecutor
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

REVISION = 'c2a2fce69e5c2509d9dc28440ec8b42532e5c293'
DIRECTORIES = ['dom/nodes', 'dom/traversal', 'dom/events', 'dom/collections']

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
parser.add_argument('--directory', action='append', help='Run whole selected directories; defaults to all four DOM-B areas')
args = parser.parse_args()
if args.directory:
    DIRECTORIES = args.directory
output = Path(os.environ['OUT']) / ('dom-b-audit-' + args.label)
output.mkdir(parents=True, exist_ok=True)
# Freeze the executable before other pooled lanes build into the shared target.
binary = output / 'amazing'
if not args.reuse_binary:
    shutil.copy2(Path(os.environ['CARGO_TARGET_DIR']) / 'debug/amazing', binary)
assert binary.is_file(), 'missing frozen executable'
root = Path(os.environ['OUT']) / 'dom-b-audit-wpt'
if not (root / '.git').exists():
    subprocess.run(['git', 'init', str(root)], check=True)
    subprocess.run(['git', '-C', str(root), 'remote', 'add', 'origin', 'https://github.com/web-platform-tests/wpt.git'], check=True)
    sparse_checkout(root, 'set', ['resources', 'common', *DIRECTORIES])
if not (root / DIRECTORIES[0]).exists():
    subprocess.run(['git', '-C', str(root), 'fetch', '--depth=1', '--filter=blob:none', 'origin', REVISION], check=True)
    subprocess.run(['git', '-C', str(root), 'checkout', '--detach', REVISION], check=True)
assert subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip() == REVISION
# Extend an existing lane-local sparse clone when a new whole area is selected.
sparse_checkout(root, 'add', ['resources', 'common', *DIRECTORIES])
assert all((root / directory).is_dir() for directory in DIRECTORIES), 'missing selected corpus directory'
selection = []
excluded = []
for directory in DIRECTORIES:
    for path in (root / directory).rglob('*'):
        if not path.is_file(): continue
        if path.name.endswith(('.any.js', '.window.js')):
            selection.append(path.relative_to(root).as_posix())
        elif path.suffix in ('.html', '.htm', '.xhtml', '.xht'):
            if 'testharness.js' in path.read_text(errors='replace'):
                selection.append(path.relative_to(root).as_posix())
            else:
                excluded.append(path.relative_to(root).as_posix())
selection.sort()
assert selection, 'empty corpus'
def run_test(item):
    number, test = item
    with tempfile.TemporaryDirectory(prefix='dom-b-audit-storage-') as storage:
        run = subprocess.run([str(binary), 'wpt-harness', str(root), test, '800', '600', '20000'],
                             env={**os.environ, 'AMAZING_STORAGE_DIR': storage},
                             capture_output=True, text=True, timeout=45)
    (output / f'{number}.json').write_text(run.stdout)
    (output / f'{number}.stderr').write_text(run.stderr)
    result = json.loads(run.stdout)
    print(test, result['status'], result['counts'], result.get('detail'), flush=True)
    return result
with ThreadPoolExecutor(max_workers=min(4, int(os.environ.get('JOBS', '4')))) as executor:
    results = list(executor.map(run_test, enumerate(selection)))
report = dict(source=args.source, platform='Linux Boat pooled dom-bindings', wpt_revision=REVISION,
              directories=DIRECTORIES, excluded_non_testharness=sorted(excluded), binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), results=results)
(output / 'results.json').write_text(json.dumps(report, separators=(',', ':')) + '\n')
print('RESULTS', output / 'results.json', flush=True)
