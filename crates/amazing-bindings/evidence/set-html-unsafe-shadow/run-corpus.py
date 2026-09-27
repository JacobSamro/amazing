"""Run the pinned shadow-dom/declarative testharness directory on Linux, with
lane-local outputs. Adapted from
crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py (same pinned WPT
revision, same freeze-the-binary/sparse-checkout/thread-pool shape)."""
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
DIRECTORIES = ['shadow-dom/declarative']
# `shadow-dom/declarative/gethtml.html` and its siblings pull in
# `../../html/resources/common.js` via a relative `<script src>` -- see
# dom-b-audit/run-corpus.py's own comment on the same issue.
SHARED_RESOURCE_DIRECTORIES = ['resources', 'html/resources']

def sparse_checkout(root, mode, paths):
    if root.resolve() == (Path.home() / 'corpora' / 'wpt').resolve():
        return
    subprocess.run(['git', '-C', str(root), 'sparse-checkout', mode, *paths], check=True)

parser = argparse.ArgumentParser()
parser.add_argument('label')
parser.add_argument('source')
parser.add_argument('--reuse-binary', action='store_true')
parser.add_argument('--timeout-ms', type=int, default=25000)
args = parser.parse_args()
output = Path(os.environ['OUT']) / ('set-html-unsafe-shadow-' + args.label)
output.mkdir(parents=True, exist_ok=True)
binary = output / 'amazing'
if not args.reuse_binary:
    shutil.copy2(Path(os.environ['CARGO_TARGET_DIR']) / 'release/amazing', binary)
assert binary.is_file(), 'missing frozen executable'
root = Path(os.environ['OUT']) / 'set-html-unsafe-shadow-wpt'
if not (root / '.git').exists():
    subprocess.run(['git', 'init', str(root)], check=True)
    subprocess.run(['git', '-C', str(root), 'remote', 'add', 'origin', 'https://github.com/web-platform-tests/wpt.git'], check=True)
    sparse_checkout(root, 'set', [*SHARED_RESOURCE_DIRECTORIES, *DIRECTORIES])
if not (root / DIRECTORIES[0]).exists():
    subprocess.run(['git', '-C', str(root), 'fetch', '--depth=1', '--filter=blob:none', 'origin', REVISION], check=True)
    subprocess.run(['git', '-C', str(root), 'checkout', '--detach', REVISION], check=True)
assert subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip() == REVISION
sparse_checkout(root, 'add', [*SHARED_RESOURCE_DIRECTORIES, *DIRECTORIES])
assert all((root / directory).is_dir() for directory in DIRECTORIES), 'missing selected corpus directory'
selection = []
excluded = []
for directory in DIRECTORIES:
    for path in (root / directory).rglob('*'):
        if not path.is_file(): continue
        rel = path.relative_to(root).as_posix()
        if '/manual/' in rel:
            excluded.append(rel)
            continue
        if path.name.endswith(('.any.js', '.window.js')):
            selection.append(rel)
        elif path.suffix in ('.html', '.htm', '.xhtml', '.xht'):
            if 'testharness.js' in path.read_text(errors='replace'):
                selection.append(rel)
            else:
                excluded.append(rel)
selection.sort()
assert selection, 'empty corpus'
def run_test(item):
    number, test = item
    with tempfile.TemporaryDirectory(prefix='set-html-unsafe-shadow-storage-') as storage:
        run = subprocess.run([str(binary), 'wpt-harness', str(root), test, '800', '600', str(args.timeout_ms)],
                             env={**os.environ, 'AMAZING_STORAGE_DIR': storage},
                             capture_output=True, text=True, timeout=args.timeout_ms / 1000 + 30)
    (output / f'{number}.json').write_text(run.stdout)
    (output / f'{number}.stderr').write_text(run.stderr)
    result = json.loads(run.stdout)
    print(test, result['status'], result['counts'], result.get('detail'), flush=True)
    return result
with ThreadPoolExecutor(max_workers=min(4, int(os.environ.get('JOBS', '4')))) as executor:
    results = list(executor.map(run_test, enumerate(selection)))
report = dict(source=args.source, platform='Linux venus-001 dom-bindings', wpt_revision=REVISION,
              directories=DIRECTORIES, excluded_non_testharness=sorted(excluded),
              binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), results=results)
(output / 'results.json').write_text(json.dumps(report, separators=(',', ':')) + '\n')
print('RESULTS', output / 'results.json', flush=True)
