#!/usr/bin/env python3
"""Capture unsupervised worker stderr for every file in a frozen corpus report.

Diagnostic only: raw worker protocol is not a substitute for supervised WPT classifications.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('report'); parser.add_argument('binary'); parser.add_argument('root'); parser.add_argument('output')
args = parser.parse_args()
report = json.loads(Path(args.report).read_text())
output = Path(args.output); output.mkdir(parents=True, exist_ok=True)


def run(item):
    number, record = item
    deadline = record['timeout_ms']
    with tempfile.TemporaryDirectory(prefix='selection-worker-diagnostic-') as storage:
        environment = {**os.environ, 'AMAZING_STORAGE_DIR': storage,
                       'AMAZING_HTTP_CACHE_DIR': str(Path(storage) / 'http'), 'RUST_BACKTRACE': '1'}
        try:
            worker = subprocess.run([args.binary, 'wpt-harness-worker', args.root, record['test'], '800', '600', str(deadline)],
                                    env=environment, capture_output=True, timeout=deadline / 1000 + 30)
            code = worker.returncode; stdout = worker.stdout; stderr = worker.stderr
        except subprocess.TimeoutExpired as error:
            code = 'timeout'; stdout = error.stdout or b''; stderr = error.stderr or b''
    (output / f'{number}.stdout').write_bytes(stdout)
    (output / f'{number}.stderr').write_bytes(stderr)
    return {'test': record['test'], 'exit': code, 'stderr': str(number) + '.stderr'}


with ThreadPoolExecutor(max_workers=4) as pool:
    records = list(pool.map(run, enumerate(report['results'])))
(output / 'index.json').write_text(json.dumps({'source': report['source'], 'binary_sha256': report['binary_sha256'],
                                             'diagnostic_only': True, 'results': records}, indent=2) + '\n')
from collections import Counter
print(json.dumps(dict(Counter(str(record['exit']) for record in records)), sort_keys=True))
