#!/usr/bin/env python3
"""Native tooling controls only; the pixel producer is synthetic, never Amazing."""
import argparse,hashlib,importlib.util,json,os,pathlib,subprocess,sys,tempfile
root=pathlib.Path(__file__).resolve().parent
s=importlib.util.spec_from_file_location('collector',root/'render-corpus-collector.py');collector=importlib.util.module_from_spec(s);s.loader.exec_module(collector)

def main():
 p=argparse.ArgumentParser();p.add_argument('--source',required=True);p.add_argument('--output',required=True);a=p.parse_args()
 raw=pathlib.Path(tempfile.mkdtemp(prefix='amazing-collector-controls-'));wpt=raw/'wpt';cohort=wpt/'css/collector-controls';nested=cohort/'nested';nested.mkdir(parents=True)
 (cohort/'reference.html').write_text('<!doctype html><p>reference</p>')
 names=['a-case.html','b+case.html','dot.case.html','under_score.html','space case.html','x@y.html','z=case.html','last-case.html']
 expected=['a-case.html','b__case.html','dot.case.html','under_score.html','space__case.html','x__y.html','z__case.html','last-case.html']
 for n in names:(nested/n).write_text('<!doctype html><link rel="match" href="../reference.html"><p>fixture</p>')
 link=raw/'linked-wpt';link.symlink_to(wpt,target_is_directory=True)
 fake=raw/'synthetic-pixel-producer.py';fake.write_text('''#!/usr/bin/env python3
import os,pathlib,sys
src,dest,width,height=pathlib.Path(sys.argv[2]),pathlib.Path(sys.argv[3]),int(sys.argv[4]),int(sys.argv[5])
pixel=b"\\x00\\x00\\x00" if os.environ.get('TOOLING_NEGATIVE')=='1' and src.name=='a-case.html' else b"\\xff\\xff\\xff"
dest.write_bytes(f"P6\\n{width} {height}\\n255\\n".encode()+pixel*(width*height))
''');fake.chmod(0o755)
 controls=[]
 def check(name,ok,detail=None):
  controls.append({'name':name,'passed':bool(ok),'detail':detail})
 rows=[{'cohort':'css/collector-controls','selection_ids_sha256':'0'*64}]
 metadata,envelope=collector.discover_metadata(link,rows);m=metadata['css/collector-controls']
 check('reference-root-canonicalized-through-real-symlink',m['selected']==8,m['inventory'])
 check('incorrect-historical-census-is-nonblocking',m['historical_census_note']['nonblocking'] and not m['historical_census_note']['historical_projection_matches'])
 absent,_=collector.discover_metadata(link,[{'cohort':'css/collector-controls'}])
 check('absent-historical-census-does-not-remove-tests',absent['css/collector-controls']['selected']==8)
 projected=sorted('css__collector-controls__nested__'+n for n in expected)
 expected_hash=hashlib.sha256(json.dumps(projected,separators=(',',':')).encode()).hexdigest()
 check('slash-and-punctuation-projection-matches-public-id-contract',m['selection_ids_sha256']==expected_hash,{'actual':m['selection_ids_sha256'],'expected':expected_hash})
 good,_=collector.discover_metadata(link,[{'cohort':'css/collector-controls','selection_ids_sha256':expected_hash}])
 check('matching-historical-census-recorded-as-note',good['css/collector-controls']['historical_census_note']['historical_projection_matches'])
 check('raw-envelope-counts-full-selected-references',envelope==8*(3*(800*600*3+64)+2*65536+32768),envelope)
 direct,_=collector.discover_metadata(wpt,rows)
 check('symlink-and-canonical-source-input-hashes-match',direct==metadata)
 argv=[sys.executable,str(root/'wpt_render_compare.py'),'--wpt-root',str(link),'--subdir','css/collector-controls','--exe',str(fake),'--width','8','--height','8','--timeout','20.0','--jobs','2','--limit','0','--compact']
 before=raw/'before';after=raw/'after'
 def run(label,args,environment):
  log=raw/(label+'.log')
  with log.open('wb') as f:r=subprocess.run(args,env=environment,stdout=f,stderr=subprocess.STDOUT)
  return r.returncode,{'path':str(log),'bytes':log.stat().st_size,'sha256':hashlib.file_digest(log.open('rb'),'sha256').hexdigest()}
 before_rc,b_log=run('before',argv+['--out',str(before)],dict(os.environ,TOOLING_NEGATIVE='0'))
 b=json.loads((before/'compact.json').read_text())
 check('actual-public-comparator-before-exit-zero',before_rc==0,before_rc)
 check('actual-public-comparator-before-entire-eight-pass',b['counts']['pass']==8 and len(b['tests'])==8,b['counts'])
 for name in projected:check('actual-public-report-id:'+name,name in b['tests'])
 positive_rc,positive_log=run('positive-zero-loss',argv+['--out',str(raw/'positive'),'--baseline',str(before/'compact.json')],dict(os.environ,TOOLING_NEGATIVE='0'))
 positive=json.loads((raw/'positive/compact.json').read_text())
 check('decimal-timeout-actual-public-zero-loss-exit-zero',positive_rc==0 and positive['baseline_compare']['lost']==[] and positive['counts']['pass']==8,positive_rc)
 contract_logs=[positive_log]
 timeout_index=argv.index('--timeout')+1
 for value in ['20','1.25']:
  decimal_argv=argv.copy();decimal_argv[timeout_index]=value
  decimal_out=raw/('timeout-'+value)
  decimal_rc,decimal_log=run('timeout-'+value,decimal_argv+['--out',str(decimal_out),'--baseline',str(before/'compact.json')],dict(os.environ,TOOLING_NEGATIVE='0'));contract_logs.append(decimal_log)
  decimal_data=json.loads((decimal_out/'compact.json').read_text())
  check('actual-public-timeout-accepted:'+value,decimal_rc==0 and decimal_data['baseline_compare']['lost']==[] and decimal_data['counts']['pass']==8,decimal_rc)
 for value in ['-1','0','0.5','601','nan','inf','-inf','not-a-number']:
  bad_argv=argv.copy();bad_argv[timeout_index]=value
  # Equals syntax prevents argparse treating -inf as an option instead of a value.
  bad_argv[timeout_index-1:timeout_index+1]=['--timeout='+value]
  bad_out=raw/('invalid-timeout-'+value)
  bad_rc,bad_log=run('invalid-timeout-'+value,bad_argv+['--out',str(bad_out)],dict(os.environ,TOOLING_NEGATIVE='0'));contract_logs.append(bad_log)
  check('actual-public-invalid-timeout-rejected:'+value,bad_rc!=0 and not (bad_out/'compact.json').exists(),bad_rc)
 rc,a_log=run('negative-after',argv+['--out',str(after),'--baseline',str(before/'compact.json')],dict(os.environ,TOOLING_NEGATIVE='1'))
 after_data=json.loads((after/'compact.json').read_text());lost=after_data['baseline_compare']['lost'];wanted='css__collector-controls__nested__a-case.html'
 check('negative-actual-lost-pass-public-cli-exit-two',rc==2,rc)
 check('negative-actual-lost-pass-id-retained',lost==[wanted],lost)
 check('negative-preserves-all-other-seven-passes',after_data['counts']['pass']==7 and after_data['counts']['mismatch']==1,after_data['counts'])
 comparison=collector.cmp.compare_to_baseline({'one':'mismatch'},{'one':'pass'})
 check('public-comparison-refuses-pass-to-mismatch',comparison['lost']==['one'])
 comparison=collector.cmp.compare_to_baseline({'one':'timeout'},{'one':'pass'})
 check('public-comparison-refuses-pass-to-timeout',comparison['lost']==['one'])
 failed=sum(not c['passed'] for c in controls)
 result={'schema':1,'source':a.source,'qualification':'Tooling controls ONLY; synthetic pixel producer, no Amazing engine build/tests or pinned WPT gate','controls':controls,'counts':{'passed':len(controls)-failed,'failed':failed},'actual_control_process_exits':{'before':before_rc,'negative_after':rc,'positive_zero_loss':positive_rc},'negative_actual_lost':lost,'raw_native_directory':str(raw),'raw_logs':[b_log,a_log]+contract_logs,'public_sources':{n:collector.gc.describe(root/n) for n in ['render-corpus-collector.py','render-corpus-collector-tests.py','wpt_render_compare.py']}}
 out=pathlib.Path(a.output);out.parent.mkdir(parents=True,exist_ok=True);out.write_text(json.dumps(result,separators=(',',':'))+'\n');print(json.dumps({'summary':str(out),'counts':result['counts'],'actual_exit':int(bool(failed))}),flush=True);return int(bool(failed))

if __name__=='__main__':sys.exit(main())
