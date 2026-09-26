#!/usr/bin/env python3
"""Summarize table results by filename family; these are triage hints, not diagnoses."""
import argparse
import json
import re
from collections import Counter, defaultdict
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('report', type=Path)
p.add_argument('output', type=Path)
a = p.parse_args()
report = json.loads(a.report.read_text())
families = [
    ('collapsed borders', r'border-collapse|collapsed-border|border-conflict'),
    ('border spacing', r'border-spacing|border-separation'),
    ('captions', r'caption'),
    ('fixed widths', r'fixed.*(?:layout|table)|table.*fixed'),
    ('visibility', r'visibility|collapse-col|collapse-row'),
    ('positioning', r'absolute|position|relative|sticky'),
    ('vertical flow', r'vertical|writing-mode|sideways'),
    ('width constraints', r'width|sizing|column|colspan|percent|calc'),
    ('row heights', r'height|rowspan|baseline|align'),
    ('empty cells', r'empty'),
    ('tree repair', r'anonymous|display|whitespace|white-space'),
    ('borders and backgrounds', r'border|background'),
]
groups = defaultdict(list)
for case in report['tests']:
    classification = case['classification']
    if classification not in ('pass', 'mismatch'):
        key = classification + ': ' + case.get('detail', '').split('\n')[0]
    else:
        key = next((name for name, pattern in families if re.search(pattern, case['id'], re.I)), 'other tables')
    groups[key].append(case)
result = []
for name, cases in groups.items():
    result.append(dict(family=name, counts=dict(Counter(c['classification'] for c in cases)),
                       failures=[c['id'] for c in cases if c['classification'] != 'pass']))
result.sort(key=lambda group: (-len(group['failures']), group['family']))
a.output.write_text(json.dumps(dict(source=report['source_commit'], counts=report['counts'],
    note='Filename families only; inspect upstream fixtures before assigning a root cause.', families=result),
    separators=(',', ':')) + '\n')
for group in result:
    print(f"{group['family']}: {json.dumps(group['counts'])}")
