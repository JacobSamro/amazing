#!/usr/bin/env python3
"""Read original hash-bound WPT failure records in place; never execute a case."""
import argparse
import base64
import importlib.util
import json
from pathlib import Path
import re
import zlib

spec = importlib.util.spec_from_file_location('checked_gate', Path(__file__).with_name('gate-failure-diagnostic.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
MAX_OUTPUT = 64 * 1024
MAX_CAPSULE = 32 * 1024 * 1024


def read(path, sha, limit, size=None):
    return gate.checked_file(path, sha, limit, size)


def unpack(path, sha):
    path = Path(path)
    manifest = json.loads(read(path, sha, 512 * 1024))
    if manifest.get('schema') != 1 or manifest.get('encoding') != 'base64-zlib-json':
        raise ValueError('original classification capsule required')
    parts = manifest.get('parts')
    if not isinstance(parts, list) or not parts or len(parts) > 16:
        raise ValueError('bounded complete capsule parts required')
    data = []
    names = set()
    for index, row in enumerate(parts):
        name = row.get('name')
        if not isinstance(name, str) or Path(name).name != name or name in names:
            raise ValueError('invalid or duplicate capsule part')
        names.add(name)
        part = json.loads(read(path.parent / name, row['sha256'], 512 * 1024, row['bytes']))
        if part.get('schema') != 1 or part.get('part') != index or not isinstance(part.get('data'), str):
            raise ValueError('capsule part identity mismatch')
        data.append(part['data'])
    if sum(map(len, data)) > 2 * 1024 * 1024:
        raise ValueError('compressed capsule exceeds bound')
    decoder = zlib.decompressobj()
    body = decoder.decompress(base64.b64decode(''.join(data), validate=True), MAX_CAPSULE + 1)
    if len(body) > MAX_CAPSULE or not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
        raise ValueError('invalid or oversized capsule stream')
    if len(body) != manifest.get('uncompressed_bytes') or gate.digest(body) != manifest.get('uncompressed_sha256'):
        raise ValueError('capsule expanded hash mismatch')
    return json.loads(body)


def catalogued(marker, endpoint, path, sha):
    path = Path(path)
    relative = path.relative_to(Path.home()).as_posix()
    if not relative.startswith(endpoint['output_root'] + '/'):
        raise ValueError('artifact belongs to a foreign output root')
    entry = marker.get('artifacts', {}).get(relative)
    if (not isinstance(entry, dict) or entry.get('sha256') != sha
            or type(entry.get('bytes')) is not int or path.stat().st_size != entry['bytes']):
        raise ValueError('artifact absent from original completion catalogue')


def bounded(value):
    if value is None:
        return None, False
    text = json.dumps(value, ensure_ascii=True, separators=(',', ':'))
    if len(text.encode()) <= 4096:
        return value, False
    # Preserve actual units; the explicit truncation is diagnostic only.
    if isinstance(value, str):
        while len(json.dumps(value, ensure_ascii=True).encode()) > 4096:
            value = value[:max(1, len(value) // 2)]
        return value, True
    return {'original_json_prefix': text[:4000]}, True


def produce(terminal_path, terminal_sha, summary_path, summary_sha, comparison_path,
            comparison_sha, capsule_path, capsule_sha, source, job, case):
    marker = json.loads(read(terminal_path, terminal_sha, gate.MAX_COMPLETION))
    endpoint, actual, _ = gate.completed_job.diagnostic_completion(marker, source, job)
    if marker.get('completed') is not True or marker.get('catalog_error') is not None:
        raise ValueError('original completed catalogue required')
    for path, sha in [(summary_path, summary_sha), (comparison_path, comparison_sha), (capsule_path, capsule_sha)]:
        catalogued(marker, endpoint, path, sha)
    summary = json.loads(read(summary_path, summary_sha, 512 * 1024))
    if summary.get('source_commit') != source or summary.get('job_exit') != actual:
        raise ValueError('gate source/terminal mismatch')
    phases = [row for row in summary.get('phase_details', []) if row.get('phase') == 'own-pair']
    if len(phases) != 1:
        raise ValueError('original own-pair receipt required exactly once')
    phase = phases[0]
    if (phase.get('source_commit') != source or type(phase.get('command_exit')) is not int
            or phase['command_exit'] == 0 or phase.get('engine_exit') != phase['command_exit']
            or phase.get('wrapper_exit') != phase['command_exit']):
        raise ValueError('original genuinely failed own-pair required')
    command = phase.get('command')
    if not isinstance(command, list) or len(command) != 5 or command[3:] != [str(capsule_path), str(comparison_path)]:
        raise ValueError('own-pair operands differ from captured command')
    comparison = json.loads(read(comparison_path, comparison_sha, 512 * 1024))
    if (comparison.get('source_after') != source or comparison.get('zero_loss') is not False
            or comparison.get('complete_comparable_measurements') is not True
            or comparison.get('after_manifest') != {'path': str(capsule_path), 'sha256': capsule_sha}):
        raise ValueError('original comparison/capsule mismatch')
    capsule = unpack(capsule_path, capsule_sha)
    if capsule.get('source') != source or capsule.get('role') != 'after':
        raise ValueError('capsule source/role mismatch')
    if not re.fullmatch('[a-f0-9]{64}', str(capsule.get('binary_sha256', ''))):
        raise ValueError('actual binary receipt required')
    cases = capsule.get('cases')
    if not isinstance(cases, list) or len(cases) != comparison.get('files'):
        raise ValueError('original complete case inventory required')
    if gate.digest(json.dumps(cases, separators=(',', ':')).encode()) != capsule.get('input_sha256'):
        raise ValueError('case inventory hash mismatch')
    if sum(row[0] == case for row in cases) != 1:
        raise ValueError('case must occur once in original inventory')
    rows = [row for row in capsule.get('records', []) if row.get('file') == case]
    if len(rows) != 1 or len(capsule['records']) != len(cases):
        raise ValueError('original complete case measurements required')
    row = rows[0]
    if row.get('status') == 'pass':
        raise ValueError('selected case has no original failure')
    native = row['raw']['record']
    # The pair interpreter's operands bind the immutable original raw owner.
    original_owner = Path(command[2]).parent.parent
    record_path = Path(native['path'])
    if not original_owner.is_relative_to('/tmp') or not record_path.is_relative_to(original_owner):
        raise ValueError('case record escapes original native owner')
    record = json.loads(read(record_path, native['sha256'], gate.MAX_RAW, native['bytes']))
    report = record.get('report')
    if (record.get('source') != source or record.get('case') != case
            or record.get('binary_sha256') != capsule['binary_sha256']
            or record.get('actual_exit') != row.get('actual_exit')
            or type(record.get('worker_timed_out')) is not bool or not isinstance(report, dict)):
        raise ValueError('original native record identity mismatch')
    argv = record.get('argv')
    viewport = capsule.get('viewport')
    if (not isinstance(argv, list) or len(argv) != 7 or argv[1] != 'wpt-harness'
            or argv[3:] != [case, str(viewport[0]), str(viewport[1]), str(capsule['timeout_ms'])]
            or not Path(argv[0]).is_absolute() or not Path(argv[0]).is_relative_to('/tmp')):
        raise ValueError('captured native argv mismatch')
    if report.get('test') != case or report.get('wpt_revision') != capsule.get('pin'):
        raise ValueError('original native case/pin mismatch')
    if report.get('status') != row.get('native_status') or report.get('harness_status') != row.get('harness_status'):
        raise ValueError('native report differs from classifications')
    subtests = report.get('subtests')
    if not isinstance(subtests, list) or len(subtests) != len(row.get('subtests', [])):
        raise ValueError('native subtest roster mismatch')
    failures = []
    omitted = 0
    for index, (subtest, classified) in enumerate(zip(subtests, row['subtests'])):
        if not isinstance(subtest, dict) or subtest.get('name') != classified.get('name') or subtest.get('status') != classified.get('native_status'):
            raise ValueError('native subtest identity mismatch')
        if subtest.get('status') == 0:
            continue
        fields = {key: bounded(subtest.get(key)) for key in ['name', 'message', 'stack']}
        item = {'index': index, 'native_status': subtest.get('status'),
                **{key: value[0] for key, value in fields.items()},
                'truncated_fields': [key for key, value in fields.items() if value[1]]}
        if len(failures) >= 64 or len(json.dumps(failures + [item], ensure_ascii=True).encode()) > 48 * 1024:
            omitted += 1
        else:
            failures.append(item)
    output = {'schema': 1, 'diagnostic_only': True, 'original_job': endpoint,
              'original_actual_exit': actual, 'source_commit': source, 'case': case,
              'binary_sha256': capsule['binary_sha256'], 'captured_argv': argv,
              'native_record': native, 'terminal_sha256': terminal_sha,
              'gate_sha256': summary_sha, 'comparison_sha256': comparison_sha,
              'capsule_sha256': capsule_sha, 'native_status': report.get('status'),
              'harness_status': report.get('harness_status'),
              'harness_message': bounded(report.get('message'))[0],
              'worker_timed_out': record['worker_timed_out'], 'failures': failures,
              'omitted_nonpassing_records': omitted, 'tests_rerun': False,
              'counts_reconstructed': False, 'classifications_changed': False}
    body = (json.dumps(output, ensure_ascii=True, sort_keys=True) + '\n').encode()
    if len(body) > MAX_OUTPUT:
        raise ValueError('diagnostic exceeds 64 KiB')
    return body


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['terminal', 'terminal-sha256', 'summary', 'summary-sha256', 'comparison',
                 'comparison-sha256', 'capsule', 'capsule-sha256', 'source', 'job', 'case', 'output']:
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    body = produce(args.terminal, args.terminal_sha256, args.summary, args.summary_sha256,
                   args.comparison, args.comparison_sha256, args.capsule, args.capsule_sha256,
                   args.source, args.job, args.case)
    path = Path(args.output)
    if not path.is_absolute() or path.parent.resolve() != path.parent:
        raise ValueError('fresh canonical output required')
    with path.open('xb') as stream:
        stream.write(body)
    print(json.dumps({'diagnostic_only': True, 'bytes': len(body), 'sha256': gate.digest(body)}))


if __name__ == '__main__':
    main()
