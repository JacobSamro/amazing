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
# Cross-directory helper scripts pulled in by `../../<dir>/...` relative
# `<script src>`s from inside a selected DIRECTORIES tree, so a page whose
# assertions are otherwise fine doesn't error on a missing file before it
# runs any of them. Found by grepping the already-checked-out tree for
# `../../` references that resolve outside `resources`/`common` and outside
# the selected directories themselves: `shadow-dom/declarative/gethtml.html`
# and its siblings (`declarative-shadow-dom-attachment.html`,
# `-opt-in.html`, `-repeats.html`) pull `../../html/resources/common.js`;
# `html/dom/usvstring-reflection.https.html` pulls
# `../../webrtc/RTCPeerConnection-helper.js`. Extend this list if a future
# `--directory` selection surfaces another one the same way.
SHARED_RESOURCE_DIRECTORIES = [
    'resources', 'common', 'html/resources', 'html/dom',
    'trusted-types/support', 'webrtc',
]

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
parser.add_argument('--timeout-ms', type=int, default=25000,
                     help='Per-test wpt-harness deadline in ms for ordinary tests. Small headroom '
                          'above the historical 20000 default; the bulk of the corpus finishes in '
                          'well under a second, so this only matters for occasional flakiness.')
parser.add_argument('--heavy-timeout-ms', type=int, default=150000,
                     help='Per-test deadline for the known-heavy html/dom/reflection-*.html pages '
                          '(DOM-B-NEXT). These auto-generated reflection.js pages have thousands of '
                          'subtests each. Measured on the release binary (the binary this script now '
                          'copies -- see the shutil.copy2 comment above for why debug was 4-5x slower '
                          'and made this whole class of page look far more expensive than it is), '
                          'solo/serialized: 11.6s-84.2s across the 9 largest reflection-*.html pages, '
                          'worst case reflection-text.html at 84.2s. 150s is a deliberate ~1.8x margin '
                          'above that measured worst case, not a guess.')
parser.add_argument('--heavy-workers', type=int, default=1,
                     help='Concurrency for the heavy reflection-*.html pages, separate from the normal '
                          'pool (DOM-B-NEXT). These pages are CPU-bound for their entire run (dense '
                          'DOM/property-access loops), so running several at once divides each one\'s '
                          'CPU share and inflates wall time roughly by the number running concurrently. '
                          'Serializing them (the default) costs at most a few minutes total across all '
                          'of them on the release binary, in exchange for a stable, low --heavy-timeout-ms '
                          'instead of one inflated by contention with an unrelated lane\'s job on a '
                          'shared host.')
args = parser.parse_args()
if args.directory:
    DIRECTORIES = args.directory
output = Path(os.environ['OUT']) / ('dom-b-audit-' + args.label)
output.mkdir(parents=True, exist_ok=True)
# Freeze the executable before other pooled lanes build into the shared target.
binary = output / 'amazing'
if not args.reuse_binary:
    # DOM-B-NEXT: this used to copy the debug build. A debug binary is
    # dramatically slower for CPU-bound pages -- reflection-embedded.html
    # alone measured 67s in release but did not finish inside a 300s
    # deadline in debug, serialized, with no other contention -- and
    # nothing about WPT pass/fail depends on debug-only behavior (no
    # debug_assert!/overflow-check-only code path in the crates this
    # corpus exercises; root Cargo.toml sets no explicit
    # debug-assertions/overflow-checks override for either profile, so
    # release already carries Rust's normal defaults). The release profile
    # otherwise only affects speed and (via `strip = true`) symbol info,
    # neither of which changes a WPT subtest's pass/fail outcome.
    shutil.copy2(Path(os.environ['CARGO_TARGET_DIR']) / 'release/amazing', binary)
assert binary.is_file(), 'missing frozen executable'
root = Path(os.environ['OUT']) / 'dom-b-audit-wpt'
if not (root / '.git').exists():
    subprocess.run(['git', 'init', str(root)], check=True)
    subprocess.run(['git', '-C', str(root), 'remote', 'add', 'origin', 'https://github.com/web-platform-tests/wpt.git'], check=True)
    sparse_checkout(root, 'set', [*SHARED_RESOURCE_DIRECTORIES, *DIRECTORIES])
if not (root / DIRECTORIES[0]).exists():
    subprocess.run(['git', '-C', str(root), 'fetch', '--depth=1', '--filter=blob:none', 'origin', REVISION], check=True)
    subprocess.run(['git', '-C', str(root), 'checkout', '--detach', REVISION], check=True)
assert subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip() == REVISION
# Extend an existing lane-local sparse clone when a new whole area is selected.
sparse_checkout(root, 'add', [*SHARED_RESOURCE_DIRECTORIES, *DIRECTORIES])
assert all((root / directory).is_dir() for directory in DIRECTORIES), 'missing selected corpus directory'
selection = []
excluded = []
for directory in DIRECTORIES:
    for path in (root / directory).rglob('*'):
        if not path.is_file(): continue
        rel = path.relative_to(root).as_posix()
        # DOM-B-NEXT: WPT's own convention -- a `/manual/` path (e.g. every
        # wai-aria/manual/*-manual.html file) is meant for a human, not
        # testharness.js automation. These pages still reference
        # testharness.js (for logging/scaffolding) but never call done(), so
        # every one of them previously burned the full per-test deadline
        # doing nothing; there are 233 of them under wai-aria/manual alone.
        # Excluding the whole directory keeps this script's selection to
        # tests that can actually complete on their own.
        if '/manual/' in rel:
            excluded.append(rel)
            continue
        if path.name.endswith(('.any.js', '.window.js')):
            selection.append(rel)
        elif path.suffix in ('.html', '.htm', '.xhtml', '.xht'):
            if 'testharness.js' in path.read_text(errors='replace'):
                selection.append(rel)
            else:
                excluded.append(path.relative_to(root).as_posix())
selection.sort()
assert selection, 'empty corpus'
# DOM-B-NEXT: the auto-generated reflection.js pages under html/dom are
# dense enough (thousands of subtests, each touching many IDL attributes)
# to be genuinely CPU-bound for their whole run, unlike the rest of the
# corpus (typically well under a second). Route them through their own
# serialized pool with a much larger deadline instead of one timeout/
# concurrency setting sized for the other ~99% of tests -- see
# --heavy-timeout-ms/--heavy-workers above for the measurements behind this.
import re
HEAVY = re.compile(r'^html/dom/reflection-[^/]+\.html$')
heavy_selection = [t for t in selection if HEAVY.match(t)]
normal_selection = [t for t in selection if not HEAVY.match(t)]
def run_test(item, timeout_ms):
    number, test = item
    with tempfile.TemporaryDirectory(prefix='dom-b-audit-storage-') as storage:
        run = subprocess.run([str(binary), 'wpt-harness', str(root), test, '800', '600', str(timeout_ms)],
                             env={**os.environ, 'AMAZING_STORAGE_DIR': storage},
                             # Grace period above the in-process deadline so a
                             # test that legitimately runs right up to
                             # timeout_ms still gets to report its own
                             # "timeout" status and partial subtests, rather
                             # than being hard-killed by Python first.
                             capture_output=True, text=True, timeout=timeout_ms / 1000 + 30)
    (output / f'{number}.json').write_text(run.stdout)
    (output / f'{number}.stderr').write_text(run.stderr)
    result = json.loads(run.stdout)
    print(test, result['status'], result['counts'], result.get('detail'), flush=True)
    return result
results = []
with ThreadPoolExecutor(max_workers=min(4, int(os.environ.get('JOBS', '4')))) as executor:
    results.extend(executor.map(lambda item: run_test(item, args.timeout_ms), enumerate(normal_selection)))
with ThreadPoolExecutor(max_workers=args.heavy_workers) as executor:
    results.extend(executor.map(lambda item: run_test(item, args.heavy_timeout_ms),
                                 enumerate(heavy_selection, start=len(normal_selection))))
report = dict(source=args.source, platform='Linux Boat pooled dom-bindings', wpt_revision=REVISION,
              directories=DIRECTORIES, excluded_non_testharness=sorted(excluded), binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), results=results)
(output / 'results.json').write_text(json.dumps(report, separators=(',', ':')) + '\n')
print('RESULTS', output / 'results.json', flush=True)
