#!/usr/bin/env python3
"""Authenticated foreground job ownership, actual exits and compact transport.

A lost SSH observation never proves a native command stopped. Schema 2 binds
its complete bootstrap execution as well as the requested gate command. HOME
holds immutable start/child/exit/completion records; streaming diagnostics stay
native so a mounted HOME write failure cannot terminate an owned child. /proc witnesses distinguish
an owned live process from a reused PID or another boot. No job is detached.
"""
import argparse,base64,hashlib,json,os,pathlib,re,shlex,signal,subprocess,sys,time,uuid,tempfile

FIELDS=('schema','lane','source_commit','box_id','host','port','job_id','output_root','command_sha256')
COMPACT=re.compile(r'(?:[A-Za-z0-9_.-]+\.)?(?:compact|summary|manifest)\.json')
def digest(data):return hashlib.sha256(data).hexdigest()
def encoded(row):return base64.b64encode(json.dumps(row,separators=(',',':')).encode()).decode()
def decode(text):return json.loads(base64.b64decode(text))
def identity(row):return {k:row[k] for k in FIELDS+(('execution_sha256','source_archive_sha256') if row['schema']==2 else ())+(('captured_proof_root',) if row.get('captured_proof_root') else ())}
def marker(row):return pathlib.Path.home()/('amazing-completed-'+row['job_id']+'.json')
def ledger(row):return pathlib.Path.home()/'.amazing-jobs'/row['job_id']
def boot_id():return pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip()
def sync_directory(path):
    # Native Linux publication must persist the directory entry as well as its
    # file bytes. Local receipt capture keeps its existing portable file fsync.
    if sys.platform.startswith('linux'):
        fd=os.open(path,os.O_RDONLY|os.O_DIRECTORY)
        try:os.fsync(fd)
        finally:os.close(fd)

def save(path,row):
    # Same-directory publication prevents a reader observing partial JSON.
    # Never overwrite a receipt or silently reuse another job's namespace.
    path.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
    temp=path.with_name(path.name+'.'+uuid.uuid4().hex+'.pending')
    try:
        with temp.open('x') as f:json.dump(row,f,separators=(',',':'));f.write('\n');f.flush();os.fsync(f.fileno())
        os.link(temp,path)
        sync_directory(path.parent)
    finally:temp.unlink(missing_ok=True)
def proc(pid):
    p=pathlib.Path('/proc')/str(pid)
    try:
        fields=(p/'stat').read_text().rpartition(') ')[2].split()
        return {'pid':pid,'start_ticks':int(fields[19]),'state':fields[0],'parent_pid':int(fields[1]),
                'cmdline_sha256':digest((p/'cmdline').read_bytes())}
    except FileNotFoundError:return None

def valid_output_root(lane,root):
    if not re.fullmatch(r'[A-Za-z0-9_-]+',lane):raise ValueError('invalid lane')
    p=pathlib.PurePosixPath(root)
    if p.is_absolute() or '..' in p.parts or not str(p).startswith('amazing/out/'+lane+'/'):raise ValueError('invalid job output root')
    if not re.fullmatch(r'[A-Za-z0-9_./-]+',str(p)):raise ValueError('invalid job output root')
    return root

def valid(row):
    if type(row.get('schema')) is not int or row['schema'] not in (1,2):raise ValueError('unknown endpoint receipt schema')
    for key,pattern in [('lane',r'[A-Za-z0-9_-]+'),('source_commit',r'[0-9a-f]{40}'),('box_id',r'bx_[A-Za-z0-9]+'),('host',r'[A-Za-z0-9_.-]+@[A-Za-z0-9:.]+'),('job_id',r'[0-9a-f]{32}'),('command_sha256',r'[0-9a-f]{64}')]+([('execution_sha256',r'[0-9a-f]{64}'),('source_archive_sha256',r'[0-9a-f]{64}')] if row['schema']==2 else []):
        if not re.fullmatch(pattern,str(row.get(key,''))):raise ValueError('invalid '+key)
    if type(row['port']) is not int or not 1<=row['port']<=65535:raise ValueError('invalid port')
    if row.get('captured_proof_root') not in (None,'/srv/amazing-proof'):raise ValueError('unsupported captured native proof root')
    valid_output_root(row['lane'],row['output_root'])
    return row

def capture(args):
    if getattr(args,'command_file',None):args.command=pathlib.Path(args.command_file).read_text()
    if getattr(args,'execution_file',None):args.execution=pathlib.Path(args.execution_file).read_text()
    row=dict(schema=1,lane=args.lane,source_commit=args.source,box_id=args.box,host=args.host,port=args.port,job_id=uuid.uuid4().hex,output_root=args.root,command_sha256=digest(args.command.encode()))
    if args.execution is not None:
        row.update(schema=2,execution_sha256=digest(args.execution.encode()),source_archive_sha256=args.source_archive_sha256)
    if getattr(args,'captured_proof_root',None):row['captured_proof_root']=args.captured_proof_root
    valid(row);save(pathlib.Path(args.receipt),row);print(encoded(row))

def catalog(row):
    root=pathlib.Path.home()/row['output_root'];files={};total=0
    if root.exists():
        if root.is_symlink():raise ValueError('symlink job root')
        for p in sorted(root.rglob('*.json')):
            if not COMPACT.fullmatch(p.name):continue
            if p.is_symlink() or not p.resolve().is_relative_to(root.resolve()):raise ValueError('nonlocal compact operand')
            with p.open('rb') as stream:
                data=stream.read();os.fsync(stream.fileno())
            sync_directory(p.parent)
            total+=len(data)
            if len(data)>524288 or total>2097152:raise ValueError('compact evidence limit exceeded')
            files[str(p.relative_to(pathlib.Path.home()))]={'bytes':len(data),'sha256':digest(data)}
    return files

def diagnostic_file(row):
    parent=pathlib.Path('/tmp')
    if row.get('captured_proof_root'):
        parent=pathlib.Path(row['captured_proof_root'])
        info=parent.stat()
        if parent.resolve(strict=True)!=parent or not parent.is_dir() or info.st_uid!=os.getuid() or info.st_mode&0o077:raise ValueError('captured raw root must be canonical owner-private')
        mounts=[]
        for line in pathlib.Path('/proc/self/mountinfo').read_text().splitlines():
            left,right=line.split(' - ',1);mount=pathlib.Path(left.split()[4])
            if parent==mount or mount in parent.parents:mounts.append((len(mount.parts),right.split()[0]))
        if not mounts or max(mounts)[1] not in ('ext2','ext3','ext4','xfs','btrfs','overlay'):raise ValueError('captured raw requires native disk, never FUSE/tmpfs')
        for ancestor in [parent,*parent.parents]:
            if any((ancestor/name).exists() or (ancestor/name).is_symlink() for name in ('.boxignore','.oneignore')):raise ValueError('captured raw root has unresolved snapshot exclusions')
    directory=pathlib.Path(tempfile.mkdtemp(prefix='amazing-job-'+row['job_id']+'-',dir=parent))
    return directory/'command.log'

def failure_detail(error,phase,path):
    return dict(phase=phase,error_type=type(error).__name__,errno=getattr(error,'errno',None),
                message=str(error)[:1000],path=str(path))


def run(row,command):
    valid(row)
    expected=row.get('execution_sha256',row['command_sha256'])
    if digest(command.encode())!=expected:raise ValueError('job command mismatch')
    root=pathlib.Path.home()/row['output_root'];records=ledger(row)
    if root.exists() or marker(row).exists() or records.exists():raise ValueError('job output or marker already exists')
    records.mkdir(parents=True,mode=0o700)
    raw_path=diagnostic_file(row)
    start=dict(identity(row),boot_id=boot_id(),started_ns=time.time_ns(),owner=proc(os.getpid()),
               native_diagnostics={'path':str(raw_path),'device':raw_path.parent.stat().st_dev})
    if start['owner'] is None:raise ValueError('owner process identity unavailable')
    # Reserve bounded space before any child starts. Never remove source/raw
    # artifacts to make room for an exit witness; the reserve is job-private.
    reserve=records/'terminal.reserve'
    with reserve.open('xb') as space:
        os.posix_fallocate(space.fileno(),0,131072)
        space.flush();os.fsync(space.fileno())
    save(records/'start.manifest.json',start)
    child=None;received=[];previous={};exit_saved=False
    def interrupted(signum,frame):
        received.append(signum)
        if child is not None:
            try:os.killpg(child.pid,signum)
            except ProcessLookupError:pass
    # Forward, do not ignore, SSH hangup and cancellation. The synchronous
    # supervisor waits for the actual child exit and writes its durable receipt.
    for sig in (signal.SIGHUP,signal.SIGTERM,signal.SIGINT):
        previous[sig]=signal.signal(sig,interrupted)
    try:
        raw_hash=hashlib.sha256();raw_bytes=0;observer_lost=False;diagnostic_error=None
        raw=raw_path.open('xb')
        def failed_diagnostic(error,phase):
            nonlocal diagnostic_error
            if diagnostic_error is None:
                diagnostic_error=failure_detail(error,phase,raw_path)
                # A failed diagnostic sink is visible, but its optional status
                # publication must not stop the owned foreground child either.
                try:save(records/'diagnostic-error.manifest.json',dict(start,diagnostic_error=diagnostic_error))
                except OSError as publish_error:
                    diagnostic_error['status_publish_error']=failure_detail(publish_error,'diagnostic-status',records)
        try:
            script=raw_path.parent/'command.sh';script.write_bytes(command.encode());script.chmod(0o600)
            child=subprocess.Popen(['bash',str(script)],process_group=0,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            witness=proc(child.pid)
            save(records/'child.manifest.json',dict(start,child=witness))
            if received:
                try:os.killpg(child.pid,received[-1])
                except ProcessLookupError:pass
            # Continue draining an owned child after diagnostic I/O failure.
            # Never substitute a successful gate or discard the failure: its
            # actual wait result survives, while compact fetch stays refused.
            while chunk:=child.stdout.read1(65536):
                if diagnostic_error is None:
                    try:
                        written=raw.write(chunk)
                        if written!=len(chunk):raise OSError(5,'short diagnostic write')
                        raw.flush();raw_hash.update(chunk);raw_bytes+=len(chunk)
                    except OSError as error:failed_diagnostic(error,'diagnostic-stream')
                if not observer_lost:
                    try:sys.stdout.buffer.write(chunk);sys.stdout.buffer.flush()
                    except (BrokenPipeError,OSError):
                        observer_lost=True
                        with open(os.devnull,'wb') as null:os.dup2(null.fileno(),sys.stdout.fileno())
            child.stdout.close();code=child.wait()
            actual=code if code>=0 else 128-code
            exit_row=dict(start,command_returncode=code,actual_exit=actual,exited_ns=time.time_ns(),signals=received,
                          observer_pipe_closed=observer_lost)
            if diagnostic_error is None:
                exit_row['execution_raw']={'path':str(raw_path),'bytes':raw_bytes,'sha256':raw_hash.hexdigest()}
            else:
                exit_row['execution_raw_error']=diagnostic_error
                exit_row['execution_raw_partial']={'path':str(raw_path),'hash_not_claimed':True}
            reserve.unlink();sync_directory(records)
            save(records/'exit.manifest.json',exit_row);exit_saved=True
            try:raw.flush();os.fsync(raw.fileno())
            except OSError as error:failed_diagnostic(error,'diagnostic-sync')
        finally:
            try:raw.close()
            except OSError as error:failed_diagnostic(error,'diagnostic-close')
        try:
            if diagnostic_error is not None:raise ValueError('diagnostic failure: '+json.dumps(diagnostic_error,separators=(',',':')))
            files=catalog(row);runner_exit=actual;error=None
        except Exception as failure:
            files={};runner_exit=3;error=str(failure)[:2000]
        final=dict(exit_row,completed=True,runner_exit=runner_exit,artifacts=files,catalog_error=error,diagnostic_error=diagnostic_error)
        save(marker(row),final)
        return runner_exit
    finally:
        # An unexpected ownership/custody failure must not leave this foreground command
        # running without its supervisor. This is our current child, never a
        # discovered old/orphan process or a sandbox lifecycle operation.
        supervision_error=sys.exc_info()[1]
        if child is not None and child.poll() is None:
            try:os.killpg(child.pid,signal.SIGTERM)
            except ProcessLookupError:pass
            try:child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                try:os.killpg(child.pid,signal.SIGKILL)
                except ProcessLookupError:pass
                child.wait()
        # A failure while opening/draining diagnostics or recording the child
        # still cannot turn a genuine, synchronously waited exit into UNKNOWN.
        # SIGKILL of the supervisor has no such witness and remains UNKNOWN.
        reserve.unlink(missing_ok=True)
        if child is not None and child.returncode is not None and not exit_saved:
            code=child.returncode
            actual=code if code>=0 else 128-code
            try:
                save(records/'exit.manifest.json',dict(start,command_returncode=code,
                     actual_exit=actual,exited_ns=time.time_ns(),signals=received,
                     execution_raw_error='diagnostic/custody interrupted; native wait result preserved',
                     supervision_error=failure_detail(supervision_error,'supervisor-custody',records) if supervision_error is not None else None))
            except FileExistsError:pass # Immutable first witness already published.
        for sig,handler in previous.items():signal.signal(sig,handler)


def launch(row,command):
    """Stream this foreground launcher to bash -s; never place payload in argv."""
    valid(row)
    if digest(command.encode())!=row.get('execution_sha256',row['command_sha256']):raise ValueError('job command mismatch')
    code=base64.b64encode(pathlib.Path(__file__).read_bytes()).decode()
    body=base64.b64encode(command.encode()).decode()
    # Base64 cannot contain underscores, so these heredoc terminators cannot
    # collide with either payload. The temporary runner remains under HOME.
    return ('runner=$(mktemp "$HOME/.amazing-launch.XXXXXX") || exit 3\n'
            'trap \'rm -f "$runner"\' EXIT\n'
            'base64 -d > "$runner" <<\'AMAZING_RUNNER_END\'\n'+code+'\nAMAZING_RUNNER_END\n'
            'env -u PYTHONHOME -u PYTHONPATH -u PYTHONUSERBASE -u VIRTUAL_ENV /usr/bin/python3 -I "$runner" run-base64stdin '+shlex.quote(encoded(row))+" <<'AMAZING_EXECUTION_END'\n"+body+'\nAMAZING_EXECUTION_END\n'
            'rc=$?\nexit "$rc"\n')


def completion(row):
    p=marker(row)
    if row['schema']==1 and not p.exists():p=pathlib.Path('/tmp')/('amazing-completed-'+row['job_id']+'.json')
    return json.loads(p.read_text())
def check_identity(row,record):
    if any(record.get(k)!=v for k,v in identity(row).items()):raise ValueError('completed job identity mismatch')
    if record.get('boot_id')!=boot_id():raise ValueError('endpoint has rebooted or resumed')

def diagnostic_completion(record,source,job_id):
    """Admit hash-bound original completion facts without rewriting a receipt.

    Callers must authenticate the input bytes before this schema check. Native
    markers carry flattened endpoint identity and completed=True; observations
    carry nested job identity and state=TERMINAL. Never accept a hybrid, infer
    an exit from output, or make a fresh observation of an old job here.
    """
    if not isinstance(record,dict):raise ValueError('completion must be an object')
    if not re.fullmatch(r'[0-9a-f]{40}',str(source)) or not re.fullmatch(r'[0-9a-f]{32}',str(job_id)):
        raise ValueError('exact diagnostic source/job required')
    if 'completed' in record:
        if record['completed'] is not True or 'job' in record or 'state' in record:
            raise ValueError('original native completion marker required; no hybrid')
        valid(record)
        if not re.fullmatch(r'[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}',str(record.get('boot_id',''))):
            raise ValueError('native completion boot identity required')
        if not isinstance(record.get('artifacts'),dict):raise ValueError('native completion catalog required')
        endpoint=identity(record);form='native-completion-marker'
    else:
        if record.get('state')!='TERMINAL' or not isinstance(record.get('job'),dict):
            raise ValueError('original terminal observation required')
        endpoint=record['job'];form='public-observation-summary'
        # Preserve the established legacy observation diagnostic contract; a
        # versioned public endpoint must satisfy its complete identity schema.
        if 'schema' in endpoint:valid(endpoint)
    actual=record.get('actual_exit');runner=record.get('runner_exit');returned=record.get('command_returncode')
    if (type(actual) is not int or actual==0 or type(runner) is not int
            or type(returned) is not int or runner!=actual
            or actual!=(returned if returned>=0 else 128-returned)
            or endpoint.get('source_commit')!=source or endpoint.get('job_id')!=job_id):
        raise ValueError('original authenticated native failure/exit/source/job required')
    return endpoint,actual,form

def observe(row):
    valid(row);result={'job':identity(row),'state':'UNKNOWN','actual_exit':None,'runner_exit':None,'boot_id':boot_id(),'reason':'no authenticated start or completion record'}
    try:
        final=completion(row);check_identity(row,final)
        if final.get('completed') is not True or type(final.get('actual_exit')) is not int:raise ValueError('job has not completed')
        result.update(state='TERMINAL',actual_exit=final['actual_exit'],runner_exit=final.get('runner_exit',final['actual_exit']),reason='authenticated actual command exit',owner=final.get('owner'),started_ns=final.get('started_ns'),catalog_error=final.get('catalog_error'),execution_raw=final.get('execution_raw'),observer_pipe_closed=final.get('observer_pipe_closed',False),signals=final.get('signals',[]),command_returncode=final.get('command_returncode'),exited_ns=final.get('exited_ns'),diagnostic_error=final.get('diagnostic_error'),native_diagnostics=final.get('native_diagnostics'))
        return result
    except FileNotFoundError:pass
    except (ValueError,KeyError,TypeError) as error:
        result['reason']=str(error);return result
    try:
        start=json.loads((ledger(row)/'start.manifest.json').read_text());check_identity(row,start)
        witness=start['owner'];current=proc(witness['pid'])
        result.update(owner=witness,started_ns=start['started_ns'],owner_alive=False,native_diagnostics=start.get('native_diagnostics'))
        ended=None
        try:
            ended=json.loads((ledger(row)/'exit.manifest.json').read_text());check_identity(row,ended)
            if type(ended.get('actual_exit')) is not int:raise ValueError('invalid native exit receipt')
            result['native_actual_exit']=ended['actual_exit']
            result.update(signals=ended.get('signals',[]),command_returncode=ended.get('command_returncode'),exited_ns=ended.get('exited_ns'),execution_raw_error=ended.get('execution_raw_error'),supervision_error=ended.get('supervision_error'))
        except FileNotFoundError:pass
        if current is None:
            if ended is not None:
                result.update(state='TERMINAL',actual_exit=ended['actual_exit'],reason='authenticated native exit; supervisor absent before final catalog',runner_exit=None)
            else:result['reason']='owned supervisor is absent; native exit not recorded'
            return result
        if any(current.get(k)!=witness.get(k) for k in ('pid','start_ticks','cmdline_sha256')):
            result['reason']='process identity changed; PID may have been reused';return result
        if current['state'] in ('Z','X'):
            if ended is not None:
                result.update(state='TERMINAL',actual_exit=ended['actual_exit'],runner_exit=None,reason='authenticated native exit; supervisor exited before final catalog')
            else:result['reason']='owned supervisor exited without completion evidence'
            return result
        try:
            child_record=json.loads((ledger(row)/'child.manifest.json').read_text());check_identity(row,child_record)
            if child_record.get('owner')!=witness or child_record.get('started_ns')!=start['started_ns']:raise ValueError('child start identity mismatch')
            child_witness=child_record.get('child');result['child']=child_witness
            if child_witness is not None:
                child_now=proc(child_witness['pid'])
                result['child_alive']=bool(child_now and child_now['start_ticks']==child_witness['start_ticks'] and child_now['state'] not in ('Z','X') and child_now['parent_pid']==witness['pid'])
        except FileNotFoundError:pass
        try:
            failure=json.loads((ledger(row)/'diagnostic-error.manifest.json').read_text());check_identity(row,failure)
            result['diagnostic_error']=failure['diagnostic_error']
        except FileNotFoundError:pass
        result.update(state='RUNNING',owner_alive=True,reason='authenticated live supervisor process')
        if ended is not None:result['reason']='native child exited; live supervisor completing receipt'
        return result
    except (FileNotFoundError,ValueError,KeyError,TypeError,PermissionError) as error:
        result['reason']='ownership observation unavailable: '+str(error)[:1000];return result

def verify(row,lane,source,paths):
    valid(row)
    if row['lane']!=lane or row['source_commit']!=source:raise ValueError('endpoint source/lane mismatch')
    final=completion(row);check_identity(row,final)
    if final.get('completed') is not True or type(final.get('actual_exit')) is not int:raise ValueError('job has not completed')
    if final.get('catalog_error'):raise ValueError('compact catalog failed: '+final['catalog_error'])
    result={};total=0
    for name in paths:
        p=pathlib.PurePosixPath(name)
        if p.is_absolute() or '..' in p.parts or not COMPACT.fullmatch(p.name) or not p.is_relative_to(row['output_root']):raise ValueError('noncompact or outside job path')
        expected=final['artifacts'][name];file=pathlib.Path.home()/name
        if file.is_symlink() or not file.resolve().is_relative_to((pathlib.Path.home()/row['output_root']).resolve()):raise ValueError('nonlocal compact operand')
        data=file.read_bytes();total+=len(data)
        if len(data)>524288 or total>2097152 or expected!={'bytes':len(data),'sha256':digest(data)}:raise ValueError('compact artifact changed')
        result[name]=dict(expected,payload=base64.b64encode(data).decode())
    return dict(job=identity(row),boot_id=final['boot_id'],actual_exit=final['actual_exit'],artifacts=result)

def export_baseline(row, summary_name):
    # Metadata-only authenticated remote reference. Never open or move ELF blobs.
    fetched = verify(row, row['lane'], row['source_commit'], [summary_name])
    if fetched['actual_exit'] != 0:
        raise ValueError('remote baseline command must have genuine exit zero')
    final = completion(row)
    summary_bytes = base64.b64decode(fetched['artifacts'][summary_name]['payload'], validate=True)
    summary = json.loads(summary_bytes)
    if summary.get('state') != 'TERMINAL' or any(type(summary.get(k)) is not int or summary[k] != 0 for k in ('command_exit','storage_exit')):
        raise ValueError('remote baseline storage must be terminal successful')
    if summary.get('source') != row['source_commit'] or summary.get('phase') != 'before':
        raise ValueError('remote baseline storage source/phase mismatch')
    descriptor = summary['artifact_manifest']
    manifest = pathlib.Path(descriptor['path'])
    if not manifest.is_absolute() or manifest.resolve(strict=True) != manifest or not manifest.is_file():
        raise ValueError('remote retention manifest must be a canonical regular file')
    data = manifest.read_bytes()
    if len(data) > 2097152 or descriptor != {'path':str(manifest),'bytes':len(data),'sha256':digest(data)}:
        raise ValueError('remote retention manifest bytes/hash mismatch')
    original = json.loads(data)
    if original.get('source') != row['source_commit'] or original.get('phase') != 'before' or (type(original.get('command_exit')) is not int or original['command_exit'] != 0) or original.get('job') != summary['job'] or original.get('profile') != summary['profile']:
        raise ValueError('remote retention manifest source/job/profile mismatch')
    if not any(pathlib.PurePosixPath(r['path']) in pathlib.PurePosixPath(str(manifest)).parents and r['device']==manifest.stat().st_dev for r in summary['capacity']['filesystems']):
        raise ValueError('remote retained manifest device differs from storage admission')
    artifacts = original['artifacts']
    elf_count = sum(a['kind'] == 'elf' for a in artifacts)
    if not elf_count or len(artifacts) != summary['retained_files'] or elf_count != summary['retained_elf_files']:
        raise ValueError('remote retention manifest artifact counts mismatch')
    return {'schema':1,'mode':'remote-reference','job':row,'completed':final,
            'catalog_sha256':digest(json.dumps(final['artifacts'],sort_keys=True,separators=(',',':')).encode()),
            'storage_summary':dict(fetched['artifacts'][summary_name],path=summary_name),
            'retention_manifest':dict(descriptor,metadata_readback=True,device=manifest.stat().st_dev,
                source=original['source'],job=original['job'],phase=original['phase'],command_exit=original['command_exit'],
                profile=original['profile'],artifact_count=len(artifacts),elf_count=elf_count,target=original['target']),
            'local_elf_readback':False,'original_payload_retained_on_original_host':True}


def export_reference(args):
    row = receipt(args)
    reference = remote(row, 'export-baseline', [args.storage_summary], seconds=60)
    destination = pathlib.Path(args.destination)
    if destination.exists() or destination.is_symlink():
        raise ValueError('reference destination already exists')
    data = (json.dumps(reference,sort_keys=True,separators=(',',':'))+'\n').encode()
    if len(data) > 524288:
        raise ValueError('remote baseline reference exceeds compact limit')
    destination.parent.mkdir(parents=True,exist_ok=True)
    with destination.open('xb') as stream:
        stream.write(data);stream.flush();os.fsync(stream.fileno())
    print(json.dumps({'path':str(destination),'bytes':len(data),'sha256':digest(data),'job':identity(row),
                      'baseline_mode':'remote-reference','local_elf_readback':False},separators=(',',':')))
    return 0


def remote(row,action,tail=(),seconds=30):
    code=pathlib.Path(__file__).read_text()
    cmd='env -u PYTHONHOME -u PYTHONPATH -u PYTHONUSERBASE -u VIRTUAL_ENV /usr/bin/python3 -I -c '+shlex.quote(code)+' '+action+' '+shlex.quote(encoded(row))+' '+ ' '.join(shlex.quote(p) for p in tail)
    ssh=['ssh','-o','BatchMode=yes','-o','StrictHostKeyChecking=no','-o','UserKnownHostsFile=/dev/null','-o','LogLevel=ERROR','-o','ConnectTimeout=20','-i',str(pathlib.Path.home()/'.ssh/ascii_box_ed25519'),'-p',str(row['port']),row['host'],'timeout '+str(max(1,seconds-2))+' '+cmd]
    result=subprocess.run(ssh,capture_output=True,timeout=seconds)
    if result.returncode:raise RuntimeError('completed endpoint rejected transport: '+result.stderr.decode(errors='replace')[-1200:])
    if len(result.stdout)>3145728:raise ValueError('compact transport exceeds bound')
    return json.loads(result.stdout)

def receipt(args):
    row=valid(json.loads(pathlib.Path(args.receipt).read_text()))
    if row['lane']!=args.lane or row['source_commit']!=args.source:raise ValueError('endpoint source/lane mismatch')
    return row

def status(args,waiting=False):
    row=receipt(args);deadline=time.monotonic()+args.seconds if waiting else None
    while True:
        try:
            data=remote(row,'observe',seconds=min(30,max(1,int(deadline-time.monotonic()))) if waiting else 30)
            if data.get('job')!=identity(row) or data.get('state') not in ('UNKNOWN','RUNNING','TERMINAL'):raise ValueError('status identity mismatch')
        except (RuntimeError,subprocess.TimeoutExpired,ValueError,json.JSONDecodeError) as error:
            data={'job':identity(row),'state':'UNKNOWN','actual_exit':None,'runner_exit':None,'reason':str(error)[:1200]}
        if not waiting or data['state']!='RUNNING' or time.monotonic()>=deadline:
            if args.output:
                dest=pathlib.Path(args.output);dest.parent.mkdir(parents=True,exist_ok=True);dest.write_text(json.dumps(data,separators=(',',':'))+'\n')
            print(json.dumps(data,separators=(',',':')))
            if not waiting:return 0
            return data['runner_exit'] if data['state']=='TERMINAL' and type(data.get('runner_exit')) is int else 3
        time.sleep(min(2,max(0,deadline-time.monotonic())))

def fetch(args):
    row=receipt(args);data=remote(row,'verify',(args.lane,args.source,*args.paths),seconds=90)
    if data['job']!=identity(row) or set(data['artifacts'])!=set(args.paths):raise ValueError('transport identity/path mismatch')
    dest=pathlib.Path(args.destination);dest.mkdir(parents=True,exist_ok=True)
    names=[pathlib.Path(p).name for p in args.paths]
    if len(set(names))!=len(names):raise ValueError('duplicate compact names')
    total=0
    for name,item in data['artifacts'].items():
        total+=item['bytes']
        if not 0<=item['bytes']<=524288 or total>2097152:raise ValueError('compact transport size exceeds bound')
        content=base64.b64decode(item['payload'],validate=True)
        if len(content)!=item['bytes'] or digest(content)!=item['sha256']:raise ValueError('transport checksum mismatch')
        p=dest/pathlib.Path(name).name
        if p.exists():
            if p.read_bytes()!=content:raise ValueError('existing compact file differs')
        else:
            with p.open('xb') as f:f.write(content)
    print(json.dumps({'job_id':row['job_id'],'source_commit':row['source_commit'],'actual_exit':data['actual_exit'],'files':{p:{k:v[k] for k in ('bytes','sha256')} for p,v in data['artifacts'].items()}},separators=(',',':')))
    return 0

def main():
    p=argparse.ArgumentParser();subs=p.add_subparsers(dest='action',required=True)
    a=subs.add_parser('validate-request');a.add_argument('--lane',required=True);a.add_argument('--root',required=True)
    a=subs.add_parser('capture')
    for name in ('receipt','lane','source','box','host','root'):a.add_argument('--'+name,required=True)
    g=a.add_mutually_exclusive_group(required=True);g.add_argument('--command');g.add_argument('--command-file')
    a.add_argument('--captured-proof-root',choices=('/srv/amazing-proof',));a.add_argument('--port',type=int,required=True);g=a.add_mutually_exclusive_group();g.add_argument('--execution');g.add_argument('--execution-file');a.add_argument('--source-archive-sha256')
    a=subs.add_parser('run');a.add_argument('receipt');a.add_argument('command')
    a=subs.add_parser('run-base64stdin');a.add_argument('receipt')
    a=subs.add_parser('launch');a.add_argument('receipt');a.add_argument('--execution-file',required=True)
    a=subs.add_parser('observe');a.add_argument('receipt')
    a=subs.add_parser('verify');a.add_argument('receipt');a.add_argument('lane');a.add_argument('source');a.add_argument('paths',nargs='+')
    for name in ('status','wait'):
        a=subs.add_parser(name)
        for key in ('receipt','lane','source'):a.add_argument('--'+key,required=True)
        a.add_argument('--output')
        if name=='wait':a.add_argument('--seconds',type=int,default=60)
    a=subs.add_parser('export-baseline');a.add_argument('receipt');a.add_argument('storage_summary')
    a=subs.add_parser('export-reference')
    for name in ('receipt','lane','source','storage-summary','destination'):a.add_argument('--'+name,required=True)
    a=subs.add_parser('fetch')
    for name in ('receipt','lane','source','destination'):a.add_argument('--'+name,required=True)
    a.add_argument('paths',nargs='+');args=p.parse_args()
    if args.action=='validate-request':valid_output_root(args.lane,args.root);return 0
    if args.action=='capture':capture(args);return 0
    if args.action=='run':return run(decode(args.receipt),args.command)
    if args.action=='run-base64stdin':return run(decode(args.receipt),base64.b64decode(sys.stdin.buffer.read().strip(),validate=True).decode())
    if args.action=='launch':print(launch(decode(args.receipt),pathlib.Path(args.execution_file).read_text()),end='');return 0
    if args.action=='observe':print(json.dumps(observe(decode(args.receipt))));return 0
    if args.action=='export-reference':return export_reference(args)
    if args.action=='export-baseline':print(json.dumps(export_baseline(decode(args.receipt),args.storage_summary)));return 0
    if args.action=='verify':print(json.dumps(verify(decode(args.receipt),args.lane,args.source,args.paths)));return 0
    if args.action in ('status','wait'):
        if args.action=='wait' and not 1<=args.seconds<=7200:raise ValueError('wait bound must be 1..7200 seconds')
        return status(args,args.action=='wait')
    return fetch(args)
if __name__=='__main__':
    try:raise SystemExit(main())
    except Exception as error:print('completed-job: '+str(error),file=sys.stderr);raise SystemExit(3)
