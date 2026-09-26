#!/usr/bin/env python3
"""Inventory every static reftest in both complete table corpus directories."""
import argparse
import importlib.util
import json
import re
import subprocess
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('wpt', type=Path)
p.add_argument('output', type=Path)
a = p.parse_args()
root = a.wpt.resolve()
spec = importlib.util.spec_from_file_location('render', Path(__file__).parent.parent / 'layout18/render-linux.py')
render = importlib.util.module_from_spec(spec)
spec.loader.exec_module(render)
tests, excluded = [], []
directories = ['css/css-tables', 'css/CSS2/tables']
for directory in directories:
    for path in sorted((root / directory).rglob('*')):
        if path.suffix not in ('.html', '.xht', '.xhtml'):
            continue
        source = path.read_text(errors='replace')
        name = path.relative_to(root).as_posix()
        try:
            _, _, reference = render.reference(source, path, root)
        except ValueError:
            continue
        reason = None
        if not reference.is_file():
            reason = 'missing reference'
        elif re.search(r'<script\b', source + reference.read_text(errors='replace'), re.I):
            reason = 'requires script execution'
        if reason:
            excluded.append(dict(test=name, reason=reason))
        else:
            tests.append(dict(id=name.removeprefix('css/').replace('/', '__'), test=name))
revision = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
a.output.write_text(json.dumps(dict(schema=1, revision=revision,
    selection=dict(kind='LAYOUT-19 whole static table directories', case_count=len(tests)),
    sparse_paths=directories, tests=tests, excluded=excluded), separators=(',', ':')) + '\n')
print(json.dumps(dict(selected=len(tests), excluded=len(excluded))), flush=True)
