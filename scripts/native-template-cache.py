#!/usr/bin/env python3
"""Inventory/opt-in retirement of two inherited, unused generic Cargo caches.

No source-specific targets, directory deletion, Boat lifecycle or raw recovery.
Actual leases and Cargo locks protect live/unknown writers. Runnable, shared,
unknown ELF, generated source/raw and caller-protected operands stay in place.
"""
import argparse
import contextlib
import datetime
import fcntl
import hashlib
import json
import os
import pathlib
import pwd
import re
import stat
import struct
import subprocess
import sys
import tempfile
import time

NAMES = ('target', 'target-shell-shell')
INCREMENTAL = {'dep-graph.bin', 'query-cache.bin', 'work-products.bin'}
SOURCE_SUFFIXES = {'.rs','.c','.cc','.cpp','.h','.hpp','.js','.py','.sh','.toml','.lock','.log','.wasm','.wat'}


class CacheError(Exception):
    pass


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream,'sha256').hexdigest()


def canonical(value):
    path=pathlib.Path(value)
    if not path.is_absolute() or path.resolve()!=path:
        raise CacheError('operand must be absolute without symlink components')
    return path


def publish(path,value):
    path.parent.mkdir(parents=True,exist_ok=True)
    with tempfile.NamedTemporaryFile(mode='w',dir=path.parent,delete=False) as stream:
        temporary=pathlib.Path(stream.name)
        json.dump(value,stream,separators=(',',':'),sort_keys=True);stream.write('\n')
        stream.flush();os.fsync(stream.fileno())
    os.replace(temporary,path)


def free(path):
    while not path.exists():path=path.parent
    row=os.statvfs(path)
    return {'path':str(path),'device':path.stat().st_dev,'available_bytes':row.f_bavail*row.f_frsize}


def classify(path,relative):
    with path.open('rb') as stream:header=stream.read(64)
    if header.startswith(b'\x7fELF'):
        if len(header)<20 or header[4] not in (1,2) or header[5] not in (1,2) or header[6]!=1:
            return 'protected-unknown-elf'
        if len(header)<(52 if header[4]==1 else 64):return 'protected-unknown-elf'
        endian='<' if header[5]==1 else '>'
        if struct.unpack_from(endian+'I',header,20)[0]!=1 or struct.unpack_from(endian+'H',header,40 if header[4]==1 else 52)[0]!=(52 if header[4]==1 else 64):
            return 'protected-unknown-elf'
        kind=struct.unpack_from(endian+'H',header,16)[0]
        return {1:'recoverable-elf-rel',2:'protected-executable',3:'protected-shared-object'}.get(kind,'protected-unknown-elf')
    if path.suffix in SOURCE_SUFFIXES:return 'protected-source-raw'
    if path.suffix=='.rlib' and header.startswith(b'!<arch>\n'):return 'recoverable-rlib'
    if path.suffix=='.rmeta' and header.startswith(b'rust\x00'):return 'recoverable-rmeta'
    if 'incremental' in relative.parts and path.name in INCREMENTAL:return 'recoverable-incremental'
    return 'protected-unknown'


def leases(home):
    root=home/'.amazing-busy'
    own=os.environ.get('AMAZING_NATIVE_BUSY_LEASE')
    lane=os.environ.get('LANE')
    if own:
        own_path=canonical(own)
        if own_path.parent!=root or not lane or not re.fullmatch(re.escape(lane)+r'\.[0-9]+',own_path.name) or not own_path.is_file():
            raise CacheError('own native lease must match authentic wrapper lane/lease')
    else:own_path=None
    held=[]
    if root.exists():
        canonical(str(root))
        for path in root.iterdir():
            if path!=own_path:held.append(str(path))
    return held


@contextlib.contextmanager
def locks(roots):
    with contextlib.ExitStack() as stack:
        for root in roots:
            for profile in ('debug','release'):
                path=root/profile/'.cargo-lock'
                if path.exists():
                    canonical(str(path))
                    stream=stack.enter_context(path.open('r+'))
                    try:fcntl.flock(stream,fcntl.LOCK_EX|fcntl.LOCK_NB)
                    except BlockingIOError:raise CacheError('live generic Cargo target lock; no retirement')
        yield


def inventory(home,protected):
    rows=[]
    roots=[home/'amazing'/name for name in NAMES]
    for root in roots:
        canonical(str(root))
        if not root.exists():continue
        if not root.is_dir():raise CacheError('generic cache root must be an actual directory')
        for directory,dirs,files in os.walk(root,followlinks=False):
            for name in dirs+files:
                path=pathlib.Path(directory)/name
                if path.is_symlink():raise CacheError('generic cache symlink; no automatic retirement')
            for name in files:
                path=pathlib.Path(directory)/name
                info=path.stat()
                if not stat.S_ISREG(info.st_mode):raise CacheError('non-regular generic artifact')
                relative=path.relative_to(root)
                kind=classify(path,relative)
                if any(path==p or p in path.parents for p in protected):kind='protected-explicit'
                if info.st_nlink>1:kind='protected-multiple-links'
                rows.append({'path':str(path),'relative':str(relative),'root':str(root),'kind':kind,
                             'bytes':info.st_size,'allocated_bytes':info.st_blocks*512,
                             'device':info.st_dev,'inode':info.st_ino,'mtime_ns':info.st_mtime_ns})
    return roots,rows


def protection(args):
    paths={canonical(p) for p in args.protected_path}
    witnesses=[]
    for operand in args.protected_manifest:
        path=canonical(operand);value=json.loads(path.read_text())
        if value.get('schema')!=1 or not isinstance(value.get('artifacts'),list):
            raise CacheError('explicit protected manifest requires retained artifact schema')
        for row in value['artifacts']:
            paths.add(canonical(row['blob']))
            if value.get('target') and row.get('relative_path'):
                relative=pathlib.Path(row['relative_path'])
                if relative.is_absolute() or '..' in relative.parts:raise CacheError('invalid protected artifact relative path')
                paths.add(canonical(str(pathlib.Path(value['target'])/relative)))
        witnesses.append({'path':str(path),'sha256':sha(path)})
    return paths,witnesses


def operate(args):
    home=canonical(args.home_root)
    actual_home=pathlib.Path(pwd.getpwuid(os.getuid()).pw_dir)
    if home!=actual_home:
        if not args.fixture_home or pathlib.Path('/tmp') not in home.parents or not any(p.name.startswith('native-template-controls.') for p in home.parents):
            raise CacheError('production candidates require actual native user HOME; fixture scope must be explicit')
    output=canonical(args.output);raw=canonical(args.raw_manifest)
    protected,witnesses=protection(args)
    roots=[home/'amazing'/name for name in NAMES]
    if any(output==r or r in output.parents or raw==r or r in raw.parents for r in roots):
        raise CacheError('raw/compact evidence must stay outside generic cache candidates')
    held=leases(home)
    if args.mode=='retire' and held:raise CacheError('live/UNKNOWN peer lease; no generic cache retirement')
    with locks(roots):
        roots,rows=inventory(home,protected)
        if args.mode=='retire':
            if not args.confirm_reconstructible:raise CacheError('retirement requires explicit reconstructible policy opt-in')
            if leases(home):raise CacheError('new live/UNKNOWN peer lease; no retirement')
            for witness in witnesses:
                if sha(pathlib.Path(witness['path']))!=witness['sha256']:raise CacheError('protected manifest changed')
        before=[free(home),free(pathlib.Path('/tmp'))]
        counts={}
        for row in rows:
            group=counts.setdefault(row['kind'],{'files':0,'logical_bytes':0,'allocated_bytes':0})
            group['files']+=1;group['logical_bytes']+=row['bytes'];group['allocated_bytes']+=row['allocated_bytes']
        value={'schema':1,'utc':now(),'mode':args.mode,'source':args.source,'home_root':str(home),
               'roots':[str(p) for p in roots],'rows':rows,'held_peer_leases':held,'protected_manifests':witnesses}
        # Raw manifest survives before the first selective unlink.
        publish(raw,value)
        removed=[]
        error=None
        if args.mode=='retire':
            try:
                for row in rows:
                    if not row['kind'].startswith('recoverable-'):continue
                    if leases(home):raise CacheError('new live/UNKNOWN peer lease; remaining files preserved')
                    path=canonical(row['path']);info=path.stat()
                    if (info.st_dev,info.st_ino,info.st_mtime_ns,info.st_size,info.st_nlink)!=(row['device'],row['inode'],row['mtime_ns'],row['bytes'],1):
                        raise CacheError('generic candidate changed; remaining files preserved')
                    row['sha256_before_unlink']=sha(path)
                    after_hash=path.stat()
                    if (after_hash.st_dev,after_hash.st_ino,after_hash.st_mtime_ns,after_hash.st_size,after_hash.st_nlink)!=(row['device'],row['inode'],row['mtime_ns'],row['bytes'],1):
                        raise CacheError('generic candidate changed while hashing; remaining files preserved')
                    path.unlink();removed.append(row)
            except (CacheError,OSError) as caught:
                error=str(caught)
        after=[free(home),free(pathlib.Path('/tmp'))]
        value.update(removed=removed,completed_at=now(),free_before=before,free_after=after,error=error,actual_exit=3 if error else 0)
        publish(raw,value)
        summary={'schema':1,'source':args.source,'mode':args.mode,'counts':counts,
                 'removed_files':len(removed),'removed_logical_bytes':sum(r['bytes'] for r in removed),
                 'removed_allocated_bytes':sum(r['allocated_bytes'] for r in removed),
                 'free_before':before,'free_after':after,'held_peer_leases':held,
                 'raw':{'path':str(raw),'bytes':raw.stat().st_size,'sha256':sha(raw)},
                 'source_specific_targets_touched':False,'whole_directories_deleted':False,'actual_exit':3 if error else 0,'error':error}
        publish(output,summary);print(json.dumps(summary,separators=(',',':')))
    return 3 if error else 0


def measure(args):
    if not args.command:raise CacheError('measurement requires one whole command')
    if args.command[:1]==['--']:args.command=args.command[1:]
    native=canonical(args.native_root);raw=canonical(args.raw_manifest);output=canonical(args.output)
    start=free(native);minimum=start['available_bytes'];samples=0
    raw.parent.mkdir(parents=True,exist_ok=True)
    profile={key:os.environ.get(key) for key in ('CARGO_PROFILE_DEV_DEBUG','CARGO_PROFILE_TEST_DEBUG','CARGO_INCREMENTAL')}
    if profile!={'CARGO_PROFILE_DEV_DEBUG':'line-tables-only','CARGO_PROFILE_TEST_DEBUG':'line-tables-only','CARGO_INCREMENTAL':'0'}:
        raise CacheError('native footprint requires explicit line-tables DEV/TEST and incremental0')
    with raw.open('xb') as stream:
        child=subprocess.Popen(args.command,stdout=stream,stderr=subprocess.STDOUT)
        while True:
            minimum=min(minimum,free(native)['available_bytes']);samples+=1
            code=child.poll()
            if code is not None:break
            time.sleep(.5)
        stream.flush();os.fsync(stream.fileno())
    code=code if code>=0 else 128-code
    summary={'schema':1,'source':args.source,'actual_exit':code,'profile':profile,'filesystem_before':start,
             'filesystem_after':free(native),'sampled_min_available_bytes':minimum,'sampled_peak_additional_bytes':max(0,start['available_bytes']-minimum),
             'samples':samples,'sampling_interval_seconds':.5,'command':args.command,
             'raw':{'path':str(raw),'bytes':raw.stat().st_size,'sha256':sha(raw)}}
    publish(output,summary);print(json.dumps(summary,separators=(',',':')));return code


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode',choices=('inventory','retire','measure'))
    parser.add_argument('--source',required=True)
    parser.add_argument('--home-root',default='/home/user')
    parser.add_argument('--fixture-home',action='store_true',help='native controls only: explicit /tmp/native-template-controls.* namespace')
    parser.add_argument('--output',required=True)
    parser.add_argument('--raw-manifest',required=True)
    parser.add_argument('--protected-path',action='append',default=[])
    parser.add_argument('--protected-manifest',action='append',default=[])
    parser.add_argument('--confirm-reconstructible',action='store_true')
    parser.add_argument('--native-root',default='/tmp')
    args, command=parser.parse_known_args()
    args.command=command
    if args.mode!='measure' and command:parser.error('unexpected operands outside measurement')
    try:
        if not re.fullmatch('[0-9a-f]{40}',args.source) or os.environ.get('COMMIT')!=args.source:
            raise CacheError('exact source must match native launcher COMMIT')
        return measure(args) if args.mode=='measure' else operate(args)
    except (CacheError,OSError,ValueError,KeyError) as error:
        print(json.dumps({'schema':1,'actual_exit':3,'error':str(error),'utc':now()}));return 3


if __name__=='__main__':sys.exit(main())
