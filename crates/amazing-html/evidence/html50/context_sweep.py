"""HTML context adoption through the complete pinned DOM node selection."""
import concurrent.futures, hashlib, importlib.util, json, os, pathlib, subprocess, sys, tempfile

root, exe, output = map(pathlib.Path, sys.argv[1:4])
jobs = int(sys.argv[4]) if len(sys.argv) > 4 else 6
spec = importlib.util.spec_from_file_location('measure_wpt_dom', pathlib.Path('crates/amazing/src/cmd/measure-wpt-dom.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
suite = sys.argv[5] if len(sys.argv) > 5 else 'dom/nodes'
entries = module.inventory(root, suite)['selected']
if not entries:
    raise SystemExit(f'empty WPT selection: {suite}')


def one(entry):
    deadline = 60000 if entry['long_timeout'] else 25000
    if entry['unsupported']:
        return module.unsupported(entry, deadline)
    with tempfile.TemporaryDirectory(prefix='amazing-html50-context-') as storage:
        env = os.environ.copy()
        env['AMAZING_STORAGE_DIR'] = storage
        try:
            proc = subprocess.run([str(exe), 'wpt-harness', str(root), entry['test'], '800', '600', str(deadline)],
                                  capture_output=True, text=True, encoding='utf-8', errors='replace',
                                  timeout=deadline / 1000 + 30, env=env)
        except subprocess.TimeoutExpired:
            return dict(schema=1, suite='wpt', wpt_revision=module.REVISION, test=entry['test'],
                        viewport=[800, 600], timeout_ms=deadline, status='timeout', harness_status=None,
                        detail='process timeout', file_counts={s: int(s == 'timeout') for s in module.FILE_STATUSES},
                        counts={s: 0 for s in module.SUBTEST_STATUSES}, subtests=[])
    try:
        result = json.loads(proc.stdout)
        assert result['wpt_revision'] == module.REVISION and result['test'] == entry['test']
        assert (proc.returncode == 0) == (result['status'] == 'pass')
        assert sum(result['counts'].values()) == len(result['subtests'])
        return result
    except Exception as error:
        return dict(schema=1, suite='wpt', wpt_revision=module.REVISION, test=entry['test'],
                    viewport=[800, 600], timeout_ms=deadline, status='error', harness_status=None,
                    detail=f'{error}; stdout={proc.stdout[-500:]!r}; stderr={proc.stderr[-500:]!r}',
                    file_counts={s: int(s == 'error') for s in module.FILE_STATUSES},
                    counts={s: 0 for s in module.SUBTEST_STATUSES}, subtests=[])


with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
    results = list(pool.map(one, entries))
raw = b''.join(module.encoded(result) for result in results)
output.parent.mkdir(parents=True, exist_ok=True)
output.write_bytes(raw)
summary = dict(revision=module.REVISION, suite=suite, selected_total=len(entries),
               subtests={s: sum(r['counts'][s] for r in results) for s in module.SUBTEST_STATUSES},
               files={s: sum(r['file_counts'][s] for r in results) for s in module.FILE_STATUSES},
               raw_sha256=hashlib.sha256(raw).hexdigest())
print(json.dumps(summary))
