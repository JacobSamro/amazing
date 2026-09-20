"""NON-CONFORMANCE diagnostics: instrument a COPY of pinned upstream resources.
Run only on win-001. Never modifies the WPT checkout or labels output WPT.
"""
from pathlib import Path
import concurrent.futures, hashlib, json, re, shutil, subprocess
BASE = Path('C:/Users/server/target/dom-bindings')
WPT = Path('C:/Users/server/target/dom-bindings-wpt')
EXE = BASE / 'dom-5-before/amazing.exe'
OUT = BASE / 'dom-5-setup-triage'
ROOT = OUT / 'instrumented'
EXPECTED_EXE = '9e6ddc78f3c3e75d849bb8e31b80f754a0dfaaa05ed6769323ede70499a96e4f'
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
assert sha(EXE) == EXPECTED_EXE
baseline = [json.loads(line) for line in (BASE/'dom-5-before/dom-1.jsonl').read_text().splitlines()]
selected = [r for r in baseline if r['status']=='error' and 'cannot convert object to primitive' in (r.get('detail') or '')]
assert len(selected)==41
OUT.mkdir(exist_ok=True)
shutil.copytree(WPT/'dom', ROOT/'dom', dirs_exist_ok=True)
shutil.copytree(WPT/'resources', ROOT/'resources', dirs_exist_ok=True)
harness=(WPT/'resources/testharness.js').read_text(encoding='utf-8')
needle='                this.status.message = String(e);'
assert harness.count(needle)==1
harness=harness.replace(needle, '''                __amazing_result({name:"DOM5_ORIGINAL_SETUP_EXCEPTION",status:1,message:e.name + ": " + e.message});
'''+needle)
(ROOT/'scaffold-harness.js').write_text(harness,encoding='utf-8')
common=(WPT/'dom/common.js').read_text(encoding='utf-8')
markers = [
('    testDiv = document.querySelector("#test");','QUERY_TEST'),
('        testDiv.parentNode.removeChild(testDiv);','REMOVE_OLD_TEST'),
('    testDiv = document.createElement("div");','CREATE_DIV'),
('    testDiv.id = "test";','SET_DIV_ID'),
('    document.body.insertBefore(testDiv, document.body.firstChild);','BODY_INSERT_BEFORE'),
('    paras = [];','AFTER_BODY_INSERT'),
('    const xmlDocument = new Document();','XML_DOCUMENT'),
('    paras[5].appendChild(xmlDocument.createCDATASection("1234"));','CDATA_SECTION'),
('    foreignDoc = document.implementation.createHTMLDocument("");','HTML_DOCUMENT'),
]
for source,label in markers:
 assert common.count(source)==1
 indent=source[:len(source)-len(source.lstrip())]
 common=common.replace(source,indent+'__amazing_result({name:"DOM5_TRACE_'+label+'",status:0,message:"reached"});\n'+source)
(ROOT/'dom/common.js').write_text(common,encoding='utf-8')
def script_tag(match):
 text=match.group(0)
 src=re.search(r'\bsrc\s*=\s*(?:"([^"]*)"|\x27([^\x27]*)\x27|([^\s>]+))',text,re.I)
 if not src:return text
 value=next(v for v in src.groups() if v is not None)
 if value.endswith('testharnessreport.js'):return '<!-- visual reporter omitted, as in the normal runner -->'
 if value.endswith('testharness.js'):
  return text[:src.start()]+'src="/scaffold-harness.js"'+text[src.end():]
 return text
for record in selected:
 path=ROOT/record['test']
 path.write_text(re.sub(r'<script\b[^>]*>[\s\S]*?</script\s*>',script_tag,path.read_text(encoding='utf-8'),flags=re.I),encoding='utf-8')
def one(record):
 cmd=[str(EXE),'wpt-harness','--scaffold',str(ROOT),record['test'],'800','600','10000']
 run=subprocess.run(cmd,capture_output=True,text=True,encoding='utf-8',timeout=45)
 result=json.loads(run.stdout)
 assert result['suite']=='scaffolding-only' and result['wpt_revision'] is None
 diagnostic=[s for s in result['subtests'] if s['name'].startswith('DOM5_')]
 return dict(test=record['test'],baseline_detail=record['detail'],diagnostic=diagnostic,scaffold_result=result)
results=[]
with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
 for result in pool.map(one,selected):
  results.append(result)
  with (OUT/'results.jsonl').open('a',encoding='utf-8') as out:out.write(json.dumps(result,sort_keys=True)+'\n')
  print(result['test'],json.dumps(result['diagnostic']),flush=True)
summary={'kind':'NON-CONFORMANCE setup exception diagnostic','engine_sha256':sha(EXE),'selected':len(selected),'results':results,
 'original_harness_sha256':sha(WPT/'resources/testharness.js'),'instrumented_harness_sha256':sha(ROOT/'scaffold-harness.js'),
 'original_common_sha256':sha(WPT/'dom/common.js'),'instrumented_common_sha256':sha(ROOT/'dom/common.js'),
 'changes':['Emit original setup exception name/message before existing String(e) expression; preserve String(e) and original control flow.',
 'Emit before selected common.js statements, without replacing DOM operations.',
 'Redirect copied testharness script tags to instrumented harness; omit visual reporter as normal runner does.'],
 'wpt_credit':0}
(OUT/'summary.json').write_text(json.dumps(summary,indent=2),encoding='utf-8')
