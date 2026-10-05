#!/usr/bin/env python3
"""Public paired render/CSS collector; raw stays native, compact has an explicit owner."""
import base64,hashlib,importlib.util,json,os,pathlib,subprocess,sys,collections,re
import argparse
root=pathlib.Path(__file__).resolve().parent
def module(name,file):
 s=importlib.util.spec_from_file_location(name,root/file);m=importlib.util.module_from_spec(s);s.loader.exec_module(m);return m
cmp=module('compare','wpt_render_compare.py');gc=module('gate_command','gate-command.py')
def discover_metadata(wpt,cohorts):
 wpt=pathlib.Path(wpt).resolve()
 selection={};raw_bound=0
 for row in cohorts:
  selected,inventory=cmp.discover(wpt,row['cohort'],0)
  ids=sorted(re.sub(r'[^A-Za-z0-9_.\-]','__',c['path']) for c in selected)
  idhash=hashlib.sha256(json.dumps(ids,separators=(',',':')).encode()).hexdigest()
  historical_note={'expected_selection_ids_sha256':row.get('selection_ids_sha256'),'actual_selection_ids_sha256':idhash,'historical_projection_matches':idhash==row.get('selection_ids_sha256'),'nonblocking':True}
  inputs=[]
  for c in selected:
   refs=[{'relation':l['relation'],'href':l['href'],'sha256':cmp.sha256_file(l['reference_path'])} for l in c['links']]
   inputs.append({'id':c['path'],'sha256':cmp.sha256_file(c['abs_path']),'references':refs})
   n=len(refs);raw_bound+=(1+2*n)*(800*600*3+64)+(1+n)*65536+32768
  selection[row['cohort']]={'selected':len(ids),'selection_ids_sha256':idhash,'input_sha256':hashlib.sha256(json.dumps(inputs,separators=(',',':'),sort_keys=True).encode()).hexdigest(),'inventory':inventory,'historical_census_note':historical_note}
 return selection,raw_bound
def main():
 p=argparse.ArgumentParser(description='Historical census notes never block; actual renderer loss is authoritative.')
 p.add_argument('--config-file',required=True);args=p.parse_args()
 conf=json.loads(pathlib.Path(args.config_file).read_text())
 source=conf['source'];role=conf['corpus_role'];tag=conf['job'];out=pathlib.Path(conf['native_output']);os.environ['OUT']=str(out)
 compact=pathlib.Path.home()/conf['compact_root'];compact.mkdir(parents=True,exist_ok=False)
 def write(name,value):
  b=(json.dumps(value,separators=(',',':'),sort_keys=True)+'\n').encode()
  if len(b)>524288:raise ValueError('compact exceeds512KiB')
  (compact/name).write_bytes(b)
 subprocess.run(['bash','scripts/ensure-wpt.sh'],check=True)
 wpt=(pathlib.Path(os.environ['NATIVE_CORPORA_ROOT'])/'wpt').resolve()
 pin=subprocess.check_output(['git','-C',str(wpt),'rev-parse','HEAD'],text=True).strip()
 if pin!=conf['pin']:raise ValueError('pinned WPT differs')
 selection,raw_bound=discover_metadata(wpt,conf['cohorts'])
 manifest={'schema':1,'source':source,'corpus_role':role,'job':tag,'profile':conf['profile'],'pin':pin,'viewport':conf['viewport'],'selection':selection,'raw_envelope_bytes':raw_bound,'raw_budget_bytes':conf['raw_budget_bytes'],'storage_lifecycle_phase':'before','custody':{'native_output_root':str(out),'compact_owner':str(compact),'finite_ttl_seconds':7200,'root_hold_required_until_consumed':True,'no_cross_box_elf_verification':True}}
 write('corpus.manifest.json',manifest)
 if raw_bound>conf['raw_budget_bytes']:raise ValueError('discovered raw envelope exceeds admitted budget before engine')
 baseline=conf.get('baseline_source','');baseline_tag=conf.get('baseline_tag','')
 if role=='after':
  b=conf['baseline_manifest']
  if b['source']!=baseline or b['corpus_role']!='before' or any(b[k]!=manifest[k] for k in ['profile','pin','viewport']):raise ValueError('matching BEFORE corpus manifest differs')
  for name,payload in conf['baseline_compacts'].items():
   raw=base64.b64decode(payload['base64'])
   if hashlib.sha256(raw).hexdigest()!=payload['sha256']:raise ValueError('authenticated baseline bytes changed')
   dest=out/'render-gate'/baseline_tag/baseline/name;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(raw)
 argv=[sys.executable,'scripts/render-gate.py','--source',source,'--tag',tag,'--width','800','--height','600','--timeout','20','--jobs','4','--limit','0']
 if conf['workspace_docs']:argv+=['--workspace']
 if role=='after':argv+=['--baseline-ref',baseline,'--baseline-tag',baseline_tag]
 render_exit=subprocess.run(argv+[r['cohort'] for r in conf['cohorts']]).returncode
 native=out/'render-gate'/tag/source
 summary=json.loads((native/'gate.summary.json').read_text());failed=bool(render_exit)
 if conf['css17']:
  os.environ['WPT_ROOT']=str(wpt)
  selections='css-logical css-color css-values css-cascade css-conditional css-fonts css-text css-backgrounds css-transforms css-flexbox css-grid css-images css-sizing css-contain css-display selectors css-shadow'.split()
  rr=gc.run(['cargo','run','-p','amazing-css','--example','wpt_conformance','--locked','--']+selections,'parsing',source,native/'parsing.log',native/'parsing.phase.manifest.json')
  text=pathlib.Path(rr['raw_native']['path']).read_text()
  def identity(line):
   line=line.strip()
   if ': computed ' in line and ' -> ' in line and ', expected ' in line:return line.split(' -> ',1)[0]+', expected '+line.rsplit(', expected ',1)[1]
   return line
  failures=collections.Counter(identity(l) for l in text.splitlines() if l.strip().startswith('FAIL '));per={}
  for m in re.finditer(r'^([\w-]+): (\d+)/(\d+) passed \((\d+) files, (\d+) non-literal args skipped\)',text,re.M):
   name,passed,total,files,skipped=m.groups();per[name]=dict(zip(['passed','total','files','skipped_non_literal'],map(int,[passed,total,files,skipped])))
  m=re.search(r'TOTAL: (\d+)/(\d+) passed across (\d+) files \((\d+) non-literal args skipped\)',text)
  parsing={'source':source,'corpus_role':role,'selections':selections,'per_selection':per,'failure_multiset':dict(sorted(failures.items())),'phase_detail':rr,'actual_exit':rr['wrapper_exit']}
  pfail=bool(rr['wrapper_exit'] or not m or set(per)!=set(selections))
  if m:parsing.update(zip(['passed','total','files','skipped_non_literal'],map(int,m.groups())))
  if role=='after':
   b=json.loads((out/'render-gate'/baseline_tag/baseline/'parsing.summary.json').read_text());lost=failures-collections.Counter(b['failure_multiset']);gained=collections.Counter(b['failure_multiset'])-failures
   same=b['selections']==selections and b.get('total')==parsing.get('total') and all({k:v for k,v in b['per_selection'][n].items() if k!='passed'}=={k:v for k,v in per[n].items() if k!='passed'} for n in selections)
   parsing['comparison']={'new_failures':dict(lost),'removed_failures':dict(gained),'same_inventory':same};pfail=pfail or bool(lost) or not same
  write('parsing.summary.json',parsing);write('parsing.phase.manifest.json',rr);failed=failed or pfail;summary['parsing']=gc.describe(compact/'parsing.summary.json')
 summary['job_exit']=int(failed);summary['corpus_manifest']=gc.describe(compact/'corpus.manifest.json');exe=pathlib.Path(os.environ['CARGO_TARGET_DIR'])/'debug/amazing';summary['native_executable']=gc.describe(exe) if exe.is_file() else {'path':str(exe),'missing':True};summary['corpus_role']=role;summary['selection_comparison_note']={'nonblocking':True,'matching_inventory':role!='after' or conf['baseline_manifest']['selection']==selection}
 for slug,row in summary['renders'].items():
  if 'raw_compact' in row:
   p=native/(slug+'.compact.json');value=json.loads(p.read_text());write(slug+'.compact.json',value)
 write('gate.summary.json',summary)
 print(json.dumps({'source':source,'role':role,'job':tag,'actual_corpus_exit':int(failed),'compact_owner':str(compact),'raw_native':summary['raw_native_directory'],'native_executable':summary['native_executable']}),flush=True)
 return int(failed)

if __name__=='__main__':
 try:sys.exit(main())
 except Exception as error:
  conf=json.loads(pathlib.Path(sys.argv[sys.argv.index('--config-file')+1]).read_text());compact=pathlib.Path.home()/conf['compact_root'];compact.mkdir(parents=True,exist_ok=True)
  import traceback
  failure={'phase':'public-corpus-collector','source':conf['source'],'corpus_role':conf['corpus_role'],'error_type':type(error).__name__,'message':str(error)[:2000],'traceback':traceback.format_exc()[-6000:],'actual_exit':3}
  (compact/'controller.failure.summary.json').write_text(json.dumps(failure)+'\n');print(json.dumps(failure),flush=True);sys.exit(3)
