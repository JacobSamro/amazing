#!/usr/bin/env python3
"""Native tooling controls; metadata/IO/pixels are fixtures, never engine proof."""
import argparse,contextlib,copy,errno,hashlib,importlib.util,io,json,os,pathlib,shutil,subprocess,sys,tempfile
ROOT=pathlib.Path(__file__).resolve().parent
def module(name,file):
 spec=importlib.util.spec_from_file_location(name,ROOT/file);result=importlib.util.module_from_spec(spec);spec.loader.exec_module(result);return result
contract=module('image_contract','image-corpus-contract.py');cmp=module('image_compare','wpt_render_compare.py');publisher=module('image_publish','image-presentation-publish.py');gate=module('image_gate','image-presentation-gate.py')
def main():
 p=argparse.ArgumentParser();p.add_argument('--source',required=True);p.add_argument('--authentic-408',type=pathlib.Path,required=True);p.add_argument('--authentic-8d',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
 raw=pathlib.Path(tempfile.mkdtemp(prefix='image-corpus-contract-'));controls=[];logs=[]
 def check(name,condition,detail=None):controls.append({'name':name,'passed':bool(condition),'detail':detail})
 def rejects(name,fn):
  try:fn()
  except (ValueError,cmp.CorpusReadFailure,OSError):check(name,True);return
  check(name,False,'invalid fixture was accepted')
 # Preserve the original public collector controls, including every timeout/ID/loss control.
 existing=raw/'original-collector-controls.compact.json';log=raw/'original-collector-controls.log'
 with log.open('wb') as stream:run=subprocess.run([sys.executable,str(ROOT/'render-corpus-collector-tests.py'),'--source',a.source,'--output',str(existing)],stdout=stream,stderr=subprocess.STDOUT)
 logs.append({'path':str(log),'bytes':log.stat().st_size,'sha256':hashlib.sha256(log.read_bytes()).hexdigest()});old=json.loads(existing.read_text());controls.extend(old['controls']);check('all-original-public-collector-controls-retained',run.returncode==0 and old['counts']['failed']==0,old['counts'])
 # Bounded native fixture-read API: errors are injected into the reader, no engine exits simulated.
 fixture=raw/'fixture.html';fixture.write_bytes(b'<p>faithful\xff</p>');check('normal-fixture-read-preserves-replacement-decoding',cmp.read_wpt_text(fixture)=='<p>faithful\ufffd</p>')
 for error_number in (103,107):
  calls=[];events=io.StringIO()
  def reader(path):
   calls.append(path)
   if len(calls)==1:raise OSError(error_number,'injected transient filesystem read',str(path))
   return path.read_text(errors='replace')
  with contextlib.redirect_stderr(events):text=cmp.read_wpt_text(fixture,_reader=reader)
  check('one-immediate-same-path-retry:'+str(error_number),text=='<p>faithful\ufffd</p>' and calls==[fixture,fixture])
  check('transient-error-and-success-attempt-provenance:'+str(error_number),'"attempt": 1' in events.getvalue() and '"attempt": 2' in events.getvalue() and str(error_number) in events.getvalue())
  calls=[]
  def permanent(path):calls.append(path);raise OSError(error_number,'injected persistent failure',str(path))
  with contextlib.redirect_stderr(io.StringIO()):rejects('persistent-IO-aborts-tooling:'+str(error_number),lambda:cmp.read_wpt_text(fixture,_reader=permanent))
  check('persistent-IO-exactly-two-attempts:'+str(error_number),len(calls)==2)
 calls=[]
 def denied(path):calls.append(path);raise OSError(errno.EACCES,'injected permission denial',str(path))
 with contextlib.redirect_stderr(io.StringIO()):rejects('nontransient-IO-aborts-without-retry',lambda:cmp.read_wpt_text(fixture,_reader=denied))
 check('permission-failure-only-one-attempt',calls==[fixture])
 # A historical helper only transitions to the exact reviewed current bytes.
 for name,legacy in [('image-resource-corpus.py',contract.LEGACY_RESOURCE),('wpt_render_compare.py',next(iter(contract.LEGACY_COMPARATORS))),('image-presentation-gate.py',next(iter(contract.LEGACY_EXECUTORS)))]:
  check('arbitrary-changed-current-helper-rejected:'+name,not contract.compatible_helper(name,legacy,'0'*64))
  current_sha=hashlib.sha256((ROOT/name).read_bytes()).hexdigest()
  check('reviewed-exact-helper-transition-accepted:'+name,contract.compatible_helper(name,legacy,current_sha))
  check('identical-helper-equality-path-retained:'+name,contract.compatible_helper(name,current_sha,current_sha))
 # Original authentic408 baseline is never relabeled or promoted from its outer1.
 auth=json.loads((a.authentic_408/'gate.summary.json').read_text())
 for key,value in auth['profile'].items():
  if value is None:os.environ.pop(key,None)
  else:os.environ[key]=value
 gate.validate_baseline(a.authentic_408,auth['source_commit'],'after');check('authentic408-whole-engine-phase-success-accepted',auth['job_exit']==0 and len(auth['phases'])==26)
 witness=json.loads((a.authentic_408/'baseline-terminal.summary.json').read_text());check('authentic408-original-outer1-preserved',witness['actual_exit']==1 and auth['corpus_role']=='after')
 failed=json.loads((a.authentic_8d/'gate.summary.json').read_text());rejects('genuine8d-engine-failure-still-rejected',lambda:gate.validate_baseline(a.authentic_8d,failed['source_commit'],'after'))
 altered=raw/'altered-408';shutil.copytree(a.authentic_408,altered);d=copy.deepcopy(auth);d['phases'][0]['actual_returncode']=1;(altered/'gate.summary.json').write_text(json.dumps(d));rejects('altered408-receipt-rejected',lambda:gate.validate_baseline(altered,auth['source_commit'],'after'))
 b=json.loads((a.authentic_408/'image-resources.compact.json').read_text());current=copy.deepcopy(b);current['source_commit']=a.source;current['adapter_sha256']=hashlib.sha256((ROOT/'image-resource-corpus.py').read_bytes()).hexdigest();current['comparator_sha256']=hashlib.sha256((ROOT/'wpt_render_compare.py').read_bytes()).hexdigest()
 result=contract.compare_resource(b,current,'after',b['source_commit']);check('full-authentic491-and226-AFTER-comparison-zero-loss',len(b['tests'])==491 and len(b['rc21_entries'])==226 and result=={'lost':[],'gained':[]})
 before=copy.deepcopy(b);before['corpus_role']='before';check('original-BEFORE-contract-remains-valid',contract.compare_resource(before,current,'before',b['source_commit'])==result)
 rejects('AFTER-cannot-be-relabelled-BEFORE',lambda:contract.compare_resource(b,current,'before',b['source_commit']))
 rejects('resource-baseline-source-mismatch-rejected',lambda:contract.compare_resource(b,current,'after','0'*40))
 for field,value in [('revision','unknown'),('profile',{}),('rc21_entries',{}),('settings',{}),('adapter_sha256','0'*64),('comparator_sha256','0'*64)]:
  bad=copy.deepcopy(b);bad[field]=value;rejects('resource-metadata-mismatch:'+field,lambda bad=bad:contract.compare_resource(bad,current,'after',b['source_commit']))
 bad=copy.deepcopy(current);bad['tests'].pop(next(iter(bad['tests'])));rejects('missing-original-resource-ID-rejected',lambda:contract.compare_resource(b,bad,'after',b['source_commit']))
 bad=copy.deepcopy(current);lost=next(k for k,v in b['tests'].items() if v=='pass');bad['tests'][lost]='mismatch';check('actual-loss-comparison-keeps-exact-ID',contract.compare_resource(b,bad,'after',b['source_commit'])['lost']==[lost],lost)
 # Exercise the public ferry with the exact original terminal bytes, without running an engine.
 template=raw/'ferry-template.sh';template.write_text('true\n');command=raw/'ferry-command.sh';manifest=raw/'ferry.manifest.json';native_baseline=raw/'ferry-native-baseline';log=raw/'ferry-control.log'
 argv=[sys.executable,str(ROOT/'image-presentation-ferry.py'),'--baseline-dir',str(a.authentic_408),'--terminal-summary',str(a.authentic_408/'baseline-terminal.summary.json'),'--baseline-source',auth['source_commit'],'--baseline-corpus-role','after','--after-command-template',str(template),'--template-sha256',hashlib.sha256(template.read_bytes()).hexdigest(),'--native-baseline-dir',str(native_baseline),'--output-command',str(command),'--output-manifest',str(manifest)]
 with log.open('wb') as stream:run=subprocess.run(argv,stdout=stream,stderr=subprocess.STDOUT)
 check('public-ferry-authentic-AFTER-CLI-exit-zero',run.returncode==0,run.returncode)
 if run.returncode==0:
  with log.open('ab') as stream:replayed=subprocess.run(['bash',str(command)],stdout=stream,stderr=subprocess.STDOUT)
  check('public-ferry-replay-original-byte-preservation',replayed.returncode==0 and (native_baseline/'baseline-terminal.summary.json').read_bytes()==(a.authentic_408/'baseline-terminal.summary.json').read_bytes())
  gate.validate_baseline(native_baseline,auth['source_commit'],'after');check('ferried-authentic-baseline-passes-same-validation',True)
 else:check('public-ferry-replay-original-byte-preservation',False)
 logs.append({'path':str(log),'bytes':log.stat().st_size,'sha256':hashlib.sha256(log.read_bytes()).hexdigest()})
 # Publication subprocess controls use explicit synthetic summaries. Actual helper exits are observed.
 synthetic_source='1'*40
 def make(label,exit_code):
  native=raw/label;native.mkdir();body=b'{"tests":{"fixture":"pass"}}\n';(native/'fixture.compact.json').write_bytes(body)
  summary={'source_commit':synthetic_source,'job_exit':exit_code,'phases':[{'phase':'fixture','source_commit':synthetic_source,'actual_returncode':exit_code}], 'compact_files':{'fixture.compact.json':{'bytes':len(body),'sha256':hashlib.sha256(body).hexdigest()}}};(native/'gate.summary.json').write_text(json.dumps(summary));return native
 for observed in (0,1):
  native=make('publication-'+str(observed),observed);destination=raw/('published-'+str(observed));log=raw/('publication-'+str(observed)+'.log')
  argv=[sys.executable,str(ROOT/'image-presentation-publish.py'),'--source',synthetic_source,'--native-root',str(native),'--published-root',str(destination),'--observed-gate-exit',str(observed)]
  with log.open('wb') as stream:run=subprocess.run(argv,stdout=stream,stderr=subprocess.STDOUT)
  logs.append({'path':str(log),'bytes':log.stat().st_size,'sha256':hashlib.sha256(log.read_bytes()).hexdigest()})
  check('public-publication-actual-subprocess-exit:'+str(observed),run.returncode==observed,run.returncode)
  receipt=json.loads((destination/'compact-publication.manifest.json').read_text());check('publication-preserves-original-observed-exit:'+str(observed),receipt['observed_gate_exit']==observed)
  check('publication-copies-only-byteexact-existing-files:'+str(observed),(destination/'fixture.compact.json').read_bytes()==(native/'fixture.compact.json').read_bytes() and (destination/'gate.summary.json').read_bytes()==(native/'gate.summary.json').read_bytes())
 native=make('publication-bad',1);rejects('publication-source-mismatch-rejected',lambda:publisher.publish('2'*40,native,raw/'bad-source',1));rejects('publication-cannot-promote-failed-gate-to-zero',lambda:publisher.publish(synthetic_source,native,raw/'bad-exit',0))
 (native/'fixture.compact.json').write_bytes(b'tampered');rejects('publication-original-hash-change-rejected',lambda:publisher.publish(synthetic_source,native,raw/'bad-hash',1))
 native=make('publication-oversize',1);(native/'fixture.compact.json').write_bytes(b'x'*(512*1024+1));rejects('oversized-publication-refused-before-destination',lambda:publisher.publish(synthetic_source,native,raw/'bad-oversize',1));check('oversize-refusal-created-no-destination',not (raw/'bad-oversize').exists())
 native=make('publication-symlink',1);(native/'fixture.compact.json').unlink();(native/'fixture.compact.json').symlink_to(fixture);rejects('compact-symlink-refused',lambda:publisher.publish(synthetic_source,native,raw/'bad-symlink',1))
 failed=sum(not row['passed'] for row in controls);result={'schema':1,'source':a.source,'qualification':'Native tooling controls only: original synthetic pixel controls plus authentic baseline metadata, injected filesystem error fixtures and synthetic publication fixtures; NOT an Amazing engine/corpus gate','counts':{'passed':len(controls)-failed,'failed':failed},'controls':controls,'original_control_count':len(old['controls']),'raw_native_directory':str(raw),'raw_logs':logs,'helper_hashes':{name:hashlib.sha256((ROOT/name).read_bytes()).hexdigest() for name in ['image-corpus-contract.py','image-corpus-contract-tests.py','image-presentation-gate.py','image-presentation-ferry.py','image-presentation-publish.py','image-resource-corpus.py','wpt_render_compare.py']}}
 a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(result,separators=(',',':'))+'\n');print(json.dumps({'counts':result['counts'],'original_controls':len(old['controls']),'actual_control_process_exit':int(bool(failed))}),flush=True);return int(bool(failed))
if __name__=='__main__':sys.exit(main())
