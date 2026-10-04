#!/usr/bin/env python3
"""Public exact-ref render composition; native raw, compact durable receipts."""
import argparse
import importlib.util
import json
import os
import pathlib
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('gate_command', ROOT / 'gate-command.py')
command = importlib.util.module_from_spec(spec); spec.loader.exec_module(command)
spec = importlib.util.spec_from_file_location('gate_summary', ROOT / 'gate-summary.py')
summary = importlib.util.module_from_spec(spec); spec.loader.exec_module(summary)


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--source', required=True); p.add_argument('--tag', required=True)
    p.add_argument('--workspace', action='store_true'); p.add_argument('--baseline', default='')
    p.add_argument('--baseline-ref', default=''); p.add_argument('--baseline-tag', default=''); p.add_argument('--release', action='store_true')
    p.add_argument('--width', type=int, default=800); p.add_argument('--height', type=int, default=600)
    p.add_argument('--timeout', type=float, default=20); p.add_argument('--jobs', type=int, default=int(os.environ.get('JOBS','1')))
    p.add_argument('--limit', type=int, default=0);p.add_argument('subdirs',nargs='+');a=p.parse_args()
    durable=pathlib.Path(os.environ['OUT'])/'render-gate'/a.tag/a.source
    durable.mkdir(parents=True,exist_ok=False)
    native=pathlib.Path(tempfile.mkdtemp(prefix='amazing-render-gate-'))
    phases=[];logs=[];receipts=[];results={};exit_code=0
    def run(label,argv,copies=()):
        nonlocal exit_code
        log=durable/(label+'.log');receipt=durable/(label+'.phase.manifest.json')
        r=command.run(argv,label,a.source,log,receipt,copies)
        phases.append(r);logs.append(str(log));receipts.append(str(receipt))
        if r['wrapper_exit']: exit_code=1
        return r
    for key in list(os.environ):
        if key in ('AMAZING_JS_VM','AMAZING_JS_VM_COMPILE_LOG','AMAZING_JS_VM_REJECT_CONTEXT'): os.environ.pop(key)
    if a.workspace:
        run('nextest',['cargo','nextest','run','--workspace','--locked','--no-fail-fast','--status-level','skip'])
        run('doctests',['cargo','test','--workspace','--locked','--doc'])
    wpt=run('ensure-wpt',['bash','scripts/ensure-wpt.sh'])
    build=run('build',['cargo','build','-p','amazing','--locked']+(['--release'] if a.release else []))
    exe=pathlib.Path(os.environ['CARGO_TARGET_DIR'])/('release' if a.release else 'debug')/'amazing'
    if not wpt['wrapper_exit'] and not build['wrapper_exit']:
        for subdir in a.subdirs:
            slug=subdir.replace('/','-');out=native/slug
            argv=[sys.executable,'scripts/wpt_render_compare.py','--wpt-root',str(pathlib.Path.home()/'corpora/wpt'),'--subdir',subdir,'--exe',str(exe),'--out',str(out),'--width',str(a.width),'--height',str(a.height),'--timeout',str(a.timeout),'--jobs',str(a.jobs),'--limit',str(a.limit),'--compact']
            baseline=None
            if a.baseline_ref:
                baseline=pathlib.Path(os.environ['OUT'])/'render-gate'/(a.baseline_tag or a.tag)/a.baseline_ref/(slug+'.compact.json')
            elif a.baseline:
                root=pathlib.Path('crates/amazing-shell-gpui/evidence/wpt-render' if a.baseline=='auto' else a.baseline)
                baseline=root/slug/'compact.json' if root.is_dir() else root
                if not baseline.is_file() and root.is_dir():baseline=root/slug/'report.json'
            if baseline is not None:
                if not baseline.is_file():
                    exit_code=1;results[slug]={'error':'required baseline missing','path':str(baseline)};continue
                argv+=['--baseline',str(baseline)]
            r=run(slug,argv,[(out/'compact.json',durable/(slug+'.compact.json'))])
            compact=durable/(slug+'.compact.json')
            if compact.is_file():
                data=json.loads(compact.read_text());results[slug]={'counts':data['counts'],'baseline_compare':data.get('baseline_compare'), 'raw_compact':command.describe(compact)}
    result={'schema':1,'source_commit':a.source,'job_exit':exit_code,'phase_exits':{r['phase']:r['wrapper_exit'] for r in phases},'phase_details':phases,'counts':summary.log_counts([l for l in logs if pathlib.Path(l).is_file()]),'renders':results,'raw_native_directory':str(native),'tooling':{name:command.describe(ROOT/name) for name in ('render-gate.py','gate-command.py','gate-summary.py')},'qualification':'workspace/docs and listed static pinned WPT only; no external decoded42/geometry4 capture claim'}
    summary.write(durable/'gate.summary.json',result)
    print(json.dumps({'summary':str(durable/'gate.summary.json'),'actual_exit':exit_code,'counts':result['counts']},separators=(',',':')))
    return exit_code

if __name__=='__main__':sys.exit(main())
