#!/usr/bin/env python3
"""Export authenticated classified pass identities; native raw reports stay put."""
import base64,collections,gzip,hashlib,json,pathlib,sys,zlib,importlib.util
spec=importlib.util.spec_from_file_location('corpus',pathlib.Path(__file__).with_name('html-constructor-wpt-corpus.py'));C=importlib.util.module_from_spec(spec);spec.loader.exec_module(C)
def read(path,binding):
 p=pathlib.Path(path)
 if not p.is_absolute() or not p.is_relative_to('/tmp') or p.resolve()!=p:raise ValueError('original canonical native raw required')
 if type(binding['bytes']) is not int or binding['bytes']>256*1024*1024:raise ValueError('native raw bound exceeded')
 data=p.read_bytes()
 if len(data)!=binding['bytes'] or C.digest(data)!=binding['sha256']:raise ValueError('original native raw hash differs')
 return data

def project(meta,rows):
 expected=json.loads(zlib.decompress(base64.b64decode(meta['records_zlib_base64'])))
 if set(rows)!=set(expected):raise ValueError('original whole selection differs')
 result={}
 for suite,items in rows.items():
  if len(items)!=len(expected[suite]):raise ValueError('original case inventory differs')
  result[suite]=[]
  for actual,original in zip(items,expected[suite]):
   if actual['test']!=original['test'] or actual['status']!=original['status']:raise ValueError('original case classification differs')
   names=collections.Counter(s['name'] for s in actual.get('subtests',[]) if s['status'] in (0,'pass'))
   hashes=collections.Counter()
   for name,count in names.items():
    if not isinstance(name,str):raise ValueError('original named pass identity unavailable')
    hashes[hashlib.sha256(name.encode('utf-16le','surrogatepass')).hexdigest()]+=count
   signature=C.digest(json.dumps(hashes,sort_keys=True,separators=(',',':')).encode())
   if signature!=original['passing_counter_sha256'] or sum(hashes.values())!=original['passing_count']:
    raise ValueError('classified pass identities do not match original Counter')
   # Natural names compress well; exact UTF16 hash/multiplicity equality above
   # proves this is the original classified pass set, not a new measurement.
   result[suite].append({'test':actual['test'],'status':actual['status'],'passing':dict(names)})
 return result

def export(meta_path,expected_sha,source,output):
 p=pathlib.Path(meta_path);data=p.read_bytes()
 if len(data)>512*1024 or C.digest(data)!=expected_sha:raise ValueError('original compact metadata hash differs')
 meta=json.loads(data)
 if meta['source_commit']!=source:raise ValueError('original source differs')
 rows={}
 for binding in meta['raw']:
  raw_path=pathlib.Path(binding['path']);raw=read(raw_path,binding)
  if raw_path.suffix=='.jsonl':suite=raw_path.stem;items=[json.loads(line) for line in raw.splitlines() if line.strip()]
  elif raw_path.parent.name=='dom-b-audit-core':suite='core';items=json.loads(raw)['results']
  elif raw_path.parent.name=='ia-modules-modules':suite='modules';items=json.loads(raw)['results']
  else:raise ValueError('unknown original typed corpus producer')
  if suite in rows:raise ValueError('duplicate original suite')
  rows[suite]=items
 value={'source':source,'original_compact_sha256':expected_sha,'original_binary_sha256':meta['binary_sha256'],
  'records':project(meta,rows),'raw':meta['raw'],'classified_pass_metadata_only':True,'engine_rerun':False}
 return C.pack(pathlib.Path(output),value)
if __name__=='__main__':
 print(json.dumps(export(*sys.argv[1:]),sort_keys=True))
