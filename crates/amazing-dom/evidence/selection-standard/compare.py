#!/usr/bin/env python3
"""Compare full pinned reports, retaining exact UTF-16 passing-name multiplicity."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path


def digest(name):
    return hashlib.sha256(name.encode('utf-16le', errors='surrogatepass')).hexdigest()


def records(path):
    text = Path(path).read_text()
    if text.lstrip().startswith('{') and '\n{"' not in text:
        data = json.loads(text)
        if 'results' in data:
            return data['results']
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def compact(record):
    names = record.get('passing_names_utf16le_sha256')
    if names is None:
        names = Counter(digest(test['name']) for test in record.get('subtests', []) if test['status'] == 0)
    return {'test': record['test'], 'status': record['status'], 'counts': record['counts'],
            'passing_names_utf16le_sha256': dict(sorted(names.items()))}


def compare(before, after):
    left = {r['test']: compact(r) for r in records(before)}
    right = {r['test']: compact(r) for r in records(after)}
    lost = {}; gained = {}; lost_files = []
    for test, old in left.items():
        new = right.get(test)
        old_names = Counter(old['passing_names_utf16le_sha256'])
        new_names = Counter(new['passing_names_utf16le_sha256']) if new else Counter()
        if old_names - new_names:
            lost[test] = dict(old_names - new_names)
        if new_names - old_names:
            gained[test] = dict(new_names - old_names)
        if old['status'] == 'pass' and (not new or new['status'] != 'pass'):
            lost_files.append(test)
    return {'before': str(before), 'after': str(after), 'before_files': len(left), 'after_files': len(right),
            'missing_files': sorted(left.keys() - right.keys()), 'added_files': sorted(right.keys() - left.keys()),
            'lost_passing_names': lost, 'lost_passing_assertions': sum(sum(v.values()) for v in lost.values()),
            'gained_passing_names': gained, 'gained_passing_assertions': sum(sum(v.values()) for v in gained.values()),
            'lost_passing_files': lost_files}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('before'); parser.add_argument('after'); parser.add_argument('output')
    args = parser.parse_args()
    result = compare(args.before, args.after)
    Path(args.output).write_text(json.dumps(result, sort_keys=True, indent=2) + '\n')
    print(json.dumps({k: v for k, v in result.items() if k not in {'lost_passing_names', 'gained_passing_names'}}, sort_keys=True))
    raise SystemExit(bool(result['lost_passing_assertions'] or result['lost_passing_files'] or result['missing_files']))


if __name__ == '__main__':
    main()
