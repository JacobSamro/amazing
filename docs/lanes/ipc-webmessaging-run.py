#!/usr/bin/env python3
"""Run the pinned WPT webmessaging/ selection through `amazing wpt-harness`.

95 .html sources under webmessaging/ at the pinned revision. No pre-filtering:
every file is handed to the harness and classified by its own pass/fail/
timeout/error verdict, the same result schema as measure-wpt-dom.py's `run()`.
"""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path

REVISION = 'c2a2fce69e5c2509d9dc28440ec8b42532e5c293'
FILE_STATUSES = ('pass', 'fail', 'timeout', 'error', 'unsupported')
SUBTEST_STATUSES = ('pass', 'fail', 'timeout', 'notrun', 'error')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(value):
    return (json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(',', ':')) + '\n').encode()


def git(root, *args):
    return subprocess.check_output(['git', '-C', str(root), *args])


def run_one(exe, root, test, timeout_ms):
    try:
        proc = subprocess.run(
            [str(exe), 'wpt-harness', str(root), test, '800', '600', str(timeout_ms)],
            capture_output=True, encoding='utf-8', errors='replace',
            timeout=timeout_ms / 1000 + 30,
        )
    except subprocess.TimeoutExpired:
        return {'schema': 1, 'suite': 'wpt', 'wpt_revision': REVISION, 'test': test,
                'viewport': [800, 600], 'timeout_ms': timeout_ms, 'status': 'timeout',
                'harness_status': None, 'detail': 'process did not exit within wrapper deadline',
                'file_counts': {s: int(s == 'timeout') for s in FILE_STATUSES},
                'counts': {s: 0 for s in SUBTEST_STATUSES}, 'subtests': []}
    try:
        result = json.loads(proc.stdout)
        if result.get('suite') != 'wpt' or result.get('wpt_revision') != REVISION:
            raise ValueError('missing provenance')
        if result['test'] != test or result['status'] not in FILE_STATUSES:
            raise ValueError('invalid identity/classification')
    except Exception as error:  # noqa: BLE001 - any parse/shape failure is an `error` verdict
        return {'schema': 1, 'suite': 'wpt', 'wpt_revision': REVISION, 'test': test,
                'viewport': [800, 600], 'timeout_ms': timeout_ms, 'status': 'error',
                'harness_status': None,
                'detail': f'{error}: exit={proc.returncode} stdout={proc.stdout[:500]!r} stderr={proc.stderr[:500]!r}',
                'file_counts': {s: int(s == 'error') for s in FILE_STATUSES},
                'counts': {s: 0 for s in SUBTEST_STATUSES}, 'subtests': []}
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--executable', type=Path, required=True)
    ap.add_argument('--wpt-root', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--timeout-ms', type=int, default=10000)
    args = ap.parse_args()
    root, exe, output = args.wpt_root.resolve(), args.executable.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    head = git(root, 'rev-parse', 'HEAD').decode().strip()
    if head != REVISION:
        raise RuntimeError(f'Wrong WPT revision: {head}')
    files = sorted(
        str(p.relative_to(root)).replace('\\', '/')
        for p in (root / 'webmessaging').rglob('*.html')
    )
    if len(files) != 95:
        raise RuntimeError(f'Expected 95 webmessaging/ .html sources, found {len(files)}')
    records = []
    for i, test in enumerate(files, 1):
        record = run_one(exe, root, test, args.timeout_ms)
        records.append(record)
        (output / (Path(test).name + '.json')).write_bytes(encoded(record))
        if i % 20 == 0 or i == len(files):
            print(f'webmessaging: {i}/{len(files)}', flush=True)
    summary = {
        'wpt_revision': REVISION,
        'selected_source_files': len(files),
        'files': {s: sum(r['file_counts'][s] for r in records) for s in FILE_STATUSES},
        'subtests': {s: sum(r['counts'][s] for r in records) for s in SUBTEST_STATUSES},
        'raw_sha256': digest(b''.join(encoded(r) for r in records)),
    }
    (output / 'summary.json').write_bytes(encoded(summary))
    print(json.dumps(summary), flush=True)


if __name__ == '__main__':
    main()
