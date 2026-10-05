"""Run the complete native network corpus and Linux workspace gate in one job.
Raw output remains on the build host; only this compact classification is copied.
"""
import base64, collections, hashlib, json, os, pathlib, re, ssl, subprocess, sys, zlib
phase = sys.argv[1]
source = os.environ['COMMIT']
root = pathlib.Path(os.environ['OUT']) / ('csp-response-provenance-' + os.environ['GATE_TICKET'] + '-' + phase)
root.mkdir(parents=True, exist_ok=False)
commands = {
    'workspace': ['cargo','nextest','run','--workspace','--locked','--no-fail-fast','--status-level','all'],
    'doctests': ['cargo','test','--workspace','--locked','--doc'],
    'browser-build':['cargo','build','-p','amazing','--locked'],
    'wpt-setup':['bash','scripts/ensure-wpt.sh'],
    'csp-wpt':['python3','-c',"import base64;exec(base64.b64decode('IyEvdXNyL2Jpbi9lbnYgcHl0aG9uMwoiIiJXaG9sZSBwaW5uZWQgQ1NQIHNvdXJjZS1maWxlIGNvcnB1cyBmb3IgaW5saW5lIGJlaGF2aW9yLCB1c2luZyB0aGUgaW5zdGFsbGVkCkFtYXppbmcgV1BUIHJ1bm5lciBhbmQgaXRzIHVubW9kaWZpZWQgdXBzdHJlYW0gdGVzdGhhcm5lc3MuIFRoaXMgaXMgYSBicm93c2VyCmludGVncmF0aW9uIG1lYXN1cmVtZW50LCBzZXBhcmF0ZSBmcm9tIHRoZSBsb3dlciBwcm9kdWNlcidzIG5hdGl2ZSBjb250cm9scy4KVW5zdXBwb3J0ZWQgcGxhdGZvcm0vc2VydmVyIGdsb2JhbHMgcmVtYWluIGV4cGxpY2l0IGNsYXNzaWZpY2F0aW9ucy4KIiIiCmltcG9ydCBhcmdwYXJzZQppbXBvcnQgY29uY3VycmVudC5mdXR1cmVzCmltcG9ydCBoYXNobGliCmltcG9ydCBpbXBvcnRsaWIudXRpbAppbXBvcnQganNvbgpmcm9tIHBhdGhsaWIgaW1wb3J0IFBhdGgKCkRJUkVDVE9SSUVTID0gKCdkZWZhdWx0LXNyYycsICdzY3JpcHQtc3JjJywgJ3NjcmlwdC1zcmMtYXR0ci1lbGVtJywgJ3Vuc2FmZS1oYXNoZXMnLAogICAgICAgICAgICAgICAnbm9uY2UtaGlkaW5nJywgJ21ldGEnLCAncGFyc2luZycsICdzZWN1cml0eXBvbGljeXZpb2xhdGlvbicsCiAgICAgICAgICAgICAgICduYXZpZ2F0aW9uJywgJ3N0eWxlLXNyYycsICdzdHlsZS1zcmMtYXR0ci1lbGVtJykKCmRlZiBlbmNvZGVkKHZhbHVlKToKICAgIHJldHVybiAoanNvbi5kdW1wcyh2YWx1ZSwgc29ydF9rZXlzPVRydWUsIHNlcGFyYXRvcnM9KCcsJywgJzonKSkgKyAnXG4nKS5lbmNvZGUoKQoKZGVmIG1haW4oKToKICAgIGFwID0gYXJncGFyc2UuQXJndW1lbnRQYXJzZXIoKQogICAgYXAuYWRkX2FyZ3VtZW50KCctLWJpbmFyeScsIHR5cGU9UGF0aCwgcmVxdWlyZWQ9VHJ1ZSkKICAgIGFwLmFkZF9hcmd1bWVudCgnLS13cHQtcm9vdCcsIHR5cGU9UGF0aCwgcmVxdWlyZWQ9VHJ1ZSkKICAgIGFwLmFkZF9hcmd1bWVudCgnLS1vdXRwdXQnLCB0eXBlPVBhdGgsIHJlcXVpcmVkPVRydWUpCiAgICBhcC5hZGRfYXJndW1lbnQoJy0tc291cmNlJywgcmVxdWlyZWQ9VHJ1ZSkKICAgIGFwLmFkZF9hcmd1bWVudCgnLS1iYXNlJywgcmVxdWlyZWQ9VHJ1ZSkKICAgIGFwLmFkZF9hcmd1bWVudCgnLS1qb2JzJywgdHlwZT1pbnQsIGRlZmF1bHQ9NCkKICAgIGFwLmFkZF9hcmd1bWVudCgnLS1iYXNlbGluZScsIHR5cGU9UGF0aCkKICAgIGFwLmFkZF9hcmd1bWVudCgnLS1jb2xsZWN0b3Itc2hhMjU2JykKICAgIGFyZ3MgPSBhcC5wYXJzZV9hcmdzKCkKICAgIHJlcG8gPSBQYXRoLmN3ZCgpLnJlc29sdmUoKQogICAgc3BlYyA9IGltcG9ydGxpYi51dGlsLnNwZWNfZnJvbV9maWxlX2xvY2F0aW9uKCd3cHRfbWVhc3VyZScsIHJlcG8vJ2NyYXRlcy9hbWF6aW5nL3NyYy9jbWQvbWVhc3VyZS13cHQtZG9tLnB5JykKICAgIG1lYXN1cmUgPSBpbXBvcnRsaWIudXRpbC5tb2R1bGVfZnJvbV9zcGVjKHNwZWMpCiAgICBzcGVjLmxvYWRlci5leGVjX21vZHVsZShtZWFzdXJlKQogICAgcm9vdD1hcmdzLndwdF9yb290LnJlc29sdmUoKQogICAgYXJncy5vdXRwdXQubWtkaXIocGFyZW50cz1UcnVlLCBleGlzdF9vaz1UcnVlKQogICAgZW50cmllcz1bXTsgZXhjbHVkZWQ9W10KICAgIGZvciBkaXJlY3RvcnkgaW4gRElSRUNUT1JJRVM6CiAgICAgICAgaW52ZW50b3J5PW1lYXN1cmUuaW52ZW50b3J5KHJvb3QsICdjb250ZW50LXNlY3VyaXR5LXBvbGljeS8nK2RpcmVjdG9yeSkKICAgICAgICBlbnRyaWVzLmV4dGVuZChpbnZlbnRvcnlbJ3NlbGVjdGVkJ10pO2V4Y2x1ZGVkLmV4dGVuZChpbnZlbnRvcnlbJ2V4Y2x1ZGVkJ10pCiAgICBzZWxlY3Rpb249eyd3cHRfcmV2aXNpb24nOm1lYXN1cmUuUkVWSVNJT04sJ2RpcmVjdG9yaWVzJzpESVJFQ1RPUklFUywnc2VsZWN0ZWQnOmVudHJpZXMsJ2V4Y2x1ZGVkJzpleGNsdWRlZH0KICAgIChhcmdzLm91dHB1dC8nc2VsZWN0aW9uLmpzb24nKS53cml0ZV9ieXRlcyhlbmNvZGVkKHNlbGVjdGlvbikpCiAgICBkZWYgcnVuKGVudHJ5KToKICAgICAgICByZXR1cm4gbWVhc3VyZS5ydW4oYXJncy5iaW5hcnkucmVzb2x2ZSgpLHJvb3QsZW50cnksNjAwMDAgaWYgZW50cnlbJ2xvbmdfdGltZW91dCddIGVsc2UgMTAwMDApCiAgICB3aXRoIGNvbmN1cnJlbnQuZnV0dXJlcy5UaHJlYWRQb29sRXhlY3V0b3IobWF4X3dvcmtlcnM9YXJncy5qb2JzKSBhcyBwb29sOgogICAgICAgIHJlY29yZHM9W10KICAgICAgICBmb3IgcmVzdWx0IGluIHBvb2wubWFwKHJ1bixlbnRyaWVzKToKICAgICAgICAgICAgcmVjb3Jkcy5hcHBlbmQocmVzdWx0KQogICAgICAgICAgICBpZiBsZW4ocmVjb3JkcyklMjU9PTA6IHByaW50KGYnQ1NQIHtsZW4ocmVjb3Jkcyl9L3tsZW4oZW50cmllcyl9JyxmbHVzaD1UcnVlKQogICAgcmF3PWInJy5qb2luKGVuY29kZWQocmVjb3JkKSBmb3IgcmVjb3JkIGluIHJlY29yZHMpCiAgICByYXdfcGF0aD1hcmdzLm91dHB1dC8ncmVzdWx0cy5qc29ubCc7cmF3X3BhdGgud3JpdGVfYnl0ZXMocmF3KQogICAgcGFzc2VkPXtyWyd0ZXN0J10gZm9yIHIgaW4gcmVjb3JkcyBpZiByWydzdGF0dXMnXT09J3Bhc3MnfQogICAgIyBJbnN0YWxsZWQgdXBzdHJlYW0gdGVzdGhhcm5lc3Mgc2VyaWFsaXplcyBzdWJ0ZXN0IHN0YXR1c2VzIGFzIGludGVnZXIgY29kZXM6CiAgICAjIFBBU1MgaXMgbnVtZXJpYyAwLiBGaWxlIGNsYXNzaWZpY2F0aW9ucyByZW1haW4gc3RyaW5ncywgc2VwYXJhdGVseS4KICAgIGRlZiBwYXNzZWRfc3VidGVzdChzdWJ0ZXN0KToKICAgICAgICByZXR1cm4gdHlwZShzdWJ0ZXN0WydzdGF0dXMnXSkgaXMgaW50IGFuZCBzdWJ0ZXN0WydzdGF0dXMnXSA9PSAwCiAgICBmb3IgcmVjb3JkIGluIHJlY29yZHM6CiAgICAgICAgYWN0dWFsID0gc3VtKHBhc3NlZF9zdWJ0ZXN0KHN1YnRlc3QpIGZvciBzdWJ0ZXN0IGluIHJlY29yZFsnc3VidGVzdHMnXSkKICAgICAgICBpZiBhY3R1YWwgIT0gcmVjb3JkWydjb3VudHMnXVsncGFzcyddOgogICAgICAgICAgICByYWlzZSBSdW50aW1lRXJyb3IoJ1Bhc3Npbmcgc3VidGVzdCBjb3VudC9pZGVudGl0eSBpbmNvbnNpc3RlbmN5OiAnK3JlY29yZFsndGVzdCddKQogICAgcGFzc19zdWJ0ZXN0cz17clsndGVzdCddKyc6Oicrc3RyKGluZGV4KSsnOjonK3NbJ25hbWUnXSBmb3IgciBpbiByZWNvcmRzIGZvciBpbmRleCxzIGluIGVudW1lcmF0ZShyWydzdWJ0ZXN0cyddKSBpZiBwYXNzZWRfc3VidGVzdChzKX0KICAgIGlmIGxlbihwYXNzX3N1YnRlc3RzKSAhPSBzdW0ocmVjb3JkWydjb3VudHMnXVsncGFzcyddIGZvciByZWNvcmQgaW4gcmVjb3Jkcyk6CiAgICAgICAgcmFpc2UgUnVudGltZUVycm9yKCdEdXBsaWNhdGUgcGFzc2luZyBzdWJ0ZXN0IGlkZW50aXRpZXMnKQogICAgY29tcGFjdD17J3NvdXJjZSc6YXJncy5zb3VyY2UsJ2Jhc2UnOmFyZ3MuYmFzZSwnd3B0X3JldmlzaW9uJzptZWFzdXJlLlJFVklTSU9OLAogICAgICAgICdjb2xsZWN0b3Jfc2hhMjU2JzphcmdzLmNvbGxlY3Rvcl9zaGEyNTYgb3IgaGFzaGxpYi5zaGEyNTYoUGF0aChfX2ZpbGVfXykucmVhZF9ieXRlcygpKS5oZXhkaWdlc3QoKSwKICAgICAgICAnZGlyZWN0b3JpZXMnOkRJUkVDVE9SSUVTLCdiaW5hcnlfc2hhMjU2JzpoYXNobGliLnNoYTI1NihhcmdzLmJpbmFyeS5yZWFkX2J5dGVzKCkpLmhleGRpZ2VzdCgpLAogICAgICAgICdyYXdfcGF0aCc6c3RyKHJhd19wYXRoKSwncmF3X3NoYTI1Nic6aGFzaGxpYi5zaGEyNTYocmF3KS5oZXhkaWdlc3QoKSwKICAgICAgICAnc2VsZWN0aW9uX3NoYTI1Nic6aGFzaGxpYi5zaGEyNTYoZW5jb2RlZChzZWxlY3Rpb24pKS5oZXhkaWdlc3QoKSwKICAgICAgICAnaGFybmVzc19zaGEyNTYnOmhhc2hsaWIuc2hhMjU2KChyb290LydyZXNvdXJjZXMvdGVzdGhhcm5lc3MuanMnKS5yZWFkX2J5dGVzKCkpLmhleGRpZ2VzdCgpLAogICAgICAgICdjb3VudHMnOntzdGF0dXM6c3VtKHJbJ3N0YXR1cyddPT1zdGF0dXMgZm9yIHIgaW4gcmVjb3JkcykgZm9yIHN0YXR1cyBpbiBtZWFzdXJlLkZJTEVfU1RBVFVTRVN9LAogICAgICAgICdzdWJ0ZXN0cyc6e3N0YXR1czpzdW0oclsnY291bnRzJ11bc3RhdHVzXSBmb3IgciBpbiByZWNvcmRzKSBmb3Igc3RhdHVzIGluIG1lYXN1cmUuU1VCVEVTVF9TVEFUVVNFU30sCiAgICAgICAgJ2NsYXNzaWZpY2F0aW9ucyc6e3JbJ3Rlc3QnXTpyWydzdGF0dXMnXSBmb3IgciBpbiByZWNvcmRzfSwKICAgICAgICAncGFzc2luZ19maWxlcyc6c29ydGVkKHBhc3NlZCksJ3Bhc3Npbmdfc3VidGVzdHMnOnNvcnRlZChwYXNzX3N1YnRlc3RzKX0KICAgIGlmIGFyZ3MuYmFzZWxpbmU6CiAgICAgICAgYmVmb3JlPWpzb24ubG9hZHMoYXJncy5iYXNlbGluZS5yZWFkX3RleHQoKSkKICAgICAgICBpZiBiZWZvcmVbJ3NlbGVjdGlvbl9zaGEyNTYnXSE9Y29tcGFjdFsnc2VsZWN0aW9uX3NoYTI1NiddOnJhaXNlIFJ1bnRpbWVFcnJvcignU2VsZWN0aW9uIGRyaWZ0JykKICAgICAgICBmb3IgdW5pdCxhZnRlciBpbiBbKCdmaWxlcycscGFzc2VkKSwoJ3N1YnRlc3RzJyxwYXNzX3N1YnRlc3RzKV06CiAgICAgICAgICAgIG9sZD1zZXQoYmVmb3JlWydwYXNzaW5nXycrdW5pdF0pO2NvbXBhY3RbJ2xvc3RfJyt1bml0XT1zb3J0ZWQob2xkLWFmdGVyKTtjb21wYWN0WydnYWluZWRfJyt1bml0XT1zb3J0ZWQoYWZ0ZXItb2xkKQogICAgZGF0YT1lbmNvZGVkKGNvbXBhY3QpCiAgICBpZiBsZW4oZGF0YSk+NTEyKjEwMjQ6cmFpc2UgUnVudGltZUVycm9yKCdDb21wYWN0IGV2aWRlbmNlIGV4Y2VlZHMgcG9saWN5OyBzcGxpdCBwYXNzIHNldHMgd2l0aG91dCBsb3NpbmcgaWRlbnRpdGllcycpCiAgICAoYXJncy5vdXRwdXQvJ2NvbXBhY3QuanNvbicpLndyaXRlX2J5dGVzKGRhdGEpCiAgICBwcmludChqc29uLmR1bXBzKHtrOmNvbXBhY3Rba10gZm9yIGsgaW4gKCdjb3VudHMnLCdzdWJ0ZXN0cycsJ3Jhd19zaGEyNTYnKX0pLGZsdXNoPVRydWUpCiAgICBpZiBhbnkoY29tcGFjdC5nZXQoJ2xvc3RfJyt1bml0KSBmb3IgdW5pdCBpbiAoJ2ZpbGVzJywnc3VidGVzdHMnKSk6cmFpc2UgU3lzdGVtRXhpdCgxKQppZiBfX25hbWVfXz09J19fbWFpbl9fJzptYWluKCkK'))",'--collector-sha256','d2db31f0941e84548e572a09c65670260a937d5fd9116bc8930b2cd5f7983172','--binary',str(pathlib.Path(os.environ['CARGO_TARGET_DIR'])/'debug'/'amazing'),'--wpt-root',str(pathlib.Path.home()/'corpora'/'wpt'),'--output',str(root/'csp-wpt'),'--source',source,'--base','9de83d4a16f04e9beeb550a5405701b3e1755e63','--jobs',os.environ.get('JOBS','4')] + (['--baseline',str(pathlib.Path(os.environ['OUT'])/('csp-response-provenance-'+os.environ['GATE_TICKET']+'-before')/'csp-wpt'/'compact.json')] if phase=='after' else []),
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
result = {'cargo_lock_sha256':hashlib.sha256(pathlib.Path('Cargo.lock').read_bytes()).hexdigest(),'schema':1,'source_commit':source,'base_commit':'9de83d4a16f04e9beeb550a5405701b3e1755e63','phase':phase,'fixture_environment':{'python':sys.version,'ssl':ssl.OPENSSL_VERSION},'suites':results}
lost = []
if phase == 'after':
    before_path = pathlib.Path(os.environ['OUT']) / ('csp-response-provenance-' + os.environ['GATE_TICKET'] + '-before') / 'before.summary.json'
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
