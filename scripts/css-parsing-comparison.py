#!/usr/bin/env python3
"""Compare pinned CSS literal-runner failures by case identity, not actual text.

Original failure diagnostics remain untouched. A changed diagnostic for an
already-failing case is not a lost passing case. This does not reconstruct pass
IDs or promote a failed case to pass; unknown formats abort as tooling errors.
"""
import argparse
import collections
import hashlib
import json
import pathlib
import re

LIMIT = 512 * 1024
COHORT = ('selections', 'total', 'files', 'skipped_non_literal')


def quoted(text, start=0):
    if start >= len(text) or text[start] != '"':
        raise ValueError('expected original Rust-debug quoted string')
    i = start + 1
    while i < len(text):
        if text[i] == '\\':
            i += 2
        elif text[i] == '"':
            return text[start:i + 1], i + 1
        else:
            i += 1
    raise ValueError('unterminated original quoted string')


def identity(line):
    if not isinstance(line, str) or not line.startswith('FAIL '):
        raise ValueError('not an original FAIL diagnostic')
    match = re.fullmatch(r'FAIL (.+?): (valid|invalid|computed) ([^:]+): (.+)', line)
    if not match:
        raise ValueError('unknown CSS failure format')
    file, kind, prop, value = match.groups()
    specified, end = quoted(value)
    tail = value[end:]
    expected = None
    if kind == 'valid':
        if tail:
            raise ValueError('unknown valid failure suffix')
    elif kind == 'invalid':
        if tail != ' unexpectedly parsed':
            raise ValueError('unknown invalid failure suffix')
    else:
        if not tail.startswith(' -> '):
            raise ValueError('computed failure lacks actual diagnostic')
        actual, separator, expected_text = tail[4:].rpartition(', expected ')
        if not separator or not actual:
            raise ValueError('computed failure lacks expected identity')
        expected, end = quoted(expected_text)
        if end != len(expected_text):
            raise ValueError('unknown computed expected suffix')
    return json.dumps([file, kind, prop, specified, expected],
                      ensure_ascii=False, separators=(',', ':'))


def failures(summary):
    raw = summary['failure_multiset']
    if not isinstance(raw, dict):
        raise ValueError('original failure multiset missing')
    ids = collections.Counter()
    diagnostics = collections.defaultdict(dict)
    for line, count in raw.items():
        if type(count) is not int or count <= 0:
            raise ValueError('invalid failure multiplicity')
        key = identity(line)
        ids[key] += count
        diagnostics[key][line] = count
    if type(summary['passed']) is not int or type(summary['total']) is not int:
        raise ValueError('original case counts missing')
    if summary['passed'] < 0 or summary['passed'] + sum(ids.values()) != summary['total']:
        raise ValueError('original counts do not reconcile')
    return ids, diagnostics


def compare(before, after):
    for summary in (before, after):
        if any(k not in summary for k in COHORT):
            raise ValueError('original cohort metadata missing')
        if not isinstance(summary['selections'], list) or not summary['selections'] or any(not isinstance(s, str) or not s for s in summary['selections']):
            raise ValueError('original selection metadata invalid')
        if any(type(summary[k]) is not int or summary[k] < 0 for k in COHORT[1:]):
            raise ValueError('original cohort counts invalid')
    if any(before.get(k) != after.get(k) for k in COHORT):
        raise ValueError('pinned literal-runner cohort differs')
    old, old_text = failures(before)
    new, new_text = failures(after)
    lost = new - old
    gained = old - new
    changed = {key: {'before': old_text[key], 'after': new_text[key]}
               for key in sorted(old.keys() & new.keys())
               if old_text[key] != new_text[key]}
    return {'same_cohort': True, 'lost_passing_cases': dict(sorted(lost.items())),
            'gained_passing_cases': dict(sorted(gained.items())),
            'lost_count': sum(lost.values()), 'gained_count': sum(gained.values()),
            'before_passed': before['passed'], 'after_passed': after['passed'],
            'changed_failed_diagnostics': changed,
            'qualification': 'Failure identities only; no pass IDs reconstructed; original failed cases remain failed'}


def load(path, expected_hash, expected_source):
    if not re.fullmatch('[0-9a-f]{64}', expected_hash) or not re.fullmatch('[0-9a-f]{40}', expected_source):
        raise ValueError('exact source and compact SHA256 required')
    body = pathlib.Path(path).read_bytes()
    if len(body) > LIMIT or hashlib.sha256(body).hexdigest() != expected_hash:
        raise ValueError('original compact size/hash mismatch')
    summary = json.loads(body)
    if summary.get('source_commit', summary.get('source')) != expected_source:
        raise ValueError('original compact source mismatch')
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for role in ('before', 'after'):
        parser.add_argument('--' + role, required=True)
        parser.add_argument('--' + role + '-sha256', required=True)
        parser.add_argument('--' + role + '-source', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    before = load(args.before, args.before_sha256, args.before_source)
    after = load(args.after, args.after_sha256, args.after_source)
    result = compare(before, after)
    result.update(before_source=args.before_source, after_source=args.after_source,
                  before_summary_sha256=args.before_sha256,
                  after_summary_sha256=args.after_sha256)
    body = (json.dumps(result, ensure_ascii=False, sort_keys=True,
                       separators=(',', ':')) + '\n').encode()
    if len(body) > LIMIT:
        raise ValueError('compact comparison exceeds limit')
    pathlib.Path(args.output).write_bytes(body)
    return int(bool(result['lost_count']))


if __name__ == '__main__':
    raise SystemExit(main())
