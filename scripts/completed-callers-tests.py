#!/usr/bin/env python3
"""Exercise real workspace and 16-shard callers with metadata-only transports."""
import json,os,pathlib,shutil,subprocess,tempfile,unittest
S=pathlib.Path(__file__).resolve().parent
SOURCE='a'*40
class Callers(unittest.TestCase):
 def fixture(self):
  tmp=tempfile.TemporaryDirectory();self.addCleanup(tmp.cleanup);root=pathlib.Path(tmp.name);scripts=root/'scripts';scripts.mkdir();tools=root/'tools';tools.mkdir();(root/'.git').mkdir()
  for name in ('linux-gate.sh','test262-boat.sh','gate-summary.py','gate-command.py','test262.py'):shutil.copyfile(S/name,scripts/name)
  (scripts/'merge-preflight.py').write_text('pass\n')
  (scripts/'test262-merge.py').write_text('import pathlib,sys;pathlib.Path(sys.argv[1]).write_text("{}")\n')
  git=tools/'git';git.write_text('#!/bin/bash\nif [[ "$*" = *--git-common-dir* ]];then echo "$FIXTURE/.git";else echo '+SOURCE+';fi\n');git.chmod(0o755)
  runner=scripts/'linux-run.sh';runner.write_text('''#!/usr/bin/env python3
import sys,json,pathlib,os
args=sys.argv[1:];lane=args[0];source=args[args.index('--ref')+1];receipt=args[args.index('--endpoint-receipt')+1];root=args[args.index('--compact-root')+1]
row={'lane':lane,'source':source,'receipt':receipt,'root':root,'command':args[-1]}
p=pathlib.Path(os.environ['FIXTURE'])/(lane+'.call.json');p.write_text(json.dumps(row));pathlib.Path(receipt).parent.mkdir(parents=True,exist_ok=True);pathlib.Path(receipt).write_text(json.dumps(row))
raise SystemExit(100 if os.environ.get('FAIL_LANE')==lane else 0)
''');runner.chmod(0o755)
  fetch=scripts/'fetch-fixture.py';fetch.write_text('''#!/usr/bin/env python3
import sys,pathlib,json,re,os
args=sys.argv[1:];lane,dest=args[:2];assert args[2]=='--endpoint-receipt' and args[4]=='--source';r=json.loads(pathlib.Path(args[3]).read_text());assert r['lane']==lane and r['source']==args[5] and args[6].startswith(r['root']+'/')
p=pathlib.Path(dest);p.mkdir(parents=True,exist_ok=True);name=pathlib.Path(args[6]).name
row={}
if name.endswith('.compact.json'):
 k=int(lane.rsplit('-',1)[1]);row={'shard':str(k)+'/16','job_exit':0,'compact_records':'fixture','host_sha256':'fixture-host','input_sha256':'fixture-input'}
(p/name).write_text(json.dumps(row))
(pathlib.Path(os.environ['FIXTURE'])/(lane+'.fetch.json')).write_text(json.dumps({'lane':lane,'source':args[5],'receipt':args[3],'remote':args[6]}))
''');fetch.chmod(0o755)
  shell=scripts/'lane-fetch.sh';shell.write_text('#!/bin/bash\nexec python3 \"$(dirname \"$0\")/fetch-fixture.py\" \"$@\"\n');shell.chmod(0o755)
  env=dict(os.environ,FIXTURE=str(root),PATH=str(tools)+':'+os.environ['PATH'])
  return root,env
 def test_workspace_exact_completed_endpoint_and_both_native_phases(self):
  root,env=self.fixture();r=subprocess.run(['bash',str(root/'scripts/linux-gate.sh'),SOURCE],env=env,capture_output=True,text=True)
  self.assertEqual(r.returncode,0,r.stderr);row=json.loads((root/'integrator-primary.call.json').read_text());fetch=json.loads((root/'integrator-primary.fetch.json').read_text())
  self.assertEqual(row['source'],SOURCE);self.assertEqual(fetch['receipt'],row['receipt']);self.assertIn(SOURCE,row['root']);self.assertIn('cargo nextest run --workspace --locked',row['command']);self.assertIn('cargo test --workspace --locked --doc',row['command']);self.assertEqual(row['command'].count('--phase-receipt'),2)
  self.assertEqual(subprocess.run(['bash','-n'],input=row['command'],text=True,capture_output=True).returncode,0)
 def test_full16_each_capture_matches_source_and_three_phase_copy_guards(self):
  root,env=self.fixture();r=subprocess.run(['bash',str(root/'scripts/test262-boat.sh'),SOURCE,'fixture','--shards','16','--baseline','baseline.json'],env=env,capture_output=True,text=True)
  self.assertEqual(r.returncode,0,r.stderr);roots=set();receipts=set()
  for k in range(16):
   row=json.loads((root/('t262-shard-'+str(k)+'.call.json')).read_text());fetch=json.loads((root/('t262-shard-'+str(k)+'.fetch.json')).read_text());roots.add(row['root']);receipts.add(row['receipt'])
   self.assertEqual(fetch['receipt'],row['receipt']);self.assertEqual(fetch['source'],SOURCE);self.assertIn('--shard '+str(k)+'/16',row['command']);self.assertIn('output_copy_exit',row['command']);self.assertIn('receipt_copy_error',row['command']);self.assertEqual(row['command'].count('--phase build'),1);self.assertEqual(row['command'].count('--phase runner'),1);self.assertEqual(row['command'].count('--phase packaging'),1)
   self.assertEqual(subprocess.run(['bash','-n'],input=row['command'],text=True,capture_output=True).returncode,0)
  self.assertEqual(len(roots),16);self.assertEqual(len(receipts),16)
 def test_failed_shard_fetch_preserves_actual_job_failure_and_uses_capture(self):
  root,env=self.fixture();env['FAIL_LANE']='t262-shard-3';r=subprocess.run(['bash',str(root/'scripts/test262-boat.sh'),SOURCE,'failure','--shards','16'],env=env,capture_output=True,text=True)
  self.assertEqual(r.returncode,3);self.assertIn('shard 3 job exit 100',r.stderr);fetch=json.loads((root/'t262-shard-3.fetch.json').read_text());self.assertTrue(fetch['remote'].endswith('.failure.manifest.json'));self.assertEqual(fetch['source'],SOURCE)
if __name__=='__main__':unittest.main()
