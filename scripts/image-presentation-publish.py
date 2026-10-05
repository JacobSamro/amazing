#!/usr/bin/env python3
"""Publish original compact evidence for successful AND failed whole native gates.

This copies already-produced summaries and hash-listed compact files only. The
return code remains the observed gate return code. No raw output, receipt or
classification is reconstructed; publication never promotes a failed gate.
"""
import argparse,hashlib,json,os,pathlib,re,sys

def publish(source,native,published,observed):
 if not re.fullmatch('[0-9a-f]{40}',source) or observed not in (0,1):raise ValueError('exact source and actual gate0/1 required')
 data=(native/'gate.summary.json').read_bytes();summary=json.loads(data)
 if summary['source_commit']!=source or summary['job_exit']!=observed:raise ValueError('summary does not bind observed source/exit')
 phases=summary['phases']
 if not phases or any(p['source_commit']!=source for p in phases):raise ValueError('phase source binding differs')
 if observed==0 and any(p['actual_returncode']!=0 for p in phases):raise ValueError('successful summary has a failed phase')
 names=['gate.summary.json']+list(summary['compact_files'])
 if len(names)!=len(set(names)):raise ValueError('duplicate compact name')
 files=[]
 for name in names:
  if pathlib.PurePosixPath(name).name!=name or not re.fullmatch(r'(?:[A-Za-z0-9_.-]+\.)?(?:compact|summary|manifest)\.json',name):raise ValueError('compact filename escapes owner')
  path=native/name
  if path.is_symlink():raise ValueError('compact publication cannot follow symlink')
  body=path.read_bytes();sha=hashlib.sha256(body).hexdigest();row=summary['compact_files'].get(name)
  if len(body)>512*1024 or row and (row['bytes']!=len(body) or row['sha256']!=sha):raise ValueError('compact bytes/hash/limit mismatch')
  files.append((name,body,sha))
 if sum(len(body) for _,body,_ in files)>2*1024*1024:raise ValueError('compact ticket exceeds2MiB')
 manifest=[{'name':name,'bytes':len(body),'sha256':sha,'original_native_path':str(native/name)} for name,body,sha in files]
 receipt={'source_commit':source,'observed_gate_exit':observed,'original_gate_summary_sha256':hashlib.sha256(data).hexdigest(),'files':manifest,'raw_source_elf_untouched':True}
 receipt_body=(json.dumps(receipt,sort_keys=True,separators=(',',':'))+'\n').encode()
 if len(receipt_body)>512*1024 or len(receipt_body)+sum(len(body) for _,body,_ in files)>2*1024*1024:raise ValueError('compact publication including manifest exceeds ticket/file limit')
 published.mkdir(parents=True,exist_ok=False)
 for name,body,sha in files:
  destination=published/name
  with destination.open('xb') as stream:stream.write(body);stream.flush();os.fsync(stream.fileno())
  if hashlib.sha256(destination.read_bytes()).hexdigest()!=sha:raise ValueError('published compact bytes changed')
 destination=published/'compact-publication.manifest.json';body=(json.dumps(receipt,sort_keys=True,separators=(',',':'))+'\n').encode()
 if len(body)>512*1024:raise ValueError('publication manifest exceeds file limit')
 with destination.open('xb') as stream:stream.write(body);stream.flush();os.fsync(stream.fileno())
 return receipt

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--source',required=True);parser.add_argument('--native-root',type=pathlib.Path,required=True);parser.add_argument('--published-root',type=pathlib.Path,required=True);parser.add_argument('--observed-gate-exit',type=int,required=True);args=parser.parse_args()
 receipt=publish(args.source,args.native_root,args.published_root,args.observed_gate_exit)
 print(json.dumps({'source':args.source,'original_gate_exit':args.observed_gate_exit,'published_files':len(receipt['files'])}),flush=True)
 return args.observed_gate_exit
if __name__=='__main__':sys.exit(main())
