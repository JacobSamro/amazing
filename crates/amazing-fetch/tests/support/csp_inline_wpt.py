#!/usr/bin/env python3
"""Whole pinned CSP source-file corpus for inline behavior, using the installed
Amazing WPT runner and its unmodified upstream testharness. This is a browser
integration measurement, separate from the lower producer's native controls.
Unsupported platform/server globals remain explicit classifications.
"""
import argparse
import concurrent.futures
import hashlib
import importlib.util
import json
from pathlib import Path

DIRECTORIES = ('default-src', 'script-src', 'script-src-attr-elem', 'unsafe-hashes',
               'nonce-hiding', 'meta', 'parsing', 'securitypolicyviolation',
               'navigation', 'style-src', 'style-src-attr-elem')

def encoded(value):
    return (json.dumps(value, sort_keys=True, separators=(',', ':')) + '\n').encode()

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--binary', type=Path, required=True)
    ap.add_argument('--wpt-root', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--source', required=True)
    ap.add_argument('--base', required=True)
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('--baseline', type=Path)
    args = ap.parse_args()
    repo = Path.cwd().resolve()
    spec = importlib.util.spec_from_file_location('wpt_measure', repo/'crates/amazing/src/cmd/measure-wpt-dom.py')
    measure = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(measure)
    root=args.wpt_root.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    entries=[]; excluded=[]
    for directory in DIRECTORIES:
        inventory=measure.inventory(root, 'content-security-policy/'+directory)
        entries.extend(inventory['selected']);excluded.extend(inventory['excluded'])
    selection={'wpt_revision':measure.REVISION,'directories':DIRECTORIES,'selected':entries,'excluded':excluded}
    (args.output/'selection.json').write_bytes(encoded(selection))
    def run(entry):
        return measure.run(args.binary.resolve(),root,entry,60000 if entry['long_timeout'] else 10000)
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        records=[]
        for result in pool.map(run,entries):
            records.append(result)
            if len(records)%25==0: print(f'CSP {len(records)}/{len(entries)}',flush=True)
    raw=b''.join(encoded(record) for record in records)
    raw_path=args.output/'results.jsonl';raw_path.write_bytes(raw)
    passed={r['test'] for r in records if r['status']=='pass'}
    pass_subtests={r['test']+'::'+str(index)+'::'+s['name'] for r in records for index,s in enumerate(r['subtests']) if s['status']=='pass'}
    compact={'source':args.source,'base':args.base,'wpt_revision':measure.REVISION,
        'directories':DIRECTORIES,'binary_sha256':hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        'raw_path':str(raw_path),'raw_sha256':hashlib.sha256(raw).hexdigest(),
        'selection_sha256':hashlib.sha256(encoded(selection)).hexdigest(),
        'harness_sha256':hashlib.sha256((root/'resources/testharness.js').read_bytes()).hexdigest(),
        'counts':{status:sum(r['status']==status for r in records) for status in measure.FILE_STATUSES},
        'subtests':{status:sum(r['counts'][status] for r in records) for status in measure.SUBTEST_STATUSES},
        'classifications':{r['test']:r['status'] for r in records},
        'passing_files':sorted(passed),'passing_subtests':sorted(pass_subtests)}
    if args.baseline:
        before=json.loads(args.baseline.read_text())
        if before['selection_sha256']!=compact['selection_sha256']:raise RuntimeError('Selection drift')
        for unit,after in [('files',passed),('subtests',pass_subtests)]:
            old=set(before['passing_'+unit]);compact['lost_'+unit]=sorted(old-after);compact['gained_'+unit]=sorted(after-old)
    data=encoded(compact)
    if len(data)>512*1024:raise RuntimeError('Compact evidence exceeds policy; split pass sets without losing identities')
    (args.output/'compact.json').write_bytes(data)
    print(json.dumps({k:compact[k] for k in ('counts','subtests','raw_sha256')}),flush=True)
    if any(compact.get('lost_'+unit) for unit in ('files','subtests')):raise SystemExit(1)
if __name__=='__main__':main()
