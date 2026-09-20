#!/usr/bin/env python3
"""Offline PSL data preparation; never fetches. Run --check on the build server.

Trusted upstream Unicode labels are encoded with RFC3492 punycode, NOT Python's
IDNA2003 codec (which would change ß). This does not authorize runtime IDNA.
No Unicode mapping/version tables are used: upstream canonical spelling is input.
"""
import hashlib
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1] / 'data' / 'psl'
PIN = json.loads((ROOT / 'pin.json').read_text(encoding='utf-8'))

def ascii_host(host):
    return '.'.join(label.lower() if label.isascii() else
                    'xn--' + label.encode('punycode').decode('ascii')
                    for label in host.split('.'))

def generate():
    entry = PIN['files']['public_suffix_list.dat']
    raw = b''.join((ROOT / part).read_bytes() for part in entry['parts'])
    for name, data in [('public_suffix_list.dat', raw),
                       ('LICENSE', (ROOT / 'LICENSE').read_bytes()),
                       ('tests/test_psl.txt', (ROOT / 'test_psl.txt').read_bytes())]:
        if hashlib.sha256(data).hexdigest() != PIN['files'][name]['sha256']:
            raise ValueError('Pinned input changed: ' + name)
    rules = set()
    sections = set()
    active = None
    for line in raw.decode('utf-8').splitlines():
        if line.startswith('// ===BEGIN '):
            active = line.split('BEGIN ')[1].split(' DOMAINS')[0]
            sections.add(active)
        if line.startswith('// ===END '):
            active = None
        if not line.strip() or line.startswith('//'):
            continue
        if active not in ('ICANN', 'PRIVATE'):
            raise ValueError('Rule outside approved sections')
        rule = ascii_host(line.split()[0])
        if not re.fullmatch(r'(?:!|\*\.)?[a-z0-9-]+(?:\.[a-z0-9-]+)*', rule):
            raise ValueError(rule)
        rules.add(rule)
    if sections != {'ICANN', 'PRIVATE'}:
        raise ValueError('Missing section')
    rules = sorted(rules)
    result = {}
    for i in range(0, len(rules), 2500):
        result[f'rules-{i//2500:02}.txt'] = ('\n'.join(rules[i:i+2500])+'\n').encode()
    cases = []
    for host, expected in re.findall(r'^checkPublicSuffix\((.*?), (.*?)\);', (ROOT / 'test_psl.txt').read_text(encoding='utf-8'), re.MULTILINE):
        def norm(s):
            return '-' if s == 'null' else ascii_host(s.strip("'")) or '-'
        cases.append(norm(host)+'\t'+norm(expected))
    result['fixtures.tsv'] = ('\n'.join(cases)+'\n').encode()
    return result

outputs = generate()
manifest = {name: hashlib.sha256(data).hexdigest() for name, data in outputs.items()}
manifest['generator'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
outputs['generated.json'] = (json.dumps(manifest, indent=2)+'\n').encode()
for name, data in outputs.items():
    if '--check' in sys.argv:
        if (ROOT/name).read_bytes() != data:
            raise ValueError('Regeneration mismatch: '+name)
    else:
        (ROOT/name).write_bytes(data)
print(f'{len(outputs)-2} rule shards; offline normalization complete')
