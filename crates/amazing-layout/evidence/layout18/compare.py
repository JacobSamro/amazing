#!/usr/bin/env python3
"""Compare pinned render reports without changing their classifications."""
import argparse
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('before', type=Path)
parser.add_argument('after', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
before = json.loads(args.before.read_text(encoding='utf-8-sig'))
after = json.loads(args.after.read_text(encoding='utf-8-sig'))
for key in ('wpt_revision', 'viewport'):
    if before[key] != after[key]:
        raise SystemExit(f'Incomparable {key}')
old = {case['id']: case for case in before['tests']}
new = {case['id']: case for case in after['tests']}
if len(old) != len(before['tests']) or len(new) != len(after['tests']):
    raise SystemExit('Duplicate case IDs in report')
if old.keys() != new.keys():
    raise SystemExit('Incomparable case selections')

def projection(report):
    fields = ('id', 'classification', 'actual_sha256', 'reference_sha256', 'diff_sha256')
    normalized = [{key: case.get(key) for key in fields}
                  for case in sorted(report['tests'], key=lambda case: case['id'])]
    return hashlib.sha256(json.dumps(normalized, sort_keys=True, separators=(',', ':')).encode()).hexdigest()

result = {
    'before': str(args.before), 'after': str(args.after),
    'revision': before['wpt_revision'], 'cases': len(old),
    'before_counts': before['counts'], 'after_counts': after['counts'],
    'gains': sorted(key for key in old if old[key]['classification'] != 'pass' and new[key]['classification'] == 'pass'),
    'losses': sorted(key for key in old if old[key]['classification'] == 'pass' and new[key]['classification'] != 'pass'),
    'actual_hash_changes': sum(old[key].get('actual_sha256') != new[key].get('actual_sha256') for key in old),
    'reference_hash_changes': sum(old[key].get('reference_sha256') != new[key].get('reference_sha256') for key in old),
    'before_projection_sha256': projection(before),
    'after_projection_sha256': projection(after),
}
args.output.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({key: (len(value) if key in ('gains', 'losses') else value)
                  for key, value in result.items()}, indent=2))
