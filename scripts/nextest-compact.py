#!/usr/bin/env python3
"""Collect nextest terminal identities and compare compact pass sets.

Use --status-level all. Raw output stays native. A gate-command.py phase receipt
must match the raw hash and source. Exit 1 means engine failure or lost passes;
exit 3 means incomplete/ambiguous collector evidence. Neither is waived.
"""
import argparse
import base64
import collections
import hashlib
import json
import pathlib
import re
import zlib

LIMIT = 512 * 1024
PROGRESS = re.compile(r'^\(\s*\d+/\d+\)\s+')
ANSI = re.compile(r'\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]')
TERMINAL = re.compile(r'^\s*(PASS|FAIL|SKIP)\s+\[[^\]]+\]\s+(.+?)\s*$')
SUMMARY = re.compile(r'\bSummary\s+\[[^\]]+\]\s+(\d+) tests? run:\s*(.*)$')


def describe(path):
    path = pathlib.Path(path)
    with path.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    return {'path': str(path.resolve()), 'bytes': path.stat().st_size, 'sha256': digest}


def normalize(records):
    result, changed = {}, 0
    for identity, status in records.items():
        if not isinstance(identity, str) or status not in ('pass', 'fail', 'skip'):
            raise ValueError('invalid identity or terminal classification')
        key = PROGRESS.sub('', identity, count=1)
        if not key or key in result:
            raise ValueError('empty or colliding normalized identity: ' + key)
        result[key] = status
        changed += key != identity
    return result, changed


def parse(lines):
    records, summaries, errors = {}, [], []
    changed = 0
    for number, raw in enumerate(lines, 1):
        line = ANSI.sub('', raw)
        match = TERMINAL.match(line)
        if match:
            status, identity = match.groups()
            key = PROGRESS.sub('', identity, count=1)
            changed += key != identity
            # Binary target + test name are the nextest identity; preserve name
            # punctuation, internal spaces and parenthesized expressions.
            if not re.match(r'^\S+\s+\S', key):
                errors.append('malformed terminal identity at line ' + str(number))
            elif key in records:
                if not summaries or records[key] != status.lower():
                    errors.append('duplicate/colliding terminal identity: ' + key)
            else:
                records[key] = status.lower()
        elif re.match(r'^\s*(PASS|FAIL|SKIP)\b', line):
            errors.append('unparsed terminal row at line ' + str(number))
        match = SUMMARY.search(line)
        if match:
            total, tail = match.groups()
            counts = {}
            for name in ('passed', 'failed', 'skipped'):
                values = re.findall(r'(\d+)\s+' + name + r'\b', tail)
                if len(values) > 1:
                    errors.append('ambiguous summary count: ' + name)
                counts[name] = int(values[0]) if values else 0
            summaries.append({'total': int(total), **counts})
    observed = collections.Counter(records.values())
    if len(summaries) != 1:
        errors.append('expected exactly one final nextest Summary')
    else:
        summary = summaries[0]
        if summary['total'] != observed['pass'] + observed['fail'] or any(
                summary[k] != observed[v] for k, v in
                [('passed', 'pass'), ('failed', 'fail'), ('skipped', 'skip')]):
            errors.append('terminal identities do not match final Summary counts')
    return records, summaries, errors, changed


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key: " + key)
        result[key] = value
    return result


def unpack(report):
    if 'classifications' in report:
        records = report['classifications']
    else:
        data = base64.b64decode(report['classifications_zlib_base64'], validate=True)
        decoder = zlib.decompressobj()
        payload = decoder.decompress(data, 16 * 1024 * 1024 + 1)
        if len(payload) > 16 * 1024 * 1024 or not decoder.eof or decoder.unused_data:
            raise ValueError('invalid or oversized compressed identity map')
        records = json.loads(payload, object_pairs_hook=unique_object)
    if not isinstance(records, dict):
        raise ValueError('classification map must be an object')
    return normalize(records)


def comparison(before, after):
    old = {k for k, v in before.items() if v == 'pass'}
    new = {k for k, v in after.items() if v == 'pass'}
    return {'baseline_pass': len(old), 'current_pass': len(new),
            'lost': sorted(old - new), 'gained': sorted(new - old)}


def collect(raw_path, phase_path, source, baseline=None):
    phase = json.loads(pathlib.Path(phase_path).read_text())
    raw = describe(raw_path)
    records, summaries, errors, changed = parse(pathlib.Path(raw_path).read_text(errors='replace').splitlines())
    native = phase.get('raw_native', {})
    if any(native.get(k) != raw[k] for k in ('bytes', 'sha256')):
        errors.append('raw hash/size does not match native phase receipt')
    if phase.get('source_commit') != source:
        errors.append('source does not match phase receipt')
    actual = phase.get('engine_exit')
    command_exit = phase.get('command_exit')
    wrapper_exit = phase.get('wrapper_exit')
    copy_exit = phase.get('output_copy_exit')
    if any(type(value) is not int for value in
           (command_exit, actual, wrapper_exit, copy_exit)):
        errors.append('actual command/engine/wrapper/copy exits require integer witnesses')
    else:
        # gate-command preserves subprocess's signed signal return code and
        # projects it into an ordinary shell exit for engine/wrapper receipts.
        projected = command_exit if command_exit >= 0 else 128 - command_exit
        if actual < 0 or wrapper_exit < 0 or actual != projected:
            errors.append('command/engine exits contradict gate-command convention')
        if wrapper_exit != actual or copy_exit != 0:
            errors.append('phase wrapper/copy failed')
    failed = any(v == 'fail' for v in records.values())
    if actual == 0 and failed:
        errors.append('engine exit zero contradicts failed tests')
    compare = None
    baseline_ref = None
    if baseline:
        baseline_ref = describe(baseline)
        try:
            before, _ = unpack(json.loads(pathlib.Path(baseline).read_text(), object_pairs_hook=unique_object))
            compare = comparison(before, records)
        except (ValueError, KeyError, TypeError) as error:
            errors.append('invalid baseline: ' + str(error))
    exit_code = 3 if errors else (1 if actual != 0 or failed or (compare and compare['lost']) else 0)
    return {'schema': 1, 'source_commit': source, 'phase': phase,
            'phase_receipt': describe(phase_path), 'raw': raw,
            'collector': describe(__file__),
            'actual_engine_exit': actual, 'collector_exit': exit_code,
            'errors': errors, 'summaries': summaries,
            'counts': dict(collections.Counter(records.values())),
            'normalization': {'regex': PROGRESS.pattern, 'changed': changed},
            'baseline': baseline_ref, 'comparison': compare,
            'classifications_zlib_base64': base64.b64encode(zlib.compress(
                json.dumps(records, sort_keys=True, separators=(',', ':')).encode(), 9)).decode()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-commit', required=True)
    parser.add_argument('--phase-receipt', required=True)
    parser.add_argument('--baseline')
    parser.add_argument('raw'); parser.add_argument('destination')
    args = parser.parse_args()
    result = collect(args.raw, args.phase_receipt, args.source_commit, args.baseline)
    encoded = (json.dumps(result, sort_keys=True, separators=(',', ':')) + '\n').encode()
    if len(encoded) > LIMIT:
        raise ValueError('compact exceeds 512 KiB; leave raw on host')
    pathlib.Path(args.destination).write_bytes(encoded)
    print(json.dumps({'collector_exit': result['collector_exit'], 'counts': result['counts'],
                      'errors': result['errors'], 'comparison': result['comparison']}))
    return result['collector_exit']


if __name__ == '__main__':
    raise SystemExit(main())
