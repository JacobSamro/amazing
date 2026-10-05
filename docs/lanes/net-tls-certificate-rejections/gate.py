"""Run the complete native network corpus and Linux workspace gate in one job.
Raw output remains on the build host; only this compact classification is copied.
"""
import base64, hashlib, json, os, pathlib, re, subprocess, sys, zlib
phase = sys.argv[1]
source = os.environ['COMMIT']
root = pathlib.Path(os.environ['OUT']) / ('certificate-rejections-' + phase)
root.mkdir(parents=True, exist_ok=False)
commands = {
    'workspace': ['cargo','nextest','run','--workspace','--locked','--no-fail-fast','--status-level','all'],
    'doctests': ['cargo','test','--workspace','--locked','--doc'],
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
    results[name] = {'exit':code,'command':command,'classifications':statuses,'summaries':summaries,'raw':{'path':str(log),'sha256':hashlib.sha256(log.read_bytes()).hexdigest(),'bytes':log.stat().st_size}}
    print(name,code,*summaries,sep='\n',flush=True)
    if code:
        print(output[-6000:],flush=True)
result = {'schema':1,'source_commit':source,'base_commit':'ccbe84327b1e177cdf05bc4d9b6ce27b433e4b5f','phase':phase,'suites':results}
data = (json.dumps(result,separators=(',',':'),sort_keys=True)+'\n').encode()
if len(data) > 512*1024:
    for suite in result['suites'].values():
        suite['classifications_zlib_base64'] = base64.b64encode(zlib.compress(json.dumps(suite.pop('classifications'),separators=(',',':'),sort_keys=True).encode(),9)).decode()
    data = (json.dumps(result,separators=(',',':'),sort_keys=True)+'\n').encode()
    if len(data) > 512*1024:
        raise SystemExit('evidence too large; raw stays remote')
(root / (phase+'.summary.json')).write_bytes(data)
raise SystemExit(any(r['exit'] for r in results.values()))
