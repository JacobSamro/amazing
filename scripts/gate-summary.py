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
TICKET_LIMIT = 2 * 1024 * 1024
STATUSES = {"pass", "fail", "timeout", "unsupported", "skipped"}


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


def shard_selection(shard):
    # An unsharded directory report is not a numbered merge shard. Keep its
    # original spelling in the receipt while selecting all fixture records.
    if shard == '':
        return 0, 1
    if not isinstance(shard, str) or not re.fullmatch(r'\d+/[1-9]\d*', shard):
        raise ValueError('invalid shard identity')
    index, total = map(int, shard.split('/'))
    if index >= total:
        raise ValueError('invalid shard index')
    return index, total


def validate_records(records):
    seen = set()
    for record in records:
        path = record['path']
        if (not isinstance(path, str) or not path or path.startswith('/')
                or '..' in path.split('/') or path in seen):
            raise ValueError('invalid or duplicate test path')
        seen.add(path)
        if record['status'] not in STATUSES:
            raise ValueError('invalid file classification')
        variants = set()
        for variant in record['variants']:
            name = variant['variant']
            if not isinstance(name, str) or not name or name in variants:
                raise ValueError('invalid or duplicate variant identity')
            variants.add(name)
            if variant['status'] not in STATUSES:
                raise ValueError('invalid variant classification')


def compact(source, destination, exit_code, commit):
    if exit_code not in (0, 1):
        raise ValueError('runner did not complete normally')
    report = json.loads(pathlib.Path(source).read_text())
    validate_records(report['records'])
    index, total = shard_selection(report['shard'])
    records = [{'path': r['path'], 'status': r['status'],
                'reason': r.get('reason', ''),
                'variants': [{'variant': v['variant'], 'status': v['status']}
                             for v in r['variants']]}
               for r in report['records']]
    # Reasons only matter for distinguishing shard selection from real skips.
    for r in records:
        r['reason'] = 'selection.excluded' if 'selection.excluded' in r['reason'] else ''
    paths = sorted(r['path'] for r in records)
    records = [r for i, r in enumerate(records)
               if r['reason'] != 'selection.excluded' and (not r['path'].split('/')[-1].count('_FIXTURE') or i % total == index)]
    result = {k: report[k] for k in ('shard', 'test262_revision', 'input_sha256',
              'host_sha256', 'runner_sha256', 'timeout_seconds', 'classification_sha256')}
    result.update(selection={k: report.get(k) for k in ('filter', 'limit', 'jobs')}, schema=2, source_commit=commit, job_exit=0, discovered=len(paths),
                  paths_sha256=hashlib.sha256(json.dumps(paths, separators=(',', ':')).encode()).hexdigest(),
                  runner_exit=exit_code, phase_exits={'build': 0, 'runner': exit_code, 'packaging': 0}, raw=raw(source),
                  summary={'files': dict(collections.Counter(r['status'] for r in records))},
                  compact_records=base64.b64encode(zlib.compress(
                      json.dumps(records, separators=(',', ':')).encode(), 9)).decode())
    write(destination, result)


def bundle(index_path, destination):
    """Lossless multi-selection storage; never discard tests to meet a budget.

    Index rows explicitly identify role/selector/mode and an existing compact.
    Whole compression removes overlap between directory selections. Chunking is
    storage only: each report and its raw/source provenance survives unchanged.
    """
    rows = json.loads(pathlib.Path(index_path).read_text())
    if not isinstance(rows, list) or not rows:
        raise ValueError('empty or malformed bundle index')
    identities, reports, record_sets, set_ids = set(), [], [], {}
    for row in rows:
        identity = (row['role'], row['selector'], row['mode'])
        if (row['role'] not in ('before', 'after') or row['mode'] not in ('default', 'vm')
                or not isinstance(row['selector'], str) or not row['selector']
                or identity in identities):
            raise ValueError('invalid or duplicate selection identity')
        identities.add(identity)
        report = json.loads(pathlib.Path(row['compact']).read_text())
        if report.get('schema') != 2 or report.get('job_exit') != 0 or report.get('runner_exit') not in (0, 1):
            raise ValueError('not a completed Test262 compact')
        shard_selection(report['shard'])
        records = unpack(report)
        validate_records(records)
        report = dict(report)
        report.pop('compact_records', None)
        # Direct records and compact_records are alternate encodings. Keep the
        # complete decoded set once, rather than repeating direct records in
        # every receipt as well as the deduplicated record_sets table.
        report.pop('records', None)
        record_bytes = json.dumps(records, separators=(',', ':'), sort_keys=True)
        if record_bytes not in set_ids:
            set_ids[record_bytes] = len(record_sets)
            record_sets.append(records)
        report['record_set'] = set_ids[record_bytes]
        reports.append({'identity': list(identity), 'receipt': report,
                        'compact_raw': raw(row['compact'])})
    payload = json.dumps({'reports': reports, 'record_sets': record_sets},
                         separators=(',', ':'), sort_keys=True).encode()
    encoded = base64.b64encode(zlib.compress(payload, 9)).decode()
    destination = pathlib.Path(destination)
    parts = []
    outputs = []
    for number, start in enumerate(range(0, len(encoded), 256 * 1024)):
        path = destination.with_name(destination.name + '.part-' + str(number) + '.compact.json')
        data = (json.dumps({'schema': 1, 'part': number, 'data': encoded[start:start + 256 * 1024]},
                           separators=(',', ':')) + '\n').encode()
        parts.append({'name': path.name, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
        outputs.append((path, data))
    manifest = {'schema': 1, 'encoding': 'base64-zlib-json', 'identities': sorted(map(list, identities)),
                'reports': len(reports), 'uncompressed_bytes': len(payload),
                'uncompressed_sha256': hashlib.sha256(payload).hexdigest(), 'parts': parts}
    manifest_data = (json.dumps(manifest, separators=(',', ':'), sort_keys=True) + '\n').encode()
    if (len(manifest_data) > LIMIT or any(len(data) > LIMIT for _, data in outputs)
            or len(manifest_data) + sum(len(data) for _, data in outputs) > TICKET_LIMIT):
        raise ValueError('lossless bundle exceeds ticket budget; no files written')
    if destination.exists() or any(path.exists() for path, _ in outputs):
        raise ValueError('bundle destination already exists')
    for path, data in outputs:
        path.write_bytes(data)
    destination.write_bytes(manifest_data)


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
    b = sub.add_parser('bundle')
    b.add_argument('index'); b.add_argument('destination')
    f = sub.add_parser('failure')
    f.add_argument('--source-commit', required=True); f.add_argument('--shard', required=True)
    f.add_argument('--phase-receipt')
    f.add_argument('--wrapper-exit', type=int, default=None)
    f.add_argument('destination'); f.add_argument('phase'); f.add_argument('exit', type=int)
    f.add_argument('files', nargs='*')
    m = sub.add_parser('manifest')
    m.add_argument('--phase-receipt', action='append', default=[])
    m.add_argument('--source-commit', default=''); m.add_argument('--phase', action='append', default=[]); m.add_argument('destination'); m.add_argument('exit', type=int); m.add_argument('files', nargs='+')
    args = parser.parse_args()
    if args.command == 'test262':
        if args.exit not in (0, 1):
            raise ValueError('runner did not complete normally')
        compact(args.source, args.destination, args.exit, args.source_commit)
    elif args.command == 'bundle':
        bundle(args.index, args.destination)
    elif args.command == 'failure':
        write(args.destination, {'schema': 1, 'source_commit': args.source_commit,
                                'shard': args.shard, 'phase': args.phase, 'actual_exit': args.exit,
                                'wrapper_exit': args.wrapper_exit,
                                'phase_detail': json.loads(pathlib.Path(args.phase_receipt).read_text()) if args.phase_receipt and pathlib.Path(args.phase_receipt).is_file() else None,
                                'raw': [raw(p) if pathlib.Path(p).is_file() else
                                        {'path': str(pathlib.Path(p).resolve()), 'missing': True}
                                        for p in args.files]})
    else:
        write(args.destination, {'schema': 1, 'source_commit': args.source_commit, 'job_exit': args.exit,
                                'phase_exits': {name: int(code) for name, code in (p.split('=', 1) for p in args.phase)},
                                'counts': log_counts([p for p in args.files if pathlib.Path(p).is_file()]),
                                'phase_details': [json.loads(pathlib.Path(p).read_text()) for p in args.phase_receipt if pathlib.Path(p).is_file()],
                                'summary_lines': [line.strip() for p in args.files
                                    for line in (pathlib.Path(p).read_text(errors='replace').splitlines() if pathlib.Path(p).is_file() else [])
                                    if 'Summary' in line or line.startswith('test result:')],
                                'raw': [raw(p) if pathlib.Path(p).is_file() else {'path': str(pathlib.Path(p).resolve()), 'missing': True} for p in args.files]})


if __name__ == '__main__':
    main()
