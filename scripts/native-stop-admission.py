#!/usr/bin/env python3
"""Read-only archived-byte admission for root's idle/resize stop, not engine gates."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys

spec=importlib.util.spec_from_file_location('archive',Path(__file__).with_name('native-proof-archive.py'))
a=importlib.util.module_from_spec(spec);spec.loader.exec_module(a)
REGISTRY=a.ROOT/'stop-admission'

def private(path,directory=False):
    info=path.lstat()
    if info.st_uid!=os.getuid() or info.st_mode&0o077 or path.is_symlink() or (not path.is_dir() if directory else not path.is_file()):
        raise ValueError('stop admission must be owner-private: '+str(path))

def sha_json(value):return hashlib.sha256(json.dumps(value,sort_keys=True,separators=(',',':')).encode()).hexdigest()

def file_records(manifest):
    return {str(a.original_path(manifest,row)):(row,Path(manifest['destination'])/'payload'/row['label']/row['path']) for row in manifest['files']}

def captured_file(descriptor,records):
    # Already-captured native files need no extra copy; legacy /tmp requires
    # matching independent archive bytes. Never reopen a remote baseline blob.
    original=Path(descriptor['path'])
    if a.ROOT in original.parents:
        a.canonical(original);a.native_mount(original);a.exclusions(original.parent)
        path=original
    else:
        if str(original) not in records:raise ValueError('uncaptured required proof: '+str(original))
        row,path=records[str(original)]
        if descriptor.get('bytes') is not None and row['bytes']!=descriptor['bytes']:raise ValueError('archived byte count mismatch: '+str(original))
        if descriptor.get('sha256') is not None and row['sha256']!=descriptor['sha256']:raise ValueError('archived hash mismatch: '+str(original))
    if not path.is_file() or path.is_symlink():raise ValueError('captured regular proof missing: '+str(path))
    if descriptor.get('bytes') is not None and path.stat().st_size!=descriptor['bytes']:raise ValueError('captured byte count changed: '+str(path))
    if descriptor.get('sha256') is not None and a.digest(path)!=descriptor['sha256']:raise ValueError('captured hash changed: '+str(path))
    return path

def requirements(final,manifest,home):
    source=final['source_commit'];job=final['job_id'];records=file_records(manifest);observed=[]
    def need(descriptor):
        path=captured_file(descriptor,records);observed.append({'original_path':descriptor['path'],'captured_path':str(path),'bytes':path.stat().st_size,'sha256':a.digest(path)});return path
    if manifest['source']!=source or manifest['job']!=job:raise ValueError('archive belongs to another source/job')
    receipt=manifest['terminal_producer'];marker=home/('amazing-completed-'+job+'.json')
    if a.digest(marker)!=receipt['sha256'] or marker.stat().st_size!=receipt['bytes']:
        raise ValueError('original terminal receipt changed')
    raw=final.get('execution_raw') or final.get('execution_raw_partial')
    if not isinstance(raw,dict) or 'path' not in raw:raise ValueError('missing actual whole execution raw descriptor')
    need(raw)
    # Source packet hash is exact public upload identity. It must be captured,
    # not merely an unverified source-name or a receiver's different tree.
    packet=final.get('source_archive_sha256')
    if not re.fullmatch(r'[0-9a-f]{64}',str(packet)) or not any(row['sha256']==packet for row in manifest['files']):
        raise ValueError('exact original source packet is not captured')
    packet_row=next(row for row in manifest['files'] if row['sha256']==packet)
    need({'path':str(a.original_path(manifest,packet_row)),'bytes':packet_row['bytes'],'sha256':packet})
    measured=0;generated=0;catalog=final.get('artifacts')
    if not isinstance(catalog,dict) or final.get('catalog_error'):raise ValueError('original completed catalog unavailable')
    for name,expected in catalog.items():
        pure=Path(name)
        if pure.is_absolute() or '..' in pure.parts or not pure.is_relative_to(final['output_root']):raise ValueError('catalog escapes original output owner')
        file=home/pure
        if file.is_symlink() or file.stat().st_size!=expected['bytes'] or a.digest(file)!=expected['sha256']:
            raise ValueError('original compact catalog bytes changed: '+name)
        row=json.loads(file.read_text())
        if isinstance(row,dict) and isinstance(row.get('artifact_manifest'),dict):
            retention=need(row['artifact_manifest'])
            retained=json.loads(retention.read_text())
            if retained.get('source')!=source or retained.get('job')!=row['job'] or retained.get('profile')!=row['profile']:
                raise ValueError('retention provenance source/job/profile mismatch')
            for artifact in retained['artifacts']:
                need({'path':artifact['blob'],'bytes':artifact['bytes'],'sha256':artifact['sha256']})
                if artifact['kind']=='elf':measured+=1
                elif artifact['kind']=='generated-source':generated+=1
                else:raise ValueError('unknown retained provenance kind')
        # Public tooling/embedding native summaries can name a separately
        # measured executable. This is independent of Cargo's all-ELF list.
        if isinstance(row,dict):
            for key in ('measured_tool_elf','native_elf'):
                if isinstance(row.get(key),dict):need(row[key]);measured+=1
    return {'whole_raw_captured':True,'source_packet_captured':True,'measured_elf_records':measured,'generated_source_records':generated,'catalog_sha256':sha_json(catalog),'verified_required_files':sorted(observed,key=lambda r:(r['original_path'],r['captured_path']))}

def register(path,expected_sha,archive_exit,box_id,home=None):
    home=Path(home or Path.home());a.root_admission()
    if not re.fullmatch(r'bx_[A-Za-z0-9]+',box_id):raise ValueError('invalid actual box identity')
    a.verify(path,expected_sha,archive_exit);manifest=json.loads(Path(path).read_text())
    marker=home/('amazing-completed-'+manifest['job']+'.json');final=json.loads(marker.read_text())
    if final.get('completed') is not True or type(final.get('actual_exit')) is not int or final.get('box_id')!=box_id:
        raise ValueError('actual terminal completed source/job/box required')
    closure=requirements(final,manifest,home)
    REGISTRY.mkdir(mode=0o700,exist_ok=True);private(REGISTRY,True)
    record={'schema':1,'box_id':box_id,'source':manifest['source'],'job':manifest['job'],'archive':{'path':str(path),'sha256':expected_sha,'actual_exit':archive_exit},'terminal_sha256':a.digest(marker),'closure':closure,'failed_producer_preserved':final['actual_exit']!=0}
    target=REGISTRY/(manifest['job']+'.manifest.json');a.save(target,record);target.chmod(0o600)
    with target.open('rb') as stream:os.fsync(stream.fileno())
    a.fsync_directory(REGISTRY)
    return record

def admission(box_id,home=None):
    home=Path(home or Path.home());jobs=[];errors=[]
    try:
        a.root_admission();private(REGISTRY,True)
        markers={p.name[len('amazing-completed-'):-len('.json')]:p for p in home.glob('amazing-completed-*.json')}
        ledger=home/'.amazing-jobs'
        if ledger.is_symlink():raise ValueError('job ledger is a symlink')
        pending=set(p.name for p in ledger.iterdir())-set(markers) if ledger.exists() else set()
        if pending:raise ValueError('live/partial jobs without terminal receipt: '+','.join(sorted(pending)))
        for job,path in sorted(markers.items()):
            final=json.loads(path.read_text())
            if final.get('box_id')!=box_id:raise ValueError('terminal job has another/unknown box owner: '+job)
            if final.get('completed') is not True or type(final.get('actual_exit')) is not int or final.get('job_id')!=job:
                raise ValueError('terminal identity incomplete: '+job)
            record_path=REGISTRY/(job+'.manifest.json');private(record_path);record=json.loads(record_path.read_text())
            if record.get('schema')!=1 or record.get('box_id')!=box_id or record.get('job')!=job or record.get('source')!=final.get('source_commit') or record.get('terminal_sha256')!=a.digest(path):
                raise ValueError('archive registration identity changed: '+job)
            archive=record['archive'];a.verify(archive['path'],archive['sha256'],archive['actual_exit'])
            closure=requirements(final,json.loads(Path(archive['path']).read_text()),home)
            if closure!=record['closure']:raise ValueError('proof closure changed: '+job)
            jobs.append({'job':job,'source':record['source'],'producer_exit':final['actual_exit'],'archive_sha256':archive['sha256'],'closure':closure})
        if set(p.name for p in home.glob('amazing-completed-*.json'))!={p.name for p in markers.values()}:raise ValueError('producer set changed during verification')
        current=set(p.name for p in ledger.iterdir()) if ledger.exists() else set()
        if current-set(markers):raise ValueError('new live/partial producer appeared during verification')
        for job,path in markers.items():
            registration=json.loads((REGISTRY/(job+'.manifest.json')).read_text())
            if registration['terminal_sha256']!=a.digest(path):raise ValueError('terminal changed during verification: '+job)
    except (OSError,ValueError,KeyError,TypeError) as error:errors.append(type(error).__name__+': '+str(error))
    return {'schema':1,'box_id':box_id,'admitted':not errors,'jobs':jobs,'errors':errors,'scope':'Recorded supported producers only; no universal unregistered/manual proof discovery, no engine merge barrier, no lifecycle action'}

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('mode',choices=('register','check'));p.add_argument('--box-id',required=True);p.add_argument('--manifest');p.add_argument('--sha256');p.add_argument('--actual-archive-exit',type=int);p.add_argument('--seconds',type=int,default=600);args=p.parse_args()
    try:
        with a.deadline(args.seconds):
            row=register(args.manifest,args.sha256,args.actual_archive_exit,args.box_id) if args.mode=='register' else admission(args.box_id)
        code=0 if args.mode=='register' or row['admitted'] else 3
    except Exception as error:row={'schema':1,'admitted':False,'error':str(error)};code=124 if isinstance(error,TimeoutError) else 3
    row['actual_exit']=code;print(json.dumps(row,separators=(',',':')),flush=True);return code
if __name__=='__main__':raise SystemExit(main())
