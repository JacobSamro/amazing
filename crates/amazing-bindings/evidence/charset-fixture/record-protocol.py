"""Run pinned upstream fixture code unchanged on Linux and record body provenance.

The lightweight GET object supplies byte parameters, as WPT's request does.
These records verify the server protocol; they do not set engine document
metadata or replace any corpus assertion.
"""
import hashlib
import json
from pathlib import Path
import re
import runpy
import sys
import types
from urllib.parse import parse_qsl, quote

# The pinned shared corpus is read-only, including Python bytecode caches.
sys.dont_write_bytecode = True

root = Path(sys.argv[1])
output = Path(sys.argv[2])
package = types.ModuleType('wptserve')
package.__path__ = [str(root / 'tools/wptserve/wptserve')]
sys.modules['wptserve'] = package
handler = runpy.run_path(str(root / 'dom/nodes/encoding.py'))['main']
baseline = json.loads((Path(__file__).parent / 'before.json').read_text())
labels = set()
for result in baseline['results']:
    for subtest in result['subtests']:
        match = re.search(r'has label ("(?:[^"\\]|\\.)*")', subtest['name'])
        if match:
            labels.add(json.loads(match.group(1)))
queries = ['label=' + quote(label, safe='') for label in sorted(labels)]
queries += ['label=', 'label=big5&label=utf-8', 'label=%80%22%26%3C%3E%27+', 'other=utf-8']
class Parameters:
    def __init__(self, query):
        self.values = parse_qsl(query, keep_blank_values=True, encoding='latin-1')
    def first(self, key):
        return next(value.encode('latin-1') for name, value in self.values
                    if name.encode('latin-1') == key)
records = []
for query in queries:
    request = types.SimpleNamespace(GET=Parameters(query))
    try:
        body = handler(request, types.SimpleNamespace())
        records.append({'query': query, 'status': 200, 'content_type': None,
                        'body': body, 'body_sha256': hashlib.sha256(body.encode('utf-8')).hexdigest()})
    except (StopIteration, KeyError):
        records.append({'query': query, 'status': 500, 'content_type': 'application/json'})
files = ['dom/nodes/encoding.py', 'dom/nodes/characterset-helper.js',
         'dom/nodes/Document-characterSet-normalization-1.html',
         'dom/nodes/Document-characterSet-normalization-2.html',
         'tools/wptserve/wptserve/handlers.py', 'tools/wptserve/wptserve/response.py',
         'tools/wptserve/wptserve/utils.py']
output.write_text(json.dumps({'revision': baseline['wpt_revision'],
    'raw_sha256': {path: hashlib.sha256((root/path).read_bytes()).hexdigest() for path in files},
    'records': records}, indent=2))
print('UPSTREAM_PROTOCOL', len(records), 'records', flush=True)
