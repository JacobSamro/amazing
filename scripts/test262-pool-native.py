#!/usr/bin/env python3
"""Build once, execute original Test262 shards sequentially; raw stays native.

This optional backend changes placement only. The production runner, compact
writer and merge checker retain ownership and zero-loss decisions.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, row):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + '.partial')
    tmp.write_text(json.dumps(row, indent=2) + '\n')
    tmp.replace(path)


def partition(shards, boxes):
    if not 1 <= boxes <= shards <= 16:
        raise ValueError('require 1 <= boxes <= shards <= 16')
    return [list(range(box, shards, boxes)) for box in range(boxes)]


def execute(args, qualification_scope='engine corpus'):
    out, raw = Path(args.out), Path(args.raw)
    out.mkdir(parents=True, exist_ok=True)
    raw.mkdir(parents=True, exist_ok=True)
    scripts = Path(args.scripts)
    host = Path(os.environ['CARGO_TARGET_DIR']) / 'release/examples/test262_host'
    receipt = {'schema': 1, 'source_commit': args.source, 'shards': args.shards,
               'shard_total': args.total, 'jobs': 4, 'timeout_seconds': 60,
               'runner_sha256': digest(scripts / 'test262.py'),
               'native_source': str(Path.cwd()), 'native_target': os.environ['CARGO_TARGET_DIR'],
               'native_raw': str(raw), 'started_ns': time.time_ns(), 'phases': [], 'shard_results': [],
               'qualification_scope': qualification_scope}

    def phase(name, argv):
        phase_path = out / (name + '.phase.manifest.json')
        command = [sys.executable, str(scripts / 'gate-command.py'), '--phase', name,
                   '--source-commit', args.source, '--output', str(raw / (name + '.log')),
                   '--receipt', str(phase_path)]
        actual = subprocess.run(command + ['--'] + argv).returncode
        receipt['phases'].append({'phase': name, 'wrapper_exit': actual,
                                  'receipt': str(phase_path),
                                  'receipt_sha256': digest(phase_path) if phase_path.exists() else None})
        return actual, json.loads(phase_path.read_text()) if phase_path.exists() else {}

    actual, _ = phase('build', ['cargo', 'build', '-p', 'amazing-js', '--example',
                               'test262_host', '--release', '--locked'])
    failed = actual != 0
    if not failed:
        receipt['host_sha256'] = digest(host)
        for shard in args.shards:
            name = f'shard-{shard}'
            result_path = raw / (name + '.json')
            actual, row = phase(name + '-runner', [sys.executable,
                str(scripts / 'test262.py'), os.environ['TEST262_ROOT'], '--binary', str(host),
                '--jobs', '4', '--timeout', '60', '--shard', f'{shard}/{args.total}',
                '--output', str(result_path)])
            # Runner 1 means genuine failed tests; it still has complete classifications.
            if actual not in (0, 1) or row.get('output_copy_exit') != 0 or row.get('receipt_copy_error'):
                failed = True
                continue
            actual, _ = phase(name + '-packaging', [sys.executable,
                str(scripts / 'gate-summary.py'), 'test262', '--source-commit', args.source,
                str(result_path), str(out / (name + '.compact.json')), str(actual)])
            failed |= actual != 0
            if actual == 0:
                compact_path = out / (name + '.compact.json')
                compact = json.loads(compact_path.read_text())
                report = json.loads(result_path.read_text())
                valid = (compact['source_commit'] == args.source and compact['shard'] == f'{shard}/{args.total}'
                         and compact['host_sha256'] == receipt['host_sha256']
                         and compact['runner_sha256'] == receipt['runner_sha256']
                         and report['jobs'] == 4 and compact['timeout_seconds'] == 60
                         and compact['raw']['path'] == str(result_path.resolve())
                         and compact['raw']['sha256'] == digest(result_path))
                receipt['shard_results'].append({'shard': f'{shard}/{args.total}',
                    'compact_path': str(compact_path), 'compact_sha256': digest(compact_path),
                    'raw': compact['raw'], 'profile_verified': valid})
                failed |= not valid
    receipt.update(finished_ns=time.time_ns(), actual_exit=3 if failed else 0)
    write(out / 'pool.summary.json', receipt)
    return receipt['actual_exit']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', required=True)
    parser.add_argument('--total', type=int, required=True)
    parser.add_argument('--shards', type=int, nargs='+', required=True)
    parser.add_argument('--scripts', required=True)
    parser.add_argument('--out', required=True)
    parser.add_argument('--raw', required=True)
    args = parser.parse_args()
    if not 1 <= args.total <= 16 or not args.shards or len(set(args.shards)) != len(args.shards) or any(not 0 <= k < args.total for k in args.shards):
        parser.error('invalid original shard inventory')
    return execute(args)


if __name__ == '__main__':
    raise SystemExit(main())
