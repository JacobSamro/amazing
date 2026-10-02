"""Zero-loss recheck of all before/main/new RegExp passes at full deadline."""
import json
import os
from pathlib import Path
import shutil
import subprocess

out = Path(os.environ['OUT'])
root = Path(os.environ['TEST262_ROOT'])
after = json.loads((out / 'jsd7-after.json').read_text())
before = json.loads((out / 'jsd7-before.json').read_text())
main = json.loads(Path('docs/lanes/js-date_jsd7_baseline-passes.json').read_text())
assert before['test262_revision'] == after['test262_revision'] == main['test262_revision']
paths = set(main['paths'])
for report in [before, after]:
    paths.update(record['path'] for record in report['records'] if record['status']=='pass')
gate = out / 'jsd7-pass-root'
gate.mkdir(exist_ok=True)
shutil.copytree(root / 'harness', gate / 'harness', dirs_exist_ok=True)
(gate / '.git').mkdir(exist_ok=True)
(gate / '.git/HEAD').write_text(after['test262_revision']+'\n')
for path in sorted(paths):
    target = gate / 'test' / path
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(root / 'test' / path, target)
subprocess.run(['python3','scripts/test262.py',str(gate),'--binary',
    str(Path(os.environ['CARGO_TARGET_DIR'])/'release/examples/test262_host'),
    '--jobs','8','--timeout','60','--output',str(out/'jsd7-pass-gate.json')],check=True)
result = json.loads((out/'jsd7-pass-gate.json').read_text())
assert all(record['status']=='pass' for record in result['records']), 'lost passing path at full deadline'
old = {r['path'] for r in before['records'] if r['status']=='pass'}
new = {r['path'] for r in after['records'] if r['status']=='pass'}
print('JSD7 DELTA',dict(gained=len(new-old),iteration_lost=len(old-new),full_deadline_losses=0,rechecked=len(paths)),flush=True)
