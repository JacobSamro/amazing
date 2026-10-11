#!/usr/bin/env python3
"""Own gate adapter: classify only actual shipping WPT-harness outputs."""
import argparse,base64,concurrent.futures,hashlib,json,pathlib,subprocess,zlib,collections,sys
PIN='c2a2fce69e5c2509d9dc28440ec8b42532e5c293'
def digest(data):return hashlib.sha256(data).hexdigest()
def pack(path,value):
 data=json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=True).encode();compressed=base64.b64encode(zlib.compress(data,9)).decode();parts=[]
 for index,start in enumerate(range(0,len(compressed),240000)):
  name=path.name+f'.part-{index}.compact.json';body=json.dumps({'schema':1,'part':index,'data':compressed[start:start+240000]},separators=(',',':')).encode()+b'\n';(path.parent/name).write_bytes(body);parts.append({'name':name,'bytes':len(body),'sha256':digest(body)})
 manifest={'schema':1,'encoding':'base64-zlib-json','parts':parts,'uncompressed_bytes':len(data),'uncompressed_sha256':digest(data)};path.write_text(json.dumps(manifest,separators=(',',':'))+'\n');return manifest

def unpack(path):
 m=json.loads(path.read_text());parts=[]
 for row in m['parts']:
  raw=(path.parent/row['name']).read_bytes();assert len(raw)==row['bytes'] and digest(raw)==row['sha256'];parts.append(json.loads(raw)['data'])
 raw=zlib.decompress(base64.b64decode(''.join(parts),validate=True));assert len(raw)==m['uncompressed_bytes'] and digest(raw)==m['uncompressed_sha256'];return json.loads(raw)
def integer(value):return type(value) is int

def classify(name, report, actual_exit, timed_out=False):
 """Retain native facts; malformed fields never acquire passing identity."""
 errors=[];subs=[];native_counts=collections.Counter();occurrences=collections.Counter()
 if not isinstance(report,dict):
  return {'file':name,'status':'timeout' if timed_out else 'error','native_status':None,
   'subtests':[],'actual_exit':actual_exit,'harness_status':None,'native_wpt_revision':None,
   'protocol_errors':['worker_deadline' if timed_out else 'native_json_unavailable']}
 native_status=report.get('status');revision=report.get('wpt_revision');harness=report.get('harness_status')
 if report.get('test')!=name:errors.append('native_case_identity_mismatch')
 if revision not in [None,PIN]:errors.append('native_pin_mismatch')
 if native_status not in ['pass','fail','timeout','error','unsupported']:errors.append('native_file_status_invalid')
 if actual_exit!=(0 if native_status=='pass' else 1):errors.append('native_exit_status_mismatch')
 rows=report.get('subtests')
 if not isinstance(rows,list):errors.append('native_subtests_not_array');rows=[]
 for index,sub in enumerate(rows):
  if not isinstance(sub,dict):
   subs.append({'name':None,'occurrence':index,'native_status':None,'status':'error','protocol_errors':['native_subtest_not_object']});errors.append('native_subtest_not_object');continue
  original_name=sub.get('name');code=sub.get('status');problems=[]
  # Preserve exact JSON values, including null/invalid names and status codes.
  key=json.dumps(original_name,sort_keys=True,separators=(',',':'),ensure_ascii=True)
  occurrence=occurrences[key];occurrences[key]+=1
  if not isinstance(original_name,str):problems.append('native_name_not_string')
  valid_code=integer(code) and code in [0,1,2,3,4]
  if not valid_code:problems.append('native_subtest_status_invalid')
  native_category=['pass','fail','timeout','notrun'][code] if integer(code) and code in [0,1,2,3] else 'error'
  native_counts[native_category]+=1
  category=['pass','fail','timeout','notrun','precondition_failed'][code] if valid_code else 'error'
  if problems:category='error';errors.extend(problems)
  subs.append({'name':original_name,'occurrence':occurrence,'native_status':code,'status':category,
   'protocol_errors':problems})
 counts=report.get('counts')
 if not isinstance(counts,dict):errors.append('native_counts_not_object')
 else:
  for key in ['pass','fail','timeout','notrun','error']:
   if not integer(counts.get(key)) or counts[key]!=native_counts[key]:errors.append('native_count_mismatch:'+key)
 if native_status=='pass' and (revision!=PIN or not integer(harness) or harness!=0 or any(x['status']!='pass' for x in subs)):
  errors.append('native_pass_claim_inconsistent')
 if timed_out:errors.append('worker_deadline')
 # Subtest passes require authentic case/pin and an actual named PASS record.
 # Other malformed siblings remain errors, preserving partial passing identities.
 if report.get('test')!=name or revision!=PIN:
  for sub in subs:
   if sub['status']=='pass':sub['status']='error';sub['protocol_errors'].append('unverified_pass_identity')
 return {'file':name,'status':'timeout' if timed_out else ('error' if errors else native_status),
  'native_status':native_status,'subtests':subs,'actual_exit':actual_exit,'harness_status':harness,
  'native_wpt_revision':revision,'native_counts':counts,'protocol_errors':errors}

def passing(records):
 files={r['file'] for r in records if r['status']=='pass' and r['subtests']}
 subs={(r['file'],s['name'],s['occurrence']) for r in records for s in r['subtests'] if s['status']=='pass'}
 return files,subs

def capture(index,name,binary,root,raw,source,binary_sha):
 command=[str(binary),'wpt-harness',str(root),name,'800','600','20000']
 process=subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE);timed_out=False
 try:stdout,stderr=process.communicate(timeout=75)
 except subprocess.TimeoutExpired:
  timed_out=True;process.kill();stdout,stderr=process.communicate()
 out=raw/f'{index:05d}.stdout.json';err=raw/f'{index:05d}.stderr.log'
 out.write_bytes(stdout);err.write_bytes(stderr)
 try:report=json.loads(stdout)
 except (UnicodeError,ValueError):report=None
 # The complete native report is persisted BEFORE any schema interpretation.
 structured=raw/f'{index:05d}.record.json'
 structured.write_text(json.dumps({'case':name,'source':source,'binary_sha256':binary_sha,
  'argv':command,'actual_exit':process.returncode,'worker_timed_out':timed_out,'report':report,
  'stdout_sha256':digest(stdout),'stderr_sha256':digest(stderr)},ensure_ascii=True,sort_keys=True)+'\n')
 record=classify(name,report,process.returncode,timed_out)
 record['raw']={key:{'path':str(path),'bytes':path.stat().st_size,'sha256':digest(path.read_bytes())}
  for key,path in [('stdout',out),('stderr',err),('record',structured)]}
 return record

def main():
 p=argparse.ArgumentParser();p.add_argument('--root',type=pathlib.Path,required=True);p.add_argument('--binary',type=pathlib.Path,required=True);p.add_argument('--raw',type=pathlib.Path,required=True);p.add_argument('--compact',type=pathlib.Path,required=True);p.add_argument('--source',required=True);p.add_argument('--role',choices=['before','after'],required=True);p.add_argument('--baseline',type=pathlib.Path);p.add_argument('--jobs',type=int,default=4);a=p.parse_args();assert a.jobs==4;a.raw.mkdir(parents=True,exist_ok=True);a.compact.mkdir(parents=True,exist_ok=True)
 revision=subprocess.check_output(['git','-C',str(a.root),'rev-parse','HEAD'],text=True).strip();assert revision==PIN
 selectors=['dom','html/dom','custom-elements'];cases=[]
 for selector in selectors:
  directory=a.root/selector;assert directory.is_dir()
  for file in sorted(directory.rglob('*')):
   if not file.is_file() or any(part in ['resources','support'] for part in file.relative_to(directory).parts[:-1]):continue
   name=file.name
   if name.endswith(('.any.js','.window.js')):cases.append(file.relative_to(a.root).as_posix())
   elif file.suffix.lower() in ['.html','.htm'] and b'testharness.js' in file.read_bytes():cases.append(file.relative_to(a.root).as_posix())
 cases=sorted(set(cases));assert cases and 'dom/lists/DOMTokenList-coverage-for-attributes.html' in cases and 'dom/nodes/Element-classlist.html' in cases
 input_rows=[(name,digest((a.root/name).read_bytes())) for name in cases];input_sha=digest(json.dumps(input_rows,separators=(',',':')).encode());binary_sha=digest(a.binary.read_bytes());runner_sha=digest(pathlib.Path(__file__).read_bytes())
 def run(row):
  index,name=row
  return capture(index,name,a.binary,a.root,a.raw,a.source,binary_sha)
 with concurrent.futures.ThreadPoolExecutor(max_workers=a.jobs) as pool:records=list(pool.map(run,enumerate(cases)))
 assert len(records)==len(cases);assert len({r['file'] for r in records})==len(cases)
 file_pass,sub_pass=passing(records);assert sub_pass
 classifications={'source':a.source,'role':a.role,'pin':PIN,'selectors':selectors,'jobs':4,'viewport':[800,600],'timeout_ms':20000,'cases':input_rows,'input_sha256':input_sha,'binary_sha256':binary_sha,'runner_sha256':runner_sha,'records':records}
 maps=a.compact/'wpt.bundle.manifest.json';manifest=pack(maps,classifications);lost_files=[];lost_subtests=[];gained_files=[];gained_subtests=[]
 if a.baseline:
  before=unpack(a.baseline);assert before['role']=='before' and before['pin']==PIN and before['input_sha256']==input_sha and before['cases']==input_rows and before['runner_sha256']==runner_sha and before['selectors']==selectors and before['jobs']==4 and before['timeout_ms']==20000 and before['viewport']==[800,600]
  old_files,old_subtests=passing(before['records']);lost_files=sorted(old_files-file_pass);lost_subtests=[list(s) for s in sorted(old_subtests-sub_pass)];gained_files=sorted(file_pass-old_files);gained_subtests=[list(s) for s in sorted(sub_pass-old_subtests)]
  paired=a.compact/'wpt.paired.bundle.manifest.json';pack(paired,{'before':before,'after':classifications})
 diagnostics=[];offending=0
 for record in records:
  if not record['protocol_errors']:continue
  offending+=1
  if len(diagnostics)>=24:continue
  # Bounded original structured facts, not a recovered pass-set or guessed name.
  samples=[{'index':i,'name':sub['name'],'native_status':sub['native_status'],'problems':sub['protocol_errors']}
   for i,sub in enumerate(record['subtests']) if sub['protocol_errors']][:4]
  item={'file':record['file'],'native_file_status':record['native_status'],'harness_status':record['harness_status'],
   'actual_exit':record['actual_exit'],'problems':record['protocol_errors'],'subtests':samples,'raw':record['raw']}
  encoded=json.dumps(item,ensure_ascii=True)
  if len(encoded)>6000:item={'file':record['file'],'problems':record['protocol_errors'],'raw':record['raw'],'sample_omitted':'bounded output'}
  diagnostics.append(item)
 (a.compact/'protocol-diagnostics.compact.json').write_text(json.dumps({'source':a.source,'role':a.role,
  'offending_files':offending,'examples':diagnostics,'examples_omitted':offending-len(diagnostics),
  'native_records_persisted_before_validation':True,'passes_inferred':False},ensure_ascii=True,separators=(',',':'))+'\n')
 summary={'schema':1,'source':a.source,'role':a.role,'pin':PIN,'selectors':selectors,'jobs':4,'timeout_ms':20000,'discovered':len(cases),'input_sha256':input_sha,'binary_sha256':binary_sha,'runner_sha256':runner_sha,'file_counts':dict(collections.Counter(r['status'] for r in records)),'qualifying_nonempty_file_pass':len(file_pass),'native_empty_pass_files':[r['file'] for r in records if r['status']=='pass' and not r['subtests']],'subtest_counts':dict(collections.Counter(s['status'] for r in records for s in r['subtests'])),'lost_files':lost_files,'lost_subtests':lost_subtests,'gained_files':gained_files,'gained_subtests':gained_subtests,'bundle':manifest,'actual_exit':int(bool(lost_files or lost_subtests)),'scope':'actual original pinned native Window WPT harness; no fixtures or source edits; worker-only cases honestly unsupported'};(a.compact/'wpt.summary.json').write_text(json.dumps(summary,separators=(',',':'),ensure_ascii=True)+'\n')
 print(json.dumps({k:v for k,v in summary.items() if k!='bundle'},separators=(',',':')));return summary['actual_exit']
if __name__=='__main__':raise SystemExit(main())
