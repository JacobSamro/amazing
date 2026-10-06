#!/usr/bin/env python3
"""Emit bounded Test262 failure details from authenticated original receipts.

This never runs a test, changes a classification, or supplies passing proof.
The outer bulk command may exit zero while a corpus phase truthfully exits one.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import zlib

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('gate_summary', ROOT / 'gate-summary.py')
summary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(summary)
MAX_RAW = 64 * 1024 * 1024
MAX_OUTPUT = 64 * 1024


def sha(data):
    return hashlib.sha256(data).hexdigest()


def checked(path, expected, limit, size=None):
    path = Path(path)
    if not path.is_absolute() or not path.is_relative_to('/tmp') or path.resolve(strict=True) != path:
        raise ValueError('canonical native /tmp input required')
    if not isinstance(expected, str) or len(expected) != 64 or any(c not in '0123456789abcdef' for c in expected):
        raise ValueError('exact sha256 required')
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW), 'rb') as stream:
        meta = os.fstat(stream.fileno())
        if not stat.S_ISREG(meta.st_mode) or meta.st_size > limit:
            raise ValueError('bounded regular input required')
        data = stream.read(limit + 1)
    if len(data) > limit or sha(data) != expected:
        raise ValueError('native input hash/size contradiction')
    if size is not None and (type(size) is not int or size != len(data)):
        raise ValueError('native input byte count contradiction')
    return data


def inflate(encoded):
    decoder = zlib.decompressobj()
    data = decoder.decompress(base64.b64decode(encoded, validate=True), MAX_RAW + 1)
    if len(data) > MAX_RAW or not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
        raise ValueError('unbounded or trailing compact payload')
    return json.loads(data)


def clipped(value, limit=8192):
    data = json.dumps(value, ensure_ascii=False).encode('utf-8')
    return {'text': data[:limit].decode('utf-8', errors='ignore'),
            'bytes': len(data), 'truncated': len(data) > limit}


def produce(terminal_path, terminal_sha, phase_path, phase_sha,
            compact_path, compact_sha, source, job, cases):
    if len(cases) not in range(1, 17) or len(set(cases)) != len(cases):
        raise ValueError('one to sixteen unique original cases required')
    terminal = json.loads(checked(terminal_path, terminal_sha, 65536))
    endpoint = terminal.get('job', {})
    code = terminal.get('actual_exit')
    if (terminal.get('state') != 'TERMINAL' or type(code) is not int or code not in (0, 1)
            or type(terminal.get('runner_exit')) is not int or terminal['runner_exit'] != code
            or type(terminal.get('command_returncode')) is not int or terminal['command_returncode'] != code
            or endpoint.get('source_commit') != source or endpoint.get('job_id') != job
            or not isinstance(source, str) or len(source) != 40 or any(c not in '0123456789abcdef' for c in source)
            or not isinstance(job, str) or len(job) != 32 or any(c not in '0123456789abcdef' for c in job)):
        raise ValueError('authentic terminal job/source/exit required')
    phase = json.loads(checked(phase_path, phase_sha, 524288))
    for key in ('command_exit', 'engine_exit', 'wrapper_exit'):
        if type(phase.get(key)) is not int or phase[key] != 1:
            raise ValueError('genuine normal failing corpus phase required')
    if (phase.get('source_commit') != source or type(phase.get('output_copy_exit')) is not int
            or phase['output_copy_exit'] != 0 or phase.get('receipt_copy_error')):
        raise ValueError('truthful source and successful original output copy required')
    raw_log = phase['raw_native']
    checked(raw_log['path'], raw_log['sha256'], MAX_RAW, raw_log['bytes'])
    compact = json.loads(checked(compact_path, compact_sha, 524288))
    if (compact.get('schema') != 2 or compact.get('source_commit') != source
            or type(compact.get('job_exit')) is not int or compact['job_exit'] != 0
            or type(compact.get('runner_exit')) is not int or compact['runner_exit'] != 1):
        raise ValueError('original normal completed Test262 compact required')
    argv = phase.get('command')
    if (not isinstance(argv, list) or 'scripts/test262.py' not in argv
            or argv.count('--output') != 1 or argv.count('--binary') != 1):
        raise ValueError('actual public Test262 command required')
    raw = compact['raw']
    if argv[argv.index('--output') + 1] != raw['path']:
        raise ValueError('original command/report path contradiction')
    report = json.loads(checked(raw['path'], raw['sha256'], MAX_RAW, raw['bytes']))
    for key in ('test262_revision', 'input_sha256', 'host_sha256', 'runner_sha256',
                'timeout_seconds', 'classification_sha256', 'shard'):
        if report.get(key) != compact.get(key):
            raise ValueError('original report/compact provenance contradiction: ' + key)
    for key in ('filter', 'jobs', 'limit'):
        if report.get(key) != compact['selection'].get(key):
            raise ValueError('original selection contradiction')
    records = report['records']
    summary.validate_records(records)
    stable = [[r['path'], r['status'], r['reason'],
               [[v['variant'], v['status'], v['reason']] for v in r['variants']]] for r in records]
    if sha(json.dumps(stable, separators=(',', ':'), ensure_ascii=True).encode()) != compact['classification_sha256']:
        raise ValueError('original classification hash contradiction')
    originals = inflate(compact['compact_records'])
    summary.validate_records(originals)
    original_by_id = {r['path']: r for r in originals}
    by_id = {r['path']: r for r in records}
    output = []
    for case in cases:
        actual = by_id.get(case)
        saved = original_by_id.get(case)
        if actual is None or saved is None or actual['status'] not in ('fail', 'timeout'):
            raise ValueError('case must be an original failing classification')
        if saved['status'] != actual['status'] or saved['variants'] != [
                {'variant': v['variant'], 'status': v['status']} for v in actual['variants']]:
            raise ValueError('original case/variant classification contradiction')
        failures = [{'variant': v['variant'], 'status': v['status'], 'reason': v['reason'],
                     'details': clipped({k: value for k, value in v.items()
                                         if k not in ('variant', 'status', 'reason')})}
                    for v in actual['variants'] if v['status'] in ('fail', 'timeout')]
        output.append({'path': case, 'status': actual['status'], 'reason': actual['reason'],
                       'failed_variants': failures, 'file_details': clipped({
                           k: value for k, value in actual.items()
                           if k not in ('path', 'status', 'reason', 'variants')}, 2048)})
    result = {'schema': 1, 'diagnostic_only': True, 'qualification_claimed': False,
              'tests_rerun': False, 'classification_unchanged': True,
              'source_commit': source, 'original_job': endpoint, 'original_actual_exit': code,
              'phase': phase['phase'], 'phase_actual_exit': 1,
              'terminal': {'path': str(terminal_path), 'sha256': terminal_sha},
              'phase_receipt': {'path': str(phase_path), 'sha256': phase_sha},
              'original_compact': {'path': str(compact_path), 'sha256': compact_sha},
              'native_report': raw, 'phase_native_raw': raw_log, 'cases': output}
    data = (json.dumps(result, sort_keys=True, ensure_ascii=True) + '\n').encode()
    if len(data) > MAX_OUTPUT:
        raise ValueError('diagnostic exceeds64KiB; request fewer original cases')
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('terminal', 'terminal-sha256', 'phase-receipt', 'phase-receipt-sha256',
                'compact', 'compact-sha256', 'source', 'job', 'output'):
        parser.add_argument('--' + key, required=True)
    parser.add_argument('--case', action='append', required=True)
    args = parser.parse_args()
    data = produce(args.terminal, args.terminal_sha256, args.phase_receipt,
                   args.phase_receipt_sha256, args.compact, args.compact_sha256,
                   args.source, args.job, args.case)
    out = Path(args.output)
    if not out.is_absolute() or out.parent.resolve(strict=True) != out.parent:
        raise ValueError('fresh canonical output required')
    with out.open('xb') as stream:
        stream.write(data)
    print(json.dumps({'bytes': len(data), 'sha256': sha(data), 'diagnostic_only': True}))


if __name__ == '__main__':
    main()
