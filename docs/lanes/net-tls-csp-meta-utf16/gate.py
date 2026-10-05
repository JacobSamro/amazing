"""Run the complete native network corpus and Linux workspace gate in one job.
Raw output remains on the build host; only this compact classification is copied.
"""
import base64, collections, hashlib, json, os, pathlib, re, ssl, subprocess, sys, zlib
phase = sys.argv[1]
source = os.environ['COMMIT']
root = pathlib.Path(os.environ['OUT']) / ('csp-meta-utf16-' + os.environ['GATE_TICKET'] + '-' + phase)
root.mkdir(parents=True, exist_ok=False)
commands = {
    'workspace': ['cargo','nextest','run','--workspace','--locked','--no-fail-fast','--status-level','all'],
    'doctests': ['cargo','test','--workspace','--locked','--doc'],
    'browser-build':['cargo','build','-p','amazing','--locked'],
    'wpt-setup':['bash','scripts/ensure-wpt.sh'],
    'csp-wpt':['python3','crates/amazing-fetch/tests/support/csp_inline_wpt.py','--binary',str(pathlib.Path(os.environ['CARGO_TARGET_DIR'])/'debug'/'amazing'),'--wpt-root',str(pathlib.Path.home()/'corpora'/'wpt'),'--output',str(root/'csp-wpt'),'--source',source,'--base','a035b2b558065d72664ce7617da0a7261027bb0d','--jobs',os.environ.get('JOBS','4')] + (['--baseline',str(pathlib.Path(os.environ['OUT'])/('csp-meta-utf16-'+os.environ['GATE_TICKET']+'-before')/'csp-wpt'/'compact.json')] if phase=='after' else []),
    'network-corpus': ['cargo','nextest','run','-p','amazing-net-tls','-p','amazing-net','-p','amazing-fetch','--features','amazing-net-tls/test-support','--locked','--no-fail-fast','--status-level','all'],
}
results = {}
for name, command in commands.items():
    log = root / (name + '.log')
    with log.open('wb') as stream:
        code = subprocess.run(command,stdout=stream,stderr=subprocess.STDOUT,env=dict(os.environ,CARGO_TERM_COLOR='never')).returncode
    output = log.read_text(errors='replace')
    statuses = {}
    for line in output.splitlines():
        match = re.search(r'^\s*(PASS|FAIL|TIMEOUT|SKIP)\s+\[[^]]+\]\s+(.+)$',line)
        if match:
            test_id = re.sub(r'^\(\s*\d+/\d+\)\s+', '', match[2].strip())
            statuses[test_id] = match[1].lower()
    summaries = [line.strip() for line in output.splitlines() if 'Summary' in line or line.startswith('test result:')]
    doc_counts = {'pass':sum(int(n) for n in re.findall(r'test result: .*? (\d+) passed;',output)), 'fail':sum(int(n) for n in re.findall(r'test result: .*?; (\d+) failed;',output))} if name=='doctests' else None
    results[name] = {'doctest_counts':doc_counts,'counts':dict(collections.Counter(statuses.values())),'exit':code,'command':command,'classifications':statuses,'summaries':summaries,'raw':{'path':str(log),'sha256':hashlib.sha256(log.read_bytes()).hexdigest(),'bytes':log.stat().st_size}}
    print(name,code,*summaries,sep='\n',flush=True)
    if code:
        print(output[-6000:],flush=True)
wpt_compact = root/'csp-wpt'/'compact.json'
if wpt_compact.exists():
    results['csp-wpt']['compact'] = json.loads(wpt_compact.read_text())
result = {'cargo_lock_sha256':hashlib.sha256(pathlib.Path('Cargo.lock').read_bytes()).hexdigest(),'schema':1,'source_commit':source,'base_commit':'a035b2b558065d72664ce7617da0a7261027bb0d','phase':phase,'fixture_environment':{'python':sys.version,'ssl':ssl.OPENSSL_VERSION},'suites':results}
lost = []
if phase == 'after':
    before_path = pathlib.Path(os.environ['OUT']) / ('csp-meta-utf16-' + os.environ['GATE_TICKET'] + '-before') / 'before.summary.json'
    before = json.loads(before_path.read_text())
    def classes(suite):
        if 'classifications' in suite: return suite['classifications']
        return json.loads(zlib.decompress(base64.b64decode(suite['classifications_zlib_base64'])))
    for name in ('workspace','network-corpus'):
        old = {key for key,value in classes(before['suites'][name]).items() if value=='pass'}
        new = {key for key,value in classes(results[name]).items() if value=='pass'}
        results[name]['lost'] = sorted(old-new)
        results[name]['gained'] = sorted(new-old)
        lost.extend(name+'::'+key for key in old-new)
    for unit in ('files','subtests'):
        lost.extend('csp-wpt::'+unit+'::'+key for key in results.get('csp-wpt',{}).get('compact',{}).get('lost_'+unit,[]))
    result['lost'] = sorted(lost)
data = (json.dumps(result,separators=(',',':'),sort_keys=True)+'\n').encode()
if len(data) > 64*1024:
    for suite in result['suites'].values():
        suite['classifications_zlib_base64'] = base64.b64encode(zlib.compress(json.dumps(suite.pop('classifications'),separators=(',',':'),sort_keys=True).encode(),9)).decode()
    data = (json.dumps(result,separators=(',',':'),sort_keys=True)+'\n').encode()
    if len(data) > 512*1024:
        raise SystemExit('evidence too large; raw stays remote')
(root / (phase+'.summary.json')).write_bytes(data)
raise SystemExit(bool(lost) or any(r['exit'] for r in results.values()))
