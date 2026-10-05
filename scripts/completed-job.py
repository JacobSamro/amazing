#!/usr/bin/env python3
"""Bind compact transport to one captured endpoint and completed remote job."""
import argparse,base64,hashlib,json,os,pathlib,re,shlex,subprocess,sys,uuid

FIELDS=('schema','lane','source_commit','box_id','host','port','job_id','output_root','command_sha256')
COMPACT=re.compile(r'(?:[A-Za-z0-9_.-]+\.)?(?:compact|summary|manifest)\.json')
def digest(data):return hashlib.sha256(data).hexdigest()
def encoded(row):return base64.b64encode(json.dumps(row,separators=(',',':')).encode()).decode()
def decode(text):return json.loads(base64.b64decode(text))
def marker(row):return pathlib.Path('/tmp')/('amazing-completed-'+row['job_id']+'.json')
def valid_output_root(lane, root):
    if not re.fullmatch(r'[A-Za-z0-9_-]+', lane):raise ValueError('invalid lane')
    p=pathlib.PurePosixPath(root)
    if p.is_absolute() or '..' in p.parts or not str(p).startswith('amazing/out/'+lane+'/'):raise ValueError('invalid job output root')
    if not re.fullmatch(r'[A-Za-z0-9_./-]+',str(p)):raise ValueError('invalid job output root')
    return root

def valid(row):
    if row.get('schema')!=1:raise ValueError('unknown endpoint receipt schema')
    for key,pattern in [('lane',r'[A-Za-z0-9_-]+'),('source_commit',r'[0-9a-f]{40}'),('box_id',r'bx_[A-Za-z0-9]+'),('host',r'[A-Za-z0-9_.-]+@[A-Za-z0-9:.]+'),('job_id',r'[0-9a-f]{32}'),('command_sha256',r'[0-9a-f]{64}')]:
        if not re.fullmatch(pattern,row[key]):raise ValueError('invalid '+key)
    if not isinstance(row['port'],int) or not 1<=row['port']<=65535:raise ValueError('invalid port')
    valid_output_root(row['lane'],row['output_root'])
    return row

def capture(args):
    row=valid(dict(schema=1,lane=args.lane,source_commit=args.source,box_id=args.box,host=args.host,port=args.port,job_id=uuid.uuid4().hex,output_root=args.root,command_sha256=digest(args.command.encode())))
    p=pathlib.Path(args.receipt);p.parent.mkdir(parents=True,exist_ok=True)
    with p.open('x') as f:json.dump(row,f,separators=(',',':'));f.write('\n')
    print(encoded(row))

def run(row,command):
    valid(row)
    if digest(command.encode())!=row['command_sha256']:raise ValueError('job command mismatch')
    root=pathlib.Path.home()/row['output_root']
    if root.exists() or marker(row).exists():raise ValueError('job output or marker already exists')
    boot=pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    code=subprocess.run(['bash','-c',command]).returncode
    files={};total=0
    if root.exists():
        if root.is_symlink():raise ValueError('symlink job root')
        for p in sorted(root.rglob('*.json')):
            if not COMPACT.fullmatch(p.name):continue
            if p.is_symlink() or not p.resolve().is_relative_to(root.resolve()):raise ValueError('nonlocal compact operand')
            data=p.read_bytes();total+=len(data)
            if len(data)>524288 or total>2097152:raise ValueError('compact evidence limit exceeded')
            files[str(p.relative_to(pathlib.Path.home()))]={'bytes':len(data),'sha256':digest(data)}
    final=dict(row,boot_id=boot,completed=True,actual_exit=code,artifacts=files)
    with marker(row).open('x') as f:json.dump(final,f,separators=(',',':'));f.write('\n')
    return code if code>=0 else 128-code

def verify(row,lane,source,paths):
    valid(row)
    if row['lane']!=lane or row['source_commit']!=source:raise ValueError('endpoint source/lane mismatch')
    final=json.loads(marker(row).read_text())
    if any(final.get(k)!=row[k] for k in FIELDS):raise ValueError('completed job identity mismatch')
    if final.get('completed') is not True or not isinstance(final.get('actual_exit'),int):raise ValueError('job has not completed')
    if final['boot_id']!=pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip():raise ValueError('endpoint has rebooted or resumed')
    result={};total=0
    for name in paths:
        p=pathlib.PurePosixPath(name)
        if p.is_absolute() or '..' in p.parts or not COMPACT.fullmatch(p.name) or not p.is_relative_to(row['output_root']):raise ValueError('noncompact or outside job path')
        expected=final['artifacts'][name];file=pathlib.Path.home()/name
        if file.is_symlink() or not file.resolve().is_relative_to((pathlib.Path.home()/row['output_root']).resolve()):raise ValueError('nonlocal compact operand')
        data=file.read_bytes();total+=len(data)
        if len(data)>524288 or total>2097152 or expected!={'bytes':len(data),'sha256':digest(data)}:raise ValueError('compact artifact changed')
        result[name]=dict(expected,payload=base64.b64encode(data).decode())
    return dict(job={k:final[k] for k in FIELDS},boot_id=final['boot_id'],actual_exit=final['actual_exit'],artifacts=result)

def fetch(args):
    row=valid(json.loads(pathlib.Path(args.receipt).read_text()))
    if row['lane']!=args.lane or row['source_commit']!=args.source:raise ValueError('endpoint source/lane mismatch')
    # Use current reviewed verifier bytes, not a remote mutable script. SSH
    # authenticates access, while the random completed marker and boot identity
    # distinguish this job from stale or resumed endpoint metadata.
    code=pathlib.Path(__file__).read_text()
    cmd='python3 -c '+shlex.quote(code)+' verify '+shlex.quote(encoded(row))+' '+shlex.quote(args.lane)+' '+shlex.quote(args.source)+' '+ ' '.join(shlex.quote(p) for p in args.paths)
    ssh=['ssh','-o','BatchMode=yes','-o','StrictHostKeyChecking=no','-o','UserKnownHostsFile=/dev/null','-o','LogLevel=ERROR','-o','ConnectTimeout=20','-i',str(pathlib.Path.home()/'.ssh/ascii_box_ed25519'),'-p',str(row['port']),row['host'],'timeout 60 '+cmd]
    result=subprocess.run(ssh,capture_output=True,timeout=90)
    if result.returncode:raise RuntimeError('completed endpoint rejected transport: '+result.stderr.decode(errors='replace')[-1200:])
    if len(result.stdout)>3145728:raise ValueError('compact transport exceeds bound')
    data=json.loads(result.stdout)
    if data['job']!={k:row[k] for k in FIELDS} or set(data['artifacts'])!=set(args.paths):raise ValueError('transport identity/path mismatch')
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
    print(json.dumps({'job_id':row['job_id'],'source_commit':row['source_commit'],'files':{p:{k:v[k] for k in ('bytes','sha256')} for p,v in data['artifacts'].items()}},separators=(',',':')))
    return 0

def main():
    p=argparse.ArgumentParser();subs=p.add_subparsers(dest='action',required=True)
    a=subs.add_parser('validate-request');a.add_argument('--lane',required=True);a.add_argument('--root',required=True)
    a=subs.add_parser('capture')
    for name in ('receipt','lane','source','box','host','root','command'):a.add_argument('--'+name,required=True)
    a.add_argument('--port',type=int,required=True)
    a=subs.add_parser('run');a.add_argument('receipt');a.add_argument('command')
    a=subs.add_parser('verify');a.add_argument('receipt');a.add_argument('lane');a.add_argument('source');a.add_argument('paths',nargs='+')
    a=subs.add_parser('fetch')
    for name in ('receipt','lane','source','destination'):a.add_argument('--'+name,required=True)
    a.add_argument('paths',nargs='+')
    args=p.parse_args()
    if args.action=='validate-request':valid_output_root(args.lane,args.root);return 0
    if args.action=='capture':capture(args);return 0
    if args.action=='run':return run(decode(args.receipt),args.command)
    if args.action=='verify':print(json.dumps(verify(decode(args.receipt),args.lane,args.source,args.paths)));return 0
    return fetch(args)
if __name__=='__main__':
    try:raise SystemExit(main())
    except Exception as error:print('completed-job: '+str(error),file=sys.stderr);raise SystemExit(3)
