#!/usr/bin/env python3
"""Filename-family triage; retain every raw classification and loss ID."""
import collections
import json
import re
import sys
from pathlib import Path
report=json.loads(Path(sys.argv[1]).read_text())
groups=collections.defaultdict(collections.Counter)
for case in report['tests']:
    name=case['id'].split('__')[-1]
    family=re.sub(r'-\d.*','',name)
    for key in ['text-combine','text-orientation','direction','bidi','orthogonal',
                'abs-pos','float','block-flow','wm-propagation','text-align']:
        if name.startswith(key):
            family=key
            break
    groups[family][case['classification']]+=1
result=dict(counts=report['counts'],groups=dict(sorted(groups.items(),key=lambda item:-sum(item[1].values()))))
Path(sys.argv[2]).write_text(json.dumps(result,indent=2)+'\n')
for family,counts in list(result['groups'].items())[:15]:
    print(family,dict(counts))
