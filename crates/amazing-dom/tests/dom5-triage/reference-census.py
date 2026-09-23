"""REFERENCE REGISTRATION CENSUS, not Amazing conformance results. Windows only."""
from pathlib import Path
from http.server import ThreadingHTTPServer, SimpleHTTPRequestHandler
import threading, functools, subprocess, json, re, html, hashlib, shutil, time
BASE=Path('C:/Users/server/target/dom-bindings'); WPT=Path('C:/Users/server/target/dom-bindings-wpt')
OUT=BASE/'dom-5-reference-census';ROOT=OUT/'root'; OUT.mkdir(exist_ok=True)
shutil.copytree(WPT/'dom', ROOT/'dom',dirs_exist_ok=True)
shutil.copytree(WPT/'resources', ROOT/'resources',dirs_exist_ok=True)
report='''var census={tests:0,counts:{}};add_result_callback(function(t){census.tests++;census.counts[t.status]=(census.counts[t.status]||0)+1;});add_completion_callback(function(ts,s){census.harness_status=s.status;census.message=s.message;var p=document.createElement('pre');p.id='dom5-census';p.textContent=JSON.stringify(census);document.body.appendChild(p);});'''
(ROOT/'resources/testharnessreport.js').write_text(report)
class Quiet(SimpleHTTPRequestHandler):
 def log_message(self,*args):pass
server=ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Quiet,directory=str(ROOT)))
threading.Thread(target=server.serve_forever,daemon=True).start()
records=[json.loads(s) for s in (BASE/'dom-5-before/dom-1.jsonl').read_text().splitlines()]
selected=[r for r in records if r['status']=='error' and '../common.js: TypeError: cannot convert object to primitive'==r['detail']]
assert len(selected)==30
browser='C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe'
results=[]
try:
 for i,r in enumerate(selected):
  start=time.monotonic();url=f'http://127.0.0.1:{server.server_port}/'+r['test']
  profile=OUT/f'profile-{i}'
  cmd=[browser,'--headless','--no-first-run','--no-default-browser-check','--disable-extensions','--dump-dom','--virtual-time-budget=15000',f'--user-data-dir={profile}',url]
  try:
   run=subprocess.run(cmd,capture_output=True,timeout=90)
   text=run.stdout.decode('utf-8',errors='replace')
   (OUT/f'{i}.stderr.log').write_bytes(run.stderr)
   match=re.search(r'<pre id="dom5-census">(.*?)</pre>',text,re.S)
   row={'test':r['test'],'reference_census':json.loads(html.unescape(match.group(1))) if match else None,'exit_code':run.returncode,'elapsed_s':round(time.monotonic()-start,2)}
   if not match:(OUT/f'{i}.html').write_text(text,encoding='utf-8')
  except subprocess.TimeoutExpired:
   row={'test':r['test'],'reference_census':None,'error':'reference deadline 90s; count unknown'}
  results.append(row);print(json.dumps(row),flush=True)
  (OUT/'results.json').write_text(json.dumps({'kind':'REFERENCE REGISTRATION CENSUS - NOT AMAZING WPT RESULTS','browser':'Microsoft Edge 153.0.4234.48','wpt_revision':'c2a2fce69e5c2509d9dc28440ec8b42532e5c293','changes':'Only visual reporter replaced with count callback. Original test pages/common.js/testharness.js unchanged. Served over loopback HTTP, not official WPT server.','harness_sha256':hashlib.sha256((ROOT/'resources/testharness.js').read_bytes()).hexdigest(),'results':results},indent=2),encoding='utf-8')
finally:
 server.shutdown();server.server_close()
