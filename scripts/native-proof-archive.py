#!/usr/bin/env python3
"""Archive declared terminal proof on captured native /srv; never delete inputs."""
import argparse
import contextlib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import sys
import time
import uuid

spec = importlib.util.spec_from_file_location('native_pack', Path(__file__).with_name('native-proof-pack.py'))
pack = importlib.util.module_from_spec(spec); spec.loader.exec_module(pack)

ROOT = Path('/srv/amazing-proof')
NATIVE_TYPES = {'ext2', 'ext3', 'ext4', 'xfs', 'btrfs', 'overlay'}


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def identifier(value):
    if not re.fullmatch(r'[a-z0-9][a-z0-9._-]{0,127}', value):
        raise ValueError('invalid owner/job identity')
    return value


def canonical(path):
    path = Path(path)
    if not path.is_absolute() or path.resolve(strict=True) != path:
        raise ValueError('proof operand must exist with no symlink components: '+str(path))
    return path


def unescape(value):
    return re.sub(r'\\([0-7]{3})', lambda m: chr(int(m[1], 8)), value)


def native_mount(path):
    path = canonical(path)
    matches = []
    for line in Path('/proc/self/mountinfo').read_text().splitlines():
        left, right = line.split(' - ', 1)
        fields, filesystem = left.split(), right.split()
        mount = Path(unescape(fields[4]))
        if path == mount or mount in path.parents:
            matches.append((len(mount.parts), {'mountpoint': str(mount),
                            'filesystem': filesystem[0], 'source': unescape(filesystem[1]),
                            'device': fields[2]}))
    if not matches:
        raise ValueError('no actual mount provenance')
    result = max(matches, key=lambda row: row[0])[1]
    if result['filesystem'] not in NATIVE_TYPES:
        raise ValueError(json.dumps({'path':str(path),'type':'filesystem','error':'proof requires native disk, not FUSE/ramfs/tmpfs: '+result['filesystem']},separators=(',',':')))
    result['stat_device'] = path.stat().st_dev
    return result


def exclusions(path):
    # Snapshot filters are relative to their containing directory. Refuse any
    # potentially applicable policy instead of pretending to implement its
    # gitignore matcher. No .gitignore interpretation or silent omissions.
    policies = []
    for parent in [path, *path.parents]:
        for name in ('.boxignore', '.oneignore'):
            policy = parent / name
            if policy.exists() or policy.is_symlink(): policies.append(str(policy))
    if policies:
        raise ValueError('snapshot exclusion policy requires explicit resolution: ' + ','.join(policies))


def root_admission():
    root = canonical(ROOT)
    if not root.is_dir() or root.stat().st_uid != os.getuid() or stat.S_IMODE(root.stat().st_mode) != 0o700:
        raise ValueError('root must provision owner-private /srv/amazing-proof mode0700')
    exclusions(root)
    return native_mount(root)


def fingerprint(info):
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns,
            info.st_ctime_ns, stat.S_IMODE(info.st_mode))


def inventory_details(roots, capture_links=False, file_inputs=()):
    rows, directories, links = [], [], []
    admitted = {label:canonical(path) for label,path in roots.items()}
    def failure(path, kind, message):
        raise ValueError(json.dumps({'path':str(path),'type':kind,'error':message},separators=(',',':')))
    def record(label, source, file, relative):
        before=file.stat()
        if not stat.S_ISREG(before.st_mode):failure(file,'nonregular','only regular native targets can be copied')
        sha=digest(file)
        if fingerprint(file.stat())!=fingerprint(before):failure(file,'regular','input changed while hashing')
        with file.open('rb') as stream:elf=stream.read(4)==b'\x7fELF'
        rows.append({'label':label,'path':relative,'bytes':before.st_size,'sha256':sha,
                     'mode':stat.S_IMODE(before.st_mode),'elf':elf,'source_identity':list(fingerprint(before))})
    for label,source in sorted(admitted.items()):
        native_mount(source)
        if source.is_file() and label in file_inputs:
            directories.append({'label':label,'path':'.'});record(label,source,source,source.name);continue
        if not source.is_dir():failure(source,'operand','directory input or explicit regular target required')
        for directory, dirs, files in os.walk(source,followlinks=False):
            parent=Path(directory);native_mount(parent)
            if any((parent/name).exists() or (parent/name).is_symlink() for name in ('.boxignore','.oneignore')):
                failure(parent,'snapshot-policy','copied exclusion policy requires explicit resolution')
            directories.append({'label':label,'path':str(parent.relative_to(source))})
            for name in dirs+files:
                link=parent/name
                if not link.is_symlink():continue
                raw=os.readlink(link);info=link.lstat()
                if not capture_links:failure(link,'symlink','explicit native-target capture required; target='+raw)
                try:resolved=link.resolve(strict=True)
                except (OSError,RuntimeError) as error:failure(link,'symlink','unresolved target='+raw+'; '+str(error))
                owners=[(key,root) for key,root in admitted.items() if resolved==root or (root.is_dir() and root in resolved.parents)]
                if not owners:failure(link,'symlink','undeclared/escaping target='+raw+'; resolved='+str(resolved))
                native_mount(resolved)
                kind='directory' if resolved.is_dir() else 'regular' if resolved.is_file() else 'nonregular'
                if kind=='nonregular':failure(link,'symlink','unsupported native target='+str(resolved))
                key,root=max(owners,key=lambda item:len(item[1].parts))
                links.append({'label':label,'path':str(link.relative_to(source)),'target':raw,
                              'resolved_target':str(resolved),'target_label':key,
                              'target_path':str(resolved.relative_to(root)) if root.is_dir() else root.name,
                              'target_kind':kind,'mode':stat.S_IMODE(info.st_mode),'uid':info.st_uid,'gid':info.st_gid,'mtime_ns':info.st_mtime_ns,'source_identity':list(fingerprint(info))})
                if os.readlink(link)!=raw or fingerprint(link.lstat())!=fingerprint(info):failure(link,'symlink','link changed during capture')
            for name in sorted(files):
                file=parent/name
                if not file.is_symlink():record(label,source,file,str(file.relative_to(source)))
    return (sorted(rows,key=lambda row:(row['label'],row['path'])),
            sorted(directories,key=lambda row:(row['label'],row['path'])),
            sorted(links,key=lambda row:(row['label'],row['path'])))


def inventory(roots):
    return inventory_details(roots)[:2]


def original_path(manifest,row):
    root=Path(manifest['inputs'][row['label']])
    return root if row['label'] in manifest.get('file_inputs',[]) else root/row['path']


def fsync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try: os.fsync(fd)
    finally: os.close(fd)


def save(path, value):
    # Exclusive publication; an existing completed or partial job is preserved.
    pending = path.with_name(path.name + '.' + uuid.uuid4().hex + '.pending')
    with pending.open('x') as stream:
        json.dump(value, stream, separators=(',', ':'), sort_keys=True)
        stream.write('\n'); stream.flush(); os.fsync(stream.fileno())
    os.link(pending, path); pending.unlink(); fsync_directory(path.parent)


def terminal(path, source, job):
    path = canonical(path)
    row = json.loads(path.read_text())
    provenance = row.get('job') if isinstance(row.get('job'), dict) else row
    actual_source = provenance.get('source_commit', provenance.get('source'))
    actual_job = provenance.get('job_id', provenance.get('job'))
    complete = row.get('state') == 'TERMINAL' or row.get('completed') is True
    code = row.get('actual_exit', row.get('command_exit', row.get('engine_exit')))
    if not complete or actual_source != source or actual_job != job or type(code) is not int:
        raise ValueError('actual terminal source/job/exit receipt required; live jobs untouched')
    return {'path': str(path), 'bytes': path.stat().st_size, 'sha256': digest(path), 'actual_exit': code}


def archive(lane, source, job, roots, terminal_path, copy_budget, reserve, capture_links=False, targets=None, pack_targets=(), pack_data_targets=()):
    identifier(lane); identifier(job)
    if not re.fullmatch(r'[0-9a-f]{40}', source): raise ValueError('exact source commit required')
    if copy_budget < 0 or reserve < 0: raise ValueError('negative capacity budget')
    mount = root_admission()
    producer = terminal(terminal_path, source, job)
    targets=targets or {}
    data=set(pack_data_targets);strict=set(pack_targets)
    if data&strict:raise ValueError('conflicting strict/data-only target modes')
    packed=strict|data
    if not packed.issubset(targets):raise ValueError('only explicitly declared native target labels can be packed')
    if set(roots)&set(targets):raise ValueError('duplicate input/target labels')
    file_inputs=[label for label,path in targets.items() if canonical(path).is_file()]
    roots = {identifier(label): canonical(path) for label, path in {**roots,**targets}.items()}
    if not roots: raise ValueError('no declared raw/ELF inputs')
    for path in roots.values():
        if ROOT == path or ROOT in path.parents or path in ROOT.parents:
            raise ValueError('archive/input root overlap')
    destination = ROOT / lane / source / job
    if destination.exists() or destination.is_symlink(): raise ValueError('prior archive/partial preserved')
    rows, directories, links = inventory_details(roots,capture_links,file_inputs)
    required = sum(row['bytes'] for row in rows if row['label'] not in packed or (row['label'] in data and row['elf'])) + 4096 * (len(rows) + len(directories)) + 65536
    if any(row['elf'] and row['label'] in strict for row in rows):raise ValueError('native ELF target packing is not admitted')
    available = shutil.disk_usage(ROOT).free
    if required > copy_budget or available < required + reserve:
        raise OSError(28, 'explicit archive capacity unavailable; inputs untouched')
    destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    canonical(destination.parent); exclusions(destination.parent); native_mount(destination.parent)
    destination.mkdir(mode=0o700)
    header = {'schema': 1, 'lane': lane, 'source': source, 'job': job,
              'destination': str(destination), 'inputs': {k: str(v) for k,v in roots.items()},
              'terminal_producer': producer, 'capture_links':capture_links,'file_inputs':file_inputs,'links':links,'mount': mount, 'required_bytes': required,
              'available_bytes': available, 'copy_budget_bytes': copy_budget,
              'reserve_bytes': reserve, 'packed_targets':sorted(packed), 'data_packed_targets':sorted(data), 'packs':{}, 'complete': False, 'source_retained': True}
    save(destination / 'start.manifest.json', header)
    try:
        packs={}
        if packed:(destination/'packs').mkdir(mode=0o700)
        for label in sorted(packed):
            selected=[row for row in rows if row['label']==label and not (label in data and row['elf'])]
            selected_dirs=[row for row in directories if row['label']==label]
            record=pack.write_pack(destination/'packs'/(label+'.tar.gz'),selected,selected_dirs,lambda row:original_path(header,row),fingerprint,ROOT,required,sum(p['bytes'] for p in packs.values()),copy_budget,reserve)
            record['readback']=pack.readback(Path(record['path']),selected,selected_dirs)
            packs[label]=record
        required+=sum(record['bytes'] for record in packs.values())
        if required>copy_budget or shutil.disk_usage(ROOT).free<required-sum(record['bytes'] for record in packs.values())+reserve:raise OSError(28,'explicit lossless archive capacity unavailable; original inputs untouched')
        header.update(packs=packs,required_bytes=required)
        for directory in directories:
            if directory['label'] in strict:continue
            (destination / 'payload' / directory['label'] / directory['path']).mkdir(parents=True, exist_ok=True, mode=0o700)
        for row in rows:
            if row['label'] in packed and not (row['label'] in data and row['elf']):continue
            original = original_path(header,row)
            final = destination / 'payload' / row['label'] / row['path']
            if list(fingerprint(original.stat())) != row['source_identity']: raise ValueError('input changed before copy')
            with original.open('rb') as source_stream, final.open('xb') as output:
                shutil.copyfileobj(source_stream, output, 1024 * 1024)
                output.flush(); os.fsync(output.fileno())
            final.chmod(row['mode'])
            with final.open('rb') as stream: os.fsync(stream.fileno())
            if final.stat().st_size != row['bytes'] or digest(final) != row['sha256']:
                raise ValueError('destination byte/hash readback failed')
        after, after_directories, after_links = inventory_details(roots,capture_links,file_inputs)
        if rows != after or directories != after_directories or links != after_links: raise ValueError('input tree changed during archive; no completed publication')
        if terminal(terminal_path, source, job) != producer: raise ValueError('terminal producer receipt changed')
        copied, copied_directories = inventory({label:destination/'payload'/label for label in roots if label not in strict})
        fields=('label','path','bytes','sha256','mode','elf')
        if [{k:r[k] for k in fields} for r in copied] != [{k:r[k] for k in fields} for r in rows if r['label'] not in packed or (r['label'] in data and r['elf'])] or copied_directories != [r for r in directories if r['label'] not in strict]:
            raise ValueError('final archived tree differs from declared original bytes')
        for label,record in packs.items():
            if digest(Path(record['path']))!=record['sha256']:raise ValueError('packed capture changed before sealing')
            pack.readback(Path(record['path']),[r for r in rows if r['label']==label and not (label in data and r['elf'])],[r for r in directories if r['label']==label])
        if packs:fsync_directory(destination/'packs')
        if root_admission() != mount:raise ValueError('captured native root mount changed')
        # Seal every payload directory before the complete receipt.
        for directory, dirs, files in os.walk(destination / 'payload', topdown=False): fsync_directory(directory)
        for parent in [destination, destination.parent, destination.parent.parent, ROOT]: fsync_directory(parent)
        manifest = dict(header, complete=True, files=rows, directories=directories,
                        file_count=len(rows), elf_count=sum(row['elf'] for row in rows),
                        logical_bytes=sum(row['bytes'] for row in rows), link_count=len(links),completed_ns=time.time_ns())
        save(destination / 'archive.manifest.json', manifest)
        for parent in [destination, destination.parent, destination.parent.parent, ROOT]: fsync_directory(parent)
        return {'schema': 1, 'complete': True, 'source': source, 'lane': lane, 'job': job,
                'destination': str(destination), 'files': len(rows), 'elf_files': manifest['elf_count'],
                'packed_targets':sorted(packed),'data_packed_targets':sorted(data),'plain_elf_files':manifest['elf_count'],'physical_required_bytes':required,'logical_bytes': manifest['logical_bytes'],'links':len(links),'link_storage':'exact metadata; admitted target payloads copied independently, no escaping archive symlinks','mount': mount, 'source_retained': True,
                'archive_manifest': {'path': str(destination/'archive.manifest.json'),
                    'bytes': (destination/'archive.manifest.json').stat().st_size,
                    'sha256': digest(destination/'archive.manifest.json')},
                'snapshot_coverage': 'documented /srv; actual restored member verification is root lifecycle qualification'}
    except BaseException as error:
        # Preserve all partial bytes and the true failure; never erase originals.
        try: save(destination/'failure.manifest.json', dict(header,error=type(error).__name__,message=str(error)))
        except OSError: pass
        raise


def verify(manifest_path, expected_sha, archive_exit):
    if archive_exit != 0:raise ValueError('actual archival command did not succeed')
    manifest_path=canonical(manifest_path)
    if ROOT not in manifest_path.parents or manifest_path.name!='archive.manifest.json':raise ValueError('not a captured-native archive manifest')
    root_admission()
    if digest(manifest_path)!=expected_sha:raise ValueError('archive manifest identity changed')
    destination=manifest_path.parent
    if (destination/'failure.manifest.json').exists():raise ValueError('archive has a failed/partial producer')
    manifest=json.loads(manifest_path.read_text())
    if manifest.get('complete') is not True or manifest.get('destination')!=str(destination):raise ValueError('archive not complete')
    packed=set(manifest.get('packed_targets',[]));data=set(manifest.get('data_packed_targets',[]));strict=packed-data
    if not data.issubset(packed):raise ValueError('data-only target must be independently packed')
    if not packed.issubset(manifest['inputs']) or any(r['elf'] and r['label'] in strict for r in manifest['files']):raise ValueError('inadmissible packed target manifest')
    if packed!=set(manifest.get('packs',{})):raise ValueError('packed target inventory mismatch')
    roots={label:destination/'payload'/label for label in manifest['inputs'] if label not in strict}
    rows,directories=inventory(roots);fields=('label','path','bytes','sha256','mode','elf')
    if [{k:r[k] for k in fields} for r in rows] != [{k:r[k] for k in fields} for r in manifest['files'] if r['label'] not in packed or (r['label'] in data and r['elf'])] or directories!=[r for r in manifest['directories'] if r['label'] not in strict]:
        raise ValueError('archived raw/ELF bytes or paths changed')
    for label in sorted(packed):
        identifier(label);record=manifest['packs'][label];path=destination/'packs'/(label+'.tar.gz')
        canonical(path);native_mount(path)
        if record['path']!=str(path) or path.stat().st_size!=record['bytes'] or digest(path)!=record['sha256']:raise ValueError('packed archive identity changed')
        pack.readback(path,[r for r in manifest['files'] if r['label']==label and not (label in data and r['elf'])],[r for r in manifest['directories'] if r['label']==label])
    for link in manifest.get('links',[]):
        label=identifier(link['target_label']);relative=Path(link['target_path'])
        if label not in manifest['inputs'] or relative.is_absolute() or '..' in relative.parts:raise ValueError('link metadata escapes captured target')
        if label in packed:
            if link['target_kind']=='directory' and not any(r['label']==label and r['path']==str(relative) for r in manifest['directories']):raise ValueError('packed directory link target missing')
            if link['target_kind']=='regular' and not any(r['label']==label and r['path']==str(relative) for r in manifest['files']):raise ValueError('packed regular link target missing')
            if link['target_kind'] not in ('directory','regular'):raise ValueError('unsupported packed link target type')
            continue
        target=roots[label]/relative
        if link['target_kind']=='directory':
            if not target.is_dir():raise ValueError('captured directory link target missing')
        elif link['target_kind']=='regular':
            if not target.is_file():raise ValueError('captured regular link target missing')
        else:raise ValueError('unsupported captured link target type')
        canonical(target);native_mount(target)
    return {'schema':1,'complete':True,'actual_archive_exit':archive_exit,'destination':str(destination),
            'source':manifest['source'],'lane':manifest['lane'],'job':manifest['job'],
            'files':len(manifest['files']),'elf_files':sum(r['elf'] for r in manifest['files']),'links':len(manifest.get('links',[])),'archive_manifest_sha256':expected_sha}


def restore_target(manifest_path, expected_sha, archive_exit, label, destination, budget, reserve):
    verify(manifest_path,expected_sha,archive_exit)
    manifest=json.loads(Path(manifest_path).read_text());identifier(label)
    if label not in manifest.get('packed_targets',[]):raise ValueError('not an independently verified packed target')
    destination=Path(destination)
    if not destination.is_absolute() or destination.exists() or destination.is_symlink():raise ValueError('native restoration needs a fresh absolute namespace')
    parent=canonical(destination.parent);native_mount(parent);exclusions(parent)
    if destination!=parent/destination.name:raise ValueError('noncanonical restoration destination')
    if any(root==destination or root in destination.parents or destination in root.parents for root in [ROOT,*[Path(x) for x in manifest['inputs'].values()]]):raise ValueError('restoration overlaps original or captured proof')
    rows=[r for r in manifest['files'] if r['label']==label];directories=[r for r in manifest['directories'] if r['label']==label]
    required=sum(r['bytes'] for r in rows)+4096*(len(rows)+len(directories))+65536
    if budget<required or reserve<0 or shutil.disk_usage(parent).free<required+reserve:raise OSError(28,'full native restoration capacity unavailable; originals retained')
    data=set(manifest.get('data_packed_targets',[]))
    plain=[r for r in rows if label in data and r['elf']]
    result=pack.readback(Path(manifest['packs'][label]['path']),[r for r in rows if r not in plain],directories,destination)
    for row in plain:
        source=Path(manifest['destination'])/'payload'/label/row['path'];final=destination/row['path']
        if source.is_symlink() or source.stat().st_size!=row['bytes'] or digest(source)!=row['sha256']:raise ValueError('plain ELF source changed before full restoration')
        with source.open('rb') as reader,final.open('xb') as writer:shutil.copyfileobj(reader,writer,1024*1024);writer.flush();os.fsync(writer.fileno())
        final.chmod(row['mode'])
        with final.open('rb') as check:
            if final.stat().st_size!=row['bytes'] or hashlib.file_digest(check,'sha256').hexdigest()!=row['sha256']:raise ValueError('plain ELF restoration readback mismatch')
            os.fsync(check.fileno())
    result.update(files=len(rows),logical_bytes=sum(r['bytes'] for r in rows),plain_elf_files=len(plain))
    for directory,dirs,files in os.walk(destination,topdown=False):fsync_directory(directory)
    fsync_directory(parent)
    return dict(result,destination=str(destination),source=manifest['source'],job=manifest['job'],label=label,archive_manifest_sha256=expected_sha,
                link_metadata=[r for r in manifest.get('links',[]) if r['label']==label],link_restoration='exact manifest metadata retained; no escaping native symlinks materialized')


@contextlib.contextmanager
def deadline(seconds):
    if not 1 <= seconds <= 6600 or signal.getitimer(signal.ITIMER_REAL) != (0.0,0.0): raise ValueError('invalid or nonowned archive timer')
    previous=signal.getsignal(signal.SIGALRM)
    def expired(signum,frame): raise TimeoutError('native proof archive deadline expired')
    signal.signal(signal.SIGALRM,expired); signal.setitimer(signal.ITIMER_REAL,seconds)
    try: yield
    finally: signal.setitimer(signal.ITIMER_REAL,0);signal.signal(signal.SIGALRM,previous)


def main():
    if len(sys.argv)>1 and sys.argv[1]=='restore-target':
        p=argparse.ArgumentParser(description='Restore every losslessly captured target byte into fresh native storage')
        p.add_argument('mode');p.add_argument('manifest');p.add_argument('--sha256',required=True);p.add_argument('--actual-archive-exit',required=True,type=int)
        p.add_argument('--label',required=True);p.add_argument('--destination',required=True);p.add_argument('--copy-budget-bytes',required=True,type=int);p.add_argument('--reserve-bytes',required=True,type=int);p.add_argument('--seconds',type=int,default=600)
        args=p.parse_args()
        try:
            with deadline(args.seconds):result=restore_target(args.manifest,args.sha256,args.actual_archive_exit,args.label,args.destination,args.copy_budget_bytes,args.reserve_bytes)
            code=0
        except Exception as error:result={'complete':False,'error':str(error)};code=124 if isinstance(error,TimeoutError) else 3
        result['actual_exit']=code;print(json.dumps(result,separators=(',',':')));return code
    if len(sys.argv)>1 and sys.argv[1]=='verify':
        p=argparse.ArgumentParser(description='Verify root stop-safety inputs; not an engine merge gate')
        p.add_argument('mode');p.add_argument('manifest');p.add_argument('--sha256',required=True);p.add_argument('--actual-archive-exit',required=True,type=int)
        args=p.parse_args()
        try:
            result=verify(args.manifest,args.sha256,args.actual_archive_exit);code=0
        except Exception as error:
            result={'schema':1,'complete':False,'error':str(error)};code=3
        print(json.dumps(result,separators=(',',':')));return code
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('lane','source','job','terminal-receipt'):p.add_argument('--'+name,required=True)
    p.add_argument('--input',action='append',required=True,help='label=absolute native terminal directory')
    p.add_argument('--copy-budget-bytes',type=int,required=True);p.add_argument('--reserve-bytes',type=int,required=True)
    p.add_argument('--seconds',type=int,default=600)
    p.add_argument('--capture-links',action='store_true')
    p.add_argument('--target',action='append',default=[],help='label=explicit admitted native directory or regular file')
    p.add_argument('--pack-data-target',action='append',default=[],help='losslessly pack target DATA, keeping every encountered native ELF PLAIN and independently readable')
    p.add_argument('--pack-target',action='append',default=[],help='losslessly capture every byte of this explicitly declared target label as native gzip tar')
    args=p.parse_args()
    try:
        pairs=[value.split('=',1) for value in args.input]
        if any(len(pair)!=2 for pair in pairs) or len({pair[0] for pair in pairs})!=len(pairs):raise ValueError('invalid or duplicate input labels')
        target_pairs=[value.split('=',1) for value in args.target]
        if any(len(pair)!=2 for pair in target_pairs) or len({pair[0] for pair in target_pairs})!=len(target_pairs):raise ValueError('invalid or duplicate target labels')
        with deadline(args.seconds):result=archive(args.lane,args.source,args.job,dict(pairs),args.terminal_receipt,args.copy_budget_bytes,args.reserve_bytes,args.capture_links,dict(target_pairs),args.pack_target,args.pack_data_target)
        code=0
    except Exception as error:
        code=124 if isinstance(error,TimeoutError) else 3
        result={'schema':1,'complete':False,'actual_exit':code,'source':args.source,'lane':args.lane,'job':args.job,'error':{'type':type(error).__name__,'errno':getattr(error,'errno',None),'message':str(error)}}
    result['actual_exit']=code
    print(json.dumps(result,separators=(',',':')),flush=True)
    return code

if __name__=='__main__':raise SystemExit(main())
