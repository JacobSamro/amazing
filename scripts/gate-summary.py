#!/usr/bin/env python3
"""Small gate receipts; full logs/results remain on the build host."""
import argparse
import base64
import collections
import hashlib
import json
import pathlib
import re
import zlib

LIMIT = 512 * 1024


def write(path, value):
    data = (json.dumps(value, separators=(',', ':'), sort_keys=True) + '\n').encode()
    if len(data) > LIMIT:
        raise ValueError('compact evidence exceeds 512 KiB; leave raw on host')
    pathlib.Path(path).write_bytes(data)


def raw(path):
    p = pathlib.Path(path)
    return {'path': str(p.resolve()), 'bytes': p.stat().st_size,
            'sha256': hashlib.sha256(p.read_bytes()).hexdigest()}


def unpack(report):
    if 'compact_records' in report:
        return json.loads(zlib.decompress(base64.b64decode(report['compact_records'])))
    return report['records']


def compact(source, destination, exit_code, commit):
    report = json.loads(pathlib.Path(source).read_text())
    records = [{'path': r['path'], 'status': r['status'],
                'reason': r.get('reason', ''),
                'variants': [{'variant': v['variant'], 'status': v['status']}
                             for v in r['variants']]}
               for r in report['records']]
    # Reasons only matter for distinguishing shard selection from real skips.
    for r in records:
        r['reason'] = 'selection.excluded' if 'selection.excluded' in r['reason'] else ''
    paths = sorted(r['path'] for r in records)
    index, total = map(int, report['shard'].split('/'))
    records = [r for i, r in enumerate(records)
               if r['reason'] != 'selection.excluded' and (not r['path'].split('/')[-1].count('_FIXTURE') or i % total == index)]
    result = {k: report[k] for k in ('shard', 'test262_revision', 'input_sha256',
              'host_sha256', 'runner_sha256', 'timeout_seconds', 'classification_sha256')}
    result.update(schema=2, source_commit=commit, job_exit=0, discovered=len(paths),
                  paths_sha256=hashlib.sha256(json.dumps(paths, separators=(',', ':')).encode()).hexdigest(),
                  runner_exit=exit_code, phase_exits={'build': 0, 'runner': exit_code, 'packaging': 0}, raw=raw(source),
                  summary={'files': dict(collections.Counter(r['status'] for r in records))},
                  compact_records=base64.b64encode(zlib.compress(
                      json.dumps(records, separators=(',', ':')).encode(), 9)).decode())
    write(destination, result)


def log_counts(files):
    """Read nextest's final Summary and doctest result rows, including slow notes."""
    counts = {'nextest': None, 'doctests': {'passed': 0, 'failed': 0, 'ignored': 0}}
    for path in files:
        text = pathlib.Path(path).read_text(errors='replace')
        for line in text.splitlines():
            if 'Summary' in line and 'tests run:' in line:
                tail = line.split('tests run:', 1)[1]
                counts['nextest'] = {name: int(match.group(1)) if match else 0
                    for name in ('passed', 'failed', 'skipped')
                    for match in [re.search(r'(\d+)\s+' + name + r'\b', tail)]}
            match = re.search(r'^test result: .*? (\d+) passed; (\d+) failed; (\d+) ignored;', line)
            if match:
                for key, value in zip(('passed', 'failed', 'ignored'), match.groups()):
                    counts['doctests'][key] += int(value)
    return counts


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='command', required=True)
    t = sub.add_parser('test262')
    t.add_argument('--source-commit', default=''); t.add_argument('source'); t.add_argument('destination'); t.add_argument('exit', type=int)
    f = sub.add_parser('failure')
    f.add_argument('--source-commit', required=True); f.add_argument('--shard', required=True)
    f.add_argument('destination'); f.add_argument('phase'); f.add_argument('exit', type=int)
    f.add_argument('files', nargs='*')
    m = sub.add_parser('manifest')
    m.add_argument('--source-commit', default=''); m.add_argument('--phase', action='append', default=[]); m.add_argument('destination'); m.add_argument('exit', type=int); m.add_argument('files', nargs='+')
    args = parser.parse_args()
    if args.command == 'test262':
        if args.exit not in (0, 1):
            raise ValueError('runner did not complete normally')
        compact(args.source, args.destination, args.exit, args.source_commit)
    elif args.command == 'failure':
        write(args.destination, {'schema': 1, 'source_commit': args.source_commit,
                                'shard': args.shard, 'phase': args.phase, 'actual_exit': args.exit,
                                'raw': [raw(p) if pathlib.Path(p).is_file() else
                                        {'path': str(pathlib.Path(p).resolve()), 'missing': True}
                                        for p in args.files]})
    else:
        write(args.destination, {'schema': 1, 'source_commit': args.source_commit, 'job_exit': args.exit,
                                'phase_exits': {name: int(code) for name, code in (p.split('=', 1) for p in args.phase)},
                                'counts': log_counts(args.files),
                                'summary_lines': [line.strip() for p in args.files
                                    for line in pathlib.Path(p).read_text(errors='replace').splitlines()
                                    if 'Summary' in line or line.startswith('test result:')],
                                'raw': [raw(p) for p in args.files]})


if __name__ == '__main__':
    main()
