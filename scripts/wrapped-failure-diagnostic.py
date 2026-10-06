#!/usr/bin/env python3
"""Bound compiler/assertion diagnostics to an original terminal and phase receipt.

No engine run, raw transfer, reconstructed receipt/classification/count or archive
recovery. Only source/phase/hash-verified retained logs are read natively.
"""
import argparse, importlib.util, json, pathlib, re
ROOT = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('gate_failure', ROOT / 'gate-failure-diagnostic.py')
checked = importlib.util.module_from_spec(spec); spec.loader.exec_module(checked)
MARKER = re.compile(r'(?:^error(?:\[[^]]+\])?:|^Traceback|^\w*(?:Error|Exception):|panicked at|assertion .*failed|\bFAIL(?:ED)?\b)')

def produce(terminal_path, terminal_sha, receipt_path, receipt_sha, source, job, owner):
    if not re.fullmatch('[a-f0-9]{40}', source) or not re.fullmatch('[a-f0-9]{32}', job):
        raise ValueError('exact source/job required')
    owner = pathlib.Path(owner)
    if not owner.is_absolute() or owner.resolve() != owner or not owner.is_relative_to('/tmp'):
        raise ValueError('canonical native /tmp owner required')
    receipt_path = pathlib.Path(receipt_path)
    if not receipt_path.is_relative_to(owner): raise ValueError('phase receipt escapes declared owner')
    terminal = json.loads(checked.checked_file(terminal_path, terminal_sha, checked.MAX_COMPLETION))
    endpoint, actual, terminal_format = checked.completed_job.diagnostic_completion(terminal, source, job)
    receipt = json.loads(checked.checked_file(receipt_path, receipt_sha, 524288))
    code = receipt.get('command_exit')
    if (receipt.get('source_commit') != source or not isinstance(receipt.get('phase'), str)
            or type(code) is not int or code == 0 or receipt.get('engine_exit') != code
            or type(receipt.get('wrapper_exit')) is not int or receipt['wrapper_exit'] == 0):
        raise ValueError('original genuinely failed wrapped phase required')
    raw = receipt['raw_native']
    if not pathlib.Path(raw['path']).is_relative_to('/tmp') or type(raw['bytes']) is not int:
        raise ValueError('retained native raw binding required')
    data = checked.checked_file(raw['path'], raw['sha256'], checked.MAX_RAW, raw['bytes'])
    lines = checked.ANSI.sub('', data.decode('utf-8', errors='replace')).splitlines()
    hits = [i for i, line in enumerate(lines) if MARKER.search(line)]
    if not hits: raise ValueError('no actual compiler/assertion marker; no diagnostic invented')
    selected = sorted({j for i in hits for j in range(max(0,i-3), min(len(lines),i+12))})
    snippets = []; used = 0; omitted = 0
    for index in selected:
        body = lines[index].encode(); text = body[:2048].decode('utf-8', errors='ignore')
        row = {'line':index+1,'text':text,'marker':index in hits,'truncated':len(body)>2048}
        cost = len(json.dumps(row).encode())
        if used+cost>48*1024: omitted += 1; continue
        snippets.append(row);used += cost
    result = {'schema':1,'diagnostic_only':True,'source_commit':source,'original_job':endpoint,
              'original_actual_exit':actual,'original_completion_format':terminal_format,'phase':receipt['phase'],'phase_actual_exit':code,
              'phase_receipt':{'path':str(receipt_path),'sha256':receipt_sha},'terminal_sha256':terminal_sha,
              'phase_command':receipt.get('command'),'raw':raw,'snippets':snippets,
              'omitted_context_lines':omitted,'tests_rerun':False,'counts_reconstructed':False}
    body = (json.dumps(result, sort_keys=True)+'\n').encode()
    if len(body)>65536: raise ValueError('diagnostic exceeds64KiB')
    return body

def main():
    p=argparse.ArgumentParser()
    for name in ['terminal','terminal-sha256','phase-receipt','phase-receipt-sha256','source','job','native-owner','output']:
        p.add_argument('--'+name,required=True)
    a=p.parse_args();data=produce(a.terminal,a.terminal_sha256,a.phase_receipt,a.phase_receipt_sha256,a.source,a.job,a.native_owner)
    out=pathlib.Path(a.output)
    if not out.is_absolute() or out.parent.resolve()!=out.parent:raise ValueError('fresh canonical output required')
    with out.open('xb') as stream:stream.write(data)
    print(json.dumps({'bytes':len(data),'sha256':checked.digest(data),'diagnostic_only':True}))
if __name__=='__main__':main()
