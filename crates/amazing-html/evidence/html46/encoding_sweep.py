import concurrent.futures, hashlib, importlib.util, json, os, pathlib, re, subprocess, sys, tempfile

root = pathlib.Path(sys.argv[1])
exe = pathlib.Path(sys.argv[2])
output = pathlib.Path(sys.argv[3])
jobs = int(sys.argv[4]) if len(sys.argv) > 4 else 6

spec = importlib.util.spec_from_file_location('measure_wpt_dom', pathlib.Path('crates/amazing/src/cmd/measure-wpt-dom.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

# HTML-41 (coordinator instruction: WPT encoding/ as the gate). This lane owns
# encoding/ per AGENTS.md's gate row ("amazing-html, amazing-encoding ... WPT
# encoding/"). measure-wpt-dom.py's own inventory() hardcodes its .any.js/
# .window.js "generated window global" exception to suite == 'url' only;
# encoding/ is almost entirely .any.js, so this sweep does its own selection
# for that suite (mirroring the 'url' exception, carried over verbatim from
# the prior HTML-37 sweep script, .agents/html/html37_sweep.py, which is
# scratch and was never committed) instead of editing that shared,
# other-lanes'-owned script.

entries = []


def inventory_encoding(root, suite='encoding'):
    marker_pinned = module.checkout_is_marker_pinned(root)
    selected = []
    for name in module.list_tree(root, suite, marker_pinned):
        parts = pathlib.Path(name).parts
        if any(p in ('resources', 'support') for p in parts):
            continue
        suffix = pathlib.Path(name).suffix
        generated = re.search(r'\.(any|window|worker)\.js$', name)
        if suffix not in ('.html', '.htm', '.xhtml', '.xht', '.svg') and not generated:
            continue
        data = (root / name).read_bytes()
        if not marker_pinned:
            original = module.git(root, 'show', module.REVISION + ':' + name)
            if data != original:
                raise RuntimeError('Modified upstream source: ' + name)
        page = module.Page(data.decode('utf-8-sig', errors='replace'))
        any_window = bool(name.endswith('.any.js'))
        if generated and not any_window:
            reason = 'worker-only generated JavaScript test globals are unsupported'
        elif not generated and not page.harness:
            reason = 'no external testharness script (e.g. crash/reference/support)'
        elif any_window and re.search(rb'^//\s*META:\s*global=(.*)$', data, re.M) and not re.search(rb'^//\s*META:\s*global=.*\bwindow\b', data, re.M):
            reason = 'window global not selected by upstream metadata'
        elif suffix != '.html' and not any_window:
            reason = 'XML/SVG/non-HTML document parsing is unsupported'
        elif '.manual.' in name or '-manual.' in name:
            reason = 'manual interaction is unsupported'
        elif '.sub.' in name:
            reason = 'WPT server template substitution is unsupported'
        elif '.https.' in name:
            reason = 'HTTPS test origin is unsupported by the local runner'
        elif page.variants:
            reason = 'WPT URL variants are unsupported by the local source-file runner'
        else:
            reason = None
        any_long = any_window and bool(re.search(rb'^//\s*META:\s*timeout=long$', data, re.M))
        selected.append({'test': name, 'sha256': module.digest(data), 'unsupported': reason,
                         'variants': page.variants, 'long_timeout': page.long_timeout or any_long})
    return selected


entries = inventory_encoding(root, 'encoding')


def one(entry):
    if entry['unsupported']:
        return module.unsupported(entry, 60000 if entry['long_timeout'] else 10000)
    deadline = 60000 if entry['long_timeout'] else 10000
    with tempfile.TemporaryDirectory(prefix='amazing-html41-storage-') as storage:
        env = os.environ.copy()
        env['AMAZING_STORAGE_DIR'] = storage
        try:
            p = subprocess.run([str(exe), 'wpt-harness', str(root), entry['test'], '800', '600', str(deadline)],
                               capture_output=True, text=True, encoding='utf-8', errors='replace',
                               timeout=deadline / 1000 + 30, env=env)
        except subprocess.TimeoutExpired:
            return {'schema': 1, 'suite': 'wpt', 'wpt_revision': module.REVISION, 'test': entry['test'],
                    'viewport': [800, 600], 'timeout_ms': deadline, 'status': 'timeout', 'harness_status': None,
                    'detail': 'process timeout', 'file_counts': {s: int(s == 'timeout') for s in module.FILE_STATUSES},
                    'counts': {s: 0 for s in module.SUBTEST_STATUSES}, 'subtests': []}
    try:
        result = json.loads(p.stdout)
        assert result['wpt_revision'] == module.REVISION and result['test'] == entry['test'], entry['test']
        assert (p.returncode == 0) == (result['status'] == 'pass'), entry['test']
    except Exception as ex:
        return {'schema': 1, 'suite': 'wpt', 'wpt_revision': module.REVISION, 'test': entry['test'],
                'viewport': [800, 600], 'timeout_ms': deadline, 'status': 'error', 'harness_status': None,
                'detail': f'harness error: {ex}; stdout={p.stdout[-500:]!r}; stderr={p.stderr[-500:]!r}',
                'file_counts': {s: int(s == 'error') for s in module.FILE_STATUSES},
                'counts': {s: 0 for s in module.SUBTEST_STATUSES}, 'subtests': []}
    return result


with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
    results = list(pool.map(one, entries))

output.parent.mkdir(parents=True, exist_ok=True)
raw = b''.join(module.encoded(x) for x in results)
output.write_bytes(raw)
counts = {key: sum(r['counts'][key] for r in results) for key in module.SUBTEST_STATUSES}
files = {key: sum(r['file_counts'][key] for r in results) for key in module.FILE_STATUSES}
per_test_status = {r['test']: r['status'] for r in results}
(output.parent / (output.stem + '-status.json')).write_bytes(module.encoded(per_test_status))
ranking = sorted(
    ((r['test'], r['status'], sum(v for k, v in r['counts'].items() if k != 'pass')) for r in results),
    key=lambda t: -t[2],
)
(output.parent / (output.stem + '-ranking.json')).write_bytes(module.encoded(ranking[:80]))
print(json.dumps({'revision': module.REVISION, 'selected_total': len(entries),
                  'subtests': counts, 'files': files,
                  'raw_sha256': hashlib.sha256(raw).hexdigest()}))
