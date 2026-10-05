#!/usr/bin/env python3
"""Authenticated foreground job ownership, actual exits and compact transport.

A lost SSH observation never proves a native command stopped. Schema 2 binds
its complete bootstrap execution as well as the requested gate command. HOME
holds immutable start/child/exit/completion records; /proc witnesses distinguish
an owned live process from a reused PID or another boot. No job is detached.
"""
import argparse,base64,hashlib,json,os,pathlib,re,shlex,signal,subprocess,sys,time,uuid

FIELDS=('schema','lane','source_commit','box_id','host','port','job_id','output_root','command_sha256')
COMPACT=re.compile(r'(?:[A-Za-z0-9_.-]+\.)?(?:compact|summary|manifest)\.json')
def digest(data):return hashlib.sha256(data).hexdigest()
def encoded(row):return base64.b64encode(json.dumps(row,separators=(',',':')).encode()).decode()
def decode(text):return json.loads(base64.b64decode(text))
def identity(row):return {k:row[k] for k in FIELDS+(('execution_sha256','source_archive_sha256') if row['schema']==2 else ())}
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
    valid_output_root(row['lane'],row['output_root'])
    return row

def capture(args):
    if getattr(args,'command_file',None):args.command=pathlib.Path(args.command_file).read_text()
    if getattr(args,'execution_file',None):args.execution=pathlib.Path(args.execution_file).read_text()
    row=dict(schema=1,lane=args.lane,source_commit=args.source,box_id=args.box,host=args.host,port=args.port,job_id=uuid.uuid4().hex,output_root=args.root,command_sha256=digest(args.command.encode()))
    if args.execution is not None:
        row.update(schema=2,execution_sha256=digest(args.execution.encode()),source_archive_sha256=args.source_archive_sha256)
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

def run(row,command):
    valid(row)
    expected=row.get('execution_sha256',row['command_sha256'])
    if digest(command.encode())!=expected:raise ValueError('job command mismatch')
    root=pathlib.Path.home()/row['output_root'];records=ledger(row)
    if root.exists() or marker(row).exists() or records.exists():raise ValueError('job output or marker already exists')
    records.mkdir(parents=True,mode=0o700)
    start=dict(identity(row),boot_id=boot_id(),started_ns=time.time_ns(),owner=proc(os.getpid()))
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
        raw_path=records/'command.log';raw_hash=hashlib.sha256();raw_bytes=0;observer_lost=False
        with raw_path.open('xb') as raw:
            script=records/'command.sh';script.write_bytes(command.encode());script.chmod(0o600)
            child=subprocess.Popen(['bash',str(script)],process_group=0,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            witness=proc(child.pid)
            save(records/'child.manifest.json',dict(start,child=witness))
            if received:
                try:os.killpg(child.pid,received[-1])
                except ProcessLookupError:pass
            # A closed observation pipe cannot erase native diagnostics or be
            # confused with the command's exit. We remain its foreground owner.
            # Hangup/cancellation still forwards to the child process group.
            while chunk:=child.stdout.read1(65536):
                raw.write(chunk);raw.flush();raw_hash.update(chunk);raw_bytes+=len(chunk)
                if not observer_lost:
                    try:sys.stdout.buffer.write(chunk);sys.stdout.buffer.flush()
                    except (BrokenPipeError,OSError):
                        observer_lost=True
                        # Prevent interpreter shutdown flushing the abandoned
                        # pipe and changing only the observer's process exit.
                        with open(os.devnull,'wb') as null:os.dup2(null.fileno(),sys.stdout.fileno())
            child.stdout.close();code=child.wait()
            actual=code if code>=0 else 128-code
            exit_row=dict(start,command_returncode=code,actual_exit=actual,exited_ns=time.time_ns(),signals=received,
                          observer_pipe_closed=observer_lost,execution_raw={'path':str(raw_path),'bytes':raw_bytes,'sha256':raw_hash.hexdigest()})
            reserve.unlink();sync_directory(records)
            # Witness the actual wait result before optional diagnostic-log
            # synchronization or compact catalog publication can fail.
            save(records/'exit.manifest.json',exit_row);exit_saved=True
            raw.flush();os.fsync(raw.fileno())
        try:
            files=catalog(row);runner_exit=actual;error=None
        except Exception as failure:
            files={};runner_exit=3;error=str(failure)[:1000]
        final=dict(exit_row,completed=True,runner_exit=runner_exit,artifacts=files,catalog_error=error)
        save(marker(row),final)
        return runner_exit
    finally:
        # Failure to publish custody must not leave this foreground command
        # running without its supervisor. This is our current child, never a
        # discovered old/orphan process or a sandbox lifecycle operation.
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
                     execution_raw_error='diagnostic/custody interrupted; native wait result preserved'))
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

def observe(row):
    valid(row);result={'job':identity(row),'state':'UNKNOWN','actual_exit':None,'runner_exit':None,'boot_id':boot_id(),'reason':'no authenticated start or completion record'}
    try:
        final=completion(row);check_identity(row,final)
        if final.get('completed') is not True or type(final.get('actual_exit')) is not int:raise ValueError('job has not completed')
        result.update(state='TERMINAL',actual_exit=final['actual_exit'],runner_exit=final.get('runner_exit',final['actual_exit']),reason='authenticated actual command exit',owner=final.get('owner'),started_ns=final.get('started_ns'),catalog_error=final.get('catalog_error'),execution_raw=final.get('execution_raw'),observer_pipe_closed=final.get('observer_pipe_closed',False))
        return result
    except FileNotFoundError:pass
    except (ValueError,KeyError,TypeError) as error:
        result['reason']=str(error);return result
    try:
        start=json.loads((ledger(row)/'start.manifest.json').read_text());check_identity(row,start)
        witness=start['owner'];current=proc(witness['pid'])
        result.update(owner=witness,started_ns=start['started_ns'],owner_alive=False)
        ended=None
        try:
            ended=json.loads((ledger(row)/'exit.manifest.json').read_text());check_identity(row,ended)
            if type(ended.get('actual_exit')) is not int:raise ValueError('invalid native exit receipt')
            result['native_actual_exit']=ended['actual_exit']
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
    a.add_argument('--port',type=int,required=True);g=a.add_mutually_exclusive_group();g.add_argument('--execution');g.add_argument('--execution-file');a.add_argument('--source-archive-sha256')
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
    a=subs.add_parser('fetch')
    for name in ('receipt','lane','source','destination'):a.add_argument('--'+name,required=True)
    a.add_argument('paths',nargs='+');args=p.parse_args()
    if args.action=='validate-request':valid_output_root(args.lane,args.root);return 0
    if args.action=='capture':capture(args);return 0
    if args.action=='run':return run(decode(args.receipt),args.command)
    if args.action=='run-base64stdin':return run(decode(args.receipt),base64.b64decode(sys.stdin.buffer.read().strip(),validate=True).decode())
    if args.action=='launch':print(launch(decode(args.receipt),pathlib.Path(args.execution_file).read_text()),end='');return 0
    if args.action=='observe':print(json.dumps(observe(decode(args.receipt))));return 0
    if args.action=='verify':print(json.dumps(verify(decode(args.receipt),args.lane,args.source,args.paths)));return 0
    if args.action in ('status','wait'):
        if args.action=='wait' and not 1<=args.seconds<=7200:raise ValueError('wait bound must be 1..7200 seconds')
        return status(args,args.action=='wait')
    return fetch(args)
if __name__=='__main__':
    try:raise SystemExit(main())
    except Exception as error:print('completed-job: '+str(error),file=sys.stderr);raise SystemExit(3)
