#!/usr/bin/env python3
"""Bounded captured-root setup before source upload or supervisor ownership."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import time
import uuid

ROOT = Path('/srv/amazing-proof')
NATIVE_TYPES = {'ext2','ext3','ext4','xfs','btrfs','overlay'}


def exclusions(path):
    for parent in [path,*path.parents]:
        for name in ('.boxignore','.oneignore'):
            item=parent/name
            if item.exists() or item.is_symlink():raise ValueError('unresolved captured-root snapshot exclusion: '+str(item))


def mount(path):
    result=[]
    for line in Path('/proc/self/mountinfo').read_text().splitlines():
        left,right=line.split(' - ',1);fields=left.split()
        directory=Path(re.sub(r'\\([0-7]{3})',lambda m:chr(int(m[1],8)),fields[4]))
        if path==directory or directory in path.parents:result.append((len(directory.parts),{'mountpoint':str(directory),'filesystem':right.split()[0],'device':fields[2],'stat_device':path.stat().st_dev}))
    if not result:raise ValueError('captured-root native mount unavailable')
    actual=max(result,key=lambda r:r[0])[1]
    if actual['filesystem'] not in NATIVE_TYPES:raise ValueError('captured-root must be native, never FUSE/tmpfs: '+actual['filesystem'])
    return actual


def admission():
    if ROOT.resolve(strict=True)!=ROOT or ROOT.is_symlink():raise ValueError('captured-root path is not canonical')
    info=ROOT.stat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid!=os.getuid() or stat.S_IMODE(info.st_mode)!=0o700:raise ValueError('captured-root must be owner-private directory mode0700; existing foreign paths never repaired')
    exclusions(ROOT)
    return {'path':str(ROOT),'uid':info.st_uid,'gid':info.st_gid,'mode':stat.S_IMODE(info.st_mode),'mount':mount(ROOT)}


def ensure(provision=False):
    # Provisioning is NEW-only. Never chown/chmod a foreign existing root.
    if ROOT.exists() or ROOT.is_symlink():return dict(admission(),created=False)
    if not provision:raise FileNotFoundError('captured-root absent; provision before any source upload or supervisor')
    parent=ROOT.parent
    if parent.resolve(strict=True)!=parent or not parent.is_dir():raise ValueError('captured-root parent is not canonical')
    info=parent.stat()
    if info.st_uid!=0 or info.st_mode&0o022:raise ValueError('captured-root parent must be protected root-owned native storage')
    exclusions(ROOT);mount(parent)
    # Root authorized this bounded exact-path setup operation. It neither
    # changes credentials nor performs sandbox lifecycle actions. The new
    # directory remains root-private until its authenticated fd is assigned.
    program="""import os,stat,sys
p='/srv/amazing-proof';parent='/srv'
if os.path.realpath(parent)!=parent or os.stat(parent).st_uid!=0 or os.stat(parent).st_mode&0o022:raise ValueError('unsafe native parent')
try:os.mkdir(p,0o700)
except FileExistsError:
 print('existing');raise SystemExit(0)
fd=os.open(p,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
try:
 s=os.fstat(fd)
 if s.st_uid!=0 or stat.S_IMODE(s.st_mode)!=0o700:raise ValueError('new captured root identity changed')
 os.fchown(fd,int(sys.argv[1]),int(sys.argv[2]));os.fsync(fd)
finally:os.close(fd)
fd=os.open(parent,os.O_RDONLY|os.O_DIRECTORY)
try:os.fsync(fd)
finally:os.close(fd)
print('created')
"""
    result=subprocess.run(['sudo','-n','/usr/bin/python3','-I','-c',program,str(os.getuid()),str(os.getgid())],capture_output=True,text=True,timeout=20)
    if result.returncode:raise OSError('new-only native root provision failed: '+result.stderr[-1500:])
    if result.stdout.strip() not in ('created','existing'):raise ValueError('native root provisioning witness missing')
    return dict(admission(),created=result.stdout.strip()=='created',provision_exit=result.returncode)


def native(provision):
    try:row={'schema':1,'phase':'captured-native-root','actual_exit':0,'root':ensure(provision),'actor_uid':os.getuid(),'actor_gid':os.getgid(),'boot_id':Path('/proc/sys/kernel/random/boot_id').read_text().strip()}
    except Exception as error:row={'schema':1,'phase':'captured-native-root','actual_exit':3,'error':{'type':type(error).__name__,'errno':getattr(error,'errno',None),'message':str(error)[:2000]}}
    print(json.dumps(row,separators=(',',':')));return row['actual_exit']


def preflight(args):
    if args.root!=str(ROOT) or not re.fullmatch(r'[0-9a-f]{40}',args.source):raise ValueError('unsupported captured root or inexact source')
    if not re.fullmatch(r'user@[A-Za-z0-9_.:-]+',args.host) or not 1<=args.port<=65535 or not re.fullmatch(r'bx_[A-Za-z0-9]+',args.box):raise ValueError('invalid actual endpoint identity')
    code=Path(__file__).read_bytes();row={'schema':1,'phase':'captured-native-root','source':args.source,'requested_box_id':args.box,'host':args.host,'port':args.port,'root':args.root,'helper_sha256':hashlib.sha256(code).hexdigest(),'actual_exit':3,'supervisor_created':False,'source_uploaded':False,'queried_at_epoch':time.time()}
    argv=['ssh','-o','BatchMode=yes','-o','StrictHostKeyChecking=no','-o','UserKnownHostsFile=/dev/null','-o','LogLevel=ERROR','-o','ConnectTimeout=20','-i',str(Path.home()/'.ssh/ascii_box_ed25519'),'-p',str(args.port),args.host,'timeout --kill-after=5 40 /usr/bin/python3 -I - native --provision-new']
    try:
        result=subprocess.run(argv,input=code,capture_output=True,timeout=50)
        row['transport_exit']=result.returncode
        if len(result.stdout)>65536:raise ValueError('oversized native prelaunch receipt')
        received=json.loads(result.stdout)
        if received.get('schema')!=1 or received.get('phase')!='captured-native-root' or type(received.get('actual_exit')) is not int:raise ValueError('malformed native root admission')
        row['native']=received
        if result.returncode==received['actual_exit']==0:
            admitted=received.get('root');actual=admitted.get('mount') if isinstance(admitted,dict) else None
            if not isinstance(admitted,dict) or admitted.get('path')!=args.root or admitted.get('mode')!=0o700 or type(admitted.get('created')) is not bool or admitted.get('uid')!=received.get('actor_uid') or not isinstance(actual,dict) or actual.get('filesystem') not in NATIVE_TYPES or type(actual.get('stat_device')) is not int or not re.fullmatch(r'[0-9a-f-]{36}',str(received.get('boot_id'))):raise ValueError('native captured-root receipt lacks actual ownership/mount identity')
            row['actual_exit']=0
        else:row['error']={'message':result.stderr.decode(errors='replace')[-1500:]}
    except Exception as error:row['error']={'type':type(error).__name__,'message':str(error)[:1500]}
    destination=Path(args.receipt);destination.parent.mkdir(parents=True,exist_ok=True)
    pending=destination.with_name(destination.name+'.'+uuid.uuid4().hex+'.pending')
    with pending.open('x') as stream:json.dump(row,stream,separators=(',',':'));stream.write('\n');stream.flush();os.fsync(stream.fileno())
    os.link(pending,destination);pending.unlink()
    print(json.dumps(row,separators=(',',':')));return row['actual_exit']


def main():
    p=argparse.ArgumentParser(description=__doc__);subs=p.add_subparsers(dest='mode',required=True)
    a=subs.add_parser('native');a.add_argument('--provision-new',action='store_true')
    a=subs.add_parser('preflight')
    for field in ('source','host','box','receipt'):a.add_argument('--'+field,required=True)
    a.add_argument('--port',type=int,required=True);a.add_argument('--root',choices=('/srv/amazing-proof',),required=True)
    args=p.parse_args()
    return native(args.provision_new) if args.mode=='native' else preflight(args)
if __name__=='__main__':
    try:raise SystemExit(main())
    except Exception as error:print('native-proof-root: '+str(error),file=sys.stderr);raise SystemExit(3)
