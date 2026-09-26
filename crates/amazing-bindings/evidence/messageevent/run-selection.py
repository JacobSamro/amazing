"""Pinned MessageEvent/EventTarget corpus. Run on the lane's remote Linux host."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('label')
parser.add_argument('source')
args = parser.parse_args()
root = Path.home() / 'corpora/wpt'
binary = Path(os.environ['CARGO_TARGET_DIR']) / 'debug/amazing'
output = Path.home() / 'amazing/out' / ('messageevent-' + args.label)
output.mkdir(parents=True, exist_ok=True)
revision = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
assert revision == 'c2a2fce69e5c2509d9dc28440ec8b42532e5c293'
selection = [
    'html/webappapis/scripting/events/messageevent-constructor.https.html',
    'webmessaging/MessageEvent.any.js',
    'webmessaging/MessageEvent_properties.htm',
    'webmessaging/MessageEvent-trusted.window.js',
    'dom/events/Event-subclasses-constructors.html',
    'dom/events/passive-by-default.html',
    'dom/nodes/Document-createEvent.https.html',
]
results = []
for number, test in enumerate(selection):
    with tempfile.TemporaryDirectory(prefix='messageevent-storage-') as storage:
        run = subprocess.run([str(binary), 'wpt-harness', str(root), test, '800', '600', '20000'],
                             env={**os.environ, 'AMAZING_STORAGE_DIR': storage},
                             capture_output=True, text=True, timeout=45)
    (output / f'{number}.json').write_text(run.stdout)
    result = json.loads(run.stdout)
    results.append(result)
    print(test, result['status'], result['counts'], result.get('detail'), flush=True)
report = dict(source=args.source, platform='Linux Boat dom-bindings', wpt_revision=revision,
              binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), results=results)
(output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
print('RESULTS', output / 'results.json', flush=True)
