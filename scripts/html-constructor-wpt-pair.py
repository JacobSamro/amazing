#!/usr/bin/env python3
"""Compare complete native classifications without duplicating paired payloads."""
import hashlib,importlib.util,json,pathlib,sys
spec=importlib.util.spec_from_file_location('corpus',pathlib.Path(__file__).with_name('html-constructor-wpt-corpus.py'))
C=importlib.util.module_from_spec(spec);spec.loader.exec_module(C)
def compare(before,after):
 assert before['role']=='before' and after['role']=='after'
 for key in ['pin','input_sha256','cases','runner_sha256','selectors','jobs','timeout_ms','viewport']:
  assert before[key]==after[key],('actual measurement inputs differ',key)
 assert before['pin']==C.PIN
 old_files,old_subs=C.passing(before['records']);new_files,new_subs=C.passing(after['records'])
 assert len({r['file'] for r in before['records']})==len(before['records'])==len(before['cases'])
 assert len({r['file'] for r in after['records']})==len(after['records'])==len(after['cases'])
 lost_files=sorted(old_files-new_files);lost_subs=[list(x) for x in sorted(old_subs-new_subs)]
 return {'source_before':before['source'],'source_after':after['source'],'files':len(before['cases']),
  'before_pass_files':len(old_files),'after_pass_files':len(new_files),'before_pass_subtests':len(old_subs),
  'after_pass_subtests':len(new_subs),'lost_files':lost_files,'lost_subtests':lost_subs,
  'gained_files':sorted(new_files-old_files),'gained_subtests':[list(x) for x in sorted(new_subs-old_subs)],
  'zero_loss':not lost_files and not lost_subs,'complete_comparable_measurements':True,
  'baseline_all_classifications_pass_required':False}
def main():
 a,b,out=map(pathlib.Path,sys.argv[1:]);v=compare(C.unpack(a),C.unpack(b))
 v['original_manifest']={'path':str(a),'sha256':C.digest(a.read_bytes())}
 v['after_manifest']={'path':str(b),'sha256':C.digest(b.read_bytes())}
 deltas={key:v.pop(key) for key in ['lost_files','lost_subtests','gained_files','gained_subtests']}
 v.update({key+'_count':len(values) for key,values in deltas.items()})
 if any(deltas.values()):
  v['delta_manifest']=C.pack(out.with_name('delta.bundle.manifest.json'),deltas)
 else:v.update(deltas)
 data=(json.dumps(v,ensure_ascii=True,separators=(',',':'))+'\n').encode()
 if len(data)>512*1024:raise ValueError('compact comparison exceeds ticket file limit')
 out.write_bytes(data);print(json.dumps({k:v[k] for k in ['files','before_pass_files','after_pass_files','before_pass_subtests','after_pass_subtests','zero_loss']}))
 return 0 if v['zero_loss'] else 1
if __name__=='__main__':raise SystemExit(main())
