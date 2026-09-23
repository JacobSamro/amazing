"""NON-CONFORMANCE call-site tracing on diagnostic copies; Windows only."""
from pathlib import Path
import re,json,subprocess,concurrent.futures,hashlib,shutil
BASE=Path('C:/Users/server/target/dom-bindings');WPT=Path('C:/Users/server/target/dom-bindings-wpt')
ROOT=BASE/'dom-5-call-triage/instrumented';OUT=BASE/'dom-5-call-triage';OUT.mkdir(exist_ok=True)
if ROOT.exists():
 raise RuntimeError('Refusing to overwrite an existing diagnostic copy')
shutil.copytree(BASE/'dom-5-setup-triage/instrumented', ROOT)
EXE=BASE/'dom-5-before/amazing.exe'
assert hashlib.sha256(EXE.read_bytes()).hexdigest()=='9e6ddc78f3c3e75d849bb8e31b80f754a0dfaaa05ed6769323ede70499a96e4f'
# Skip quoted strings, templates and comments. Only identifier member calls are
# rewritten; the receiver stays a Reference so its `this` binding is preserved.
pattern=re.compile(r'''("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`|//[^\n]*|/\*[\s\S]*?\*/)|\.([A-Za-z_$][\w$]*)\s*\(''')
def trace(code,label):
 def change(m):
  if m.group(1):return m.group(0)
  method=m.group(2);line=code.count('\n',0,m.start())+1
  previous=re.search(r'[A-Za-z_$][\w.$\[\]]*$',code[:m.start()])
  receiver=previous.group(0) if previous else '(expression)'
  detail=f'{label}:{line} {receiver}.{method}'
  return '[(__amazing_result({name:"DOM5_CALL",status:0,message:'+json.dumps(detail)+'}),'+json.dumps(method)+')]('
 return pattern.sub(change,code)
# Copy original DOM JS resources and page scripts, keeping setup diagnostic
# harness and page harness redirects from the earlier instrumented root.
for src in (WPT/'dom').rglob('*.js'):
 rel=src.relative_to(WPT);(ROOT/rel).write_text(trace(src.read_text(encoding='utf-8'),str(rel).replace('\\','/')),encoding='utf-8')
records=[json.loads(s) for s in (BASE/'dom-5-before/dom-1.jsonl').read_text().splitlines()]
selected=[r for r in records if r['status']=='error' and ('value is not callable' in (r['detail'] or '') or ('cannot convert object to primitive' in (r['detail'] or '') and '../common.js:' not in r['detail']))]
def script_tag(m,label):
 tag,body,close=m.groups()
 src=re.search(r'\bsrc\s*=\s*(?:"([^"]*)"|\x27([^\x27]*)\x27|([^\s>]+))',tag,re.I)
 if src:
  value=next(v for v in src.groups() if v is not None)
  if value.endswith('testharnessreport.js'):return ''
  if value.endswith('testharness.js'):tag=tag[:src.start()]+'src="/scaffold-harness.js"'+tag[src.end():]
  return tag+body+close
 return tag+trace(body,label)+close
for r in selected:
 src=(WPT/r['test']).read_text(encoding='utf-8')
 (ROOT/r['test']).write_text(re.sub(r'(<script\b[^>]*>)([\s\S]*?)(</script\s*>)',lambda m:script_tag(m,r['test']),src,flags=re.I),encoding='utf-8')
def one(r):
 proc=subprocess.run([str(EXE),'wpt-harness','--scaffold',str(ROOT),r['test'],'800','600','10000'],capture_output=True,encoding='utf-8',timeout=45)
 result=json.loads(proc.stdout);assert result['suite']=='scaffolding-only'
 calls=[s['message'] for s in result['subtests'] if s['name']=='DOM5_CALL']
 errors=[s['message'] for s in result['subtests'] if s['name']=='DOM5_ORIGINAL_SETUP_EXCEPTION']
 original=[s for s in result['subtests'] if not s['name'].startswith('DOM5_')]
 changed_parse='SyntaxError:' in (result['detail'] or '') and 'SyntaxError:' not in (r['detail'] or '')
 return {'test':r['test'],'baseline_detail':r['detail'],'trace_count':len(calls),'last_calls':calls[-12:],'original_setup_errors':errors,'diagnostic_detail':result['detail'],'diagnostic_status':result['status'],'changed_parse':changed_parse,'recorded_test_results':len(original),'original_results':original,'trace':calls}
results=[]
with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
 for r in pool.map(one,selected):
  results.append(r);print(r['test'],json.dumps(r['last_calls'][-3:]),r['diagnostic_detail'],flush=True)
(OUT/'results.json').write_text(json.dumps({'kind':'NON-CONFORMANCE member call-site trace','selected':len(selected),'exe_sha256':hashlib.sha256(EXE.read_bytes()).hexdigest(),'instrumentation':'Identifier member-call keys evaluate a native trace before looking up the original method, preserving receiver Reference/this. Strings/templates/comments are skipped; this is diagnostic transformation, never a WPT result.','results':results},indent=2),encoding='utf-8')
