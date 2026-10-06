#!/usr/bin/env python3
"""Whole native captured-root admission and real Bash3.2 prelaunch controls."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shlex
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import patch

p=argparse.ArgumentParser();p.add_argument('--source',required=True);p.add_argument('--bash32',required=True);p.add_argument('--output',required=True);OPTIONS=p.parse_args();S=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('roots',S/'native-proof-root.py');r=importlib.util.module_from_spec(spec);spec.loader.exec_module(r)
class Controls(unittest.TestCase):
 def fixture(self):
  parent=Path(tempfile.mkdtemp(prefix='root-admission.',dir='/tmp'));root=parent/'root';root.mkdir(mode=0o700);return parent,root
 def test_actual_production_native_root_existing_is_readonly_and_ownerprivate(self):
  before=r.ROOT.stat();row=r.ensure(True);after=r.ROOT.stat();self.assertFalse(row['created']);self.assertEqual(row['mode'],0o700);self.assertEqual(before.st_ino,after.st_ino);self.assertEqual(before.st_ctime_ns,after.st_ctime_ns);self.assertEqual(row['mount']['stat_device'],after.st_dev)
 def test_actual_native_private_fixture_and_missing_refusal_create_no_ledger(self):
  parent,root=self.fixture()
  with patch.object(r,'ROOT',root):self.assertFalse(r.ensure(False)['created'])
  with patch.object(r,'ROOT',parent/'missing'):
   with self.assertRaises(FileNotFoundError):r.ensure(False)
  self.assertEqual(set(x.name for x in parent.iterdir()),{'root'})
 def test_actual_symlink_foreign_mode_file_and_permissions_are_never_repaired(self):
  parent,root=self.fixture();link=parent/'link';link.symlink_to(root);file=parent/'file';file.write_text('unchanged')
  for path in (link,file):
   with patch.object(r,'ROOT',path),self.assertRaises(ValueError):r.ensure(True)
  root.chmod(0o755);before=root.stat()
  with patch.object(r,'ROOT',root),self.assertRaises(ValueError):r.ensure(True)
  self.assertEqual(root.stat().st_ctime_ns,before.st_ctime_ns);self.assertTrue(link.is_symlink());self.assertEqual(file.read_text(),'unchanged')
 def test_foreign_uid_native_root_is_rejected_without_provision(self):
  parent,root=self.fixture();actual=os.getuid
  with patch.object(r,'ROOT',root),patch.object(r.os,'getuid',return_value=actual()+1),self.assertRaises(ValueError):r.ensure(True)
  self.assertEqual(root.stat().st_uid,actual())
 def test_actual_exclusion_files_and_dangling_policy_links_reject(self):
  for kind in ('file','link'):
   parent,root=self.fixture();policy=parent/'.boxignore'
   if kind=='file':policy.write_text('root\n')
   else:policy.symlink_to(parent/'missing')
   with patch.object(r,'ROOT',root),self.assertRaisesRegex(ValueError,'exclusion'):r.ensure(True)
 def test_fuse_tmpfs_and_unknown_mounts_cannot_reach_provision_or_capture(self):
  parent,root=self.fixture()
  for filesystem in ('fuse','fuse.lazy','tmpfs','ramfs','unknown'):
   line='7 2 0:1 / / rw - '+filesystem+' none rw\n';real=Path.read_text
   def read(path,*a,**k):return line if str(path)=='/proc/self/mountinfo' else real(path,*a,**k)
   with patch.object(r,'ROOT',root),patch.object(Path,'read_text',read),self.assertRaisesRegex(ValueError,'native'):r.ensure(True)
 def test_original_default_tmp_and_capture_argv_remain_both_modes_actual_bash32(self):
  result=subprocess.run([OPTIONS.bash32,'--version'],capture_output=True,text=True,timeout=10);self.assertEqual(result.returncode,0);self.assertIn('version 3.2.',result.stdout)
  text=(S/'linux-run.sh').read_text();self.assertIn('PROOF_CAPTURE_ARGS=(capture)',text)
  self.assertNotIn('native-proof-root.py',text[text.index('PROOF_CAPTURE_ARGS=(capture)'):text.index('capture_rc=$?',text.index('PROOF_CAPTURE_ARGS=(capture)'))])
 def test_bad_ui_paths_and_missing_capture_options_reject_before_owning_or_shipping(self):
  for path in ('','/tmp','/srv','relative','/srv/amazing-proof/','/srv/amazing-proof/child','$(false)','/srv/amazing-proof'):
   base=Path(tempfile.mkdtemp(prefix='root-caller.',dir='/tmp'));scripts=base/'scripts';scripts.mkdir();(scripts/'linux-run.sh').write_bytes((S/'linux-run.sh').read_bytes());tools=base/'tools';tools.mkdir();git=tools/'git';git.write_text('#!/bin/sh\nprintf "%s\\n" '+shlex.quote(str(base/'.git'))+'\n');git.chmod(0o755);sentinel=base/'allocation';boat=scripts/'boat-lane.py';boat.write_text('#!/bin/sh\ntouch '+shlex.quote(str(sentinel))+'\nexit 99\n');boat.chmod(0o755)
   args=[OPTIONS.bash32,str(scripts/'linux-run.sh'),'native-root-control','--host','boat','--ref','1'*40,'--captured-proof-root',path,'--','exit 0'];result=subprocess.run(args,capture_output=True,text=True,timeout=10,env={**os.environ,'PATH':str(tools)+':'+os.environ['PATH']});self.assertEqual(result.returncode,2,result.stderr);self.assertFalse(sentinel.exists());self.assertFalse((base/'.agents').exists());self.assertFalse((base/'endpoint').exists())
  result=subprocess.run([OPTIONS.bash32,str(scripts/'linux-run.sh'),'native-root-control','--host','boat','--captured-proof-root'],capture_output=True,text=True,timeout=10,env={**os.environ,'PATH':str(tools)+':'+os.environ['PATH']});self.assertEqual(result.returncode,2);self.assertIn('needs a value',result.stderr);self.assertNotIn('unbound variable',result.stderr);self.assertFalse(sentinel.exists())
 def test_native_preflight_is_before_mission_source_upload_lease_and_supervisor(self):
  text=(S/'linux-run.sh').read_text();position=text.index('native-proof-root.py" preflight');self.assertLess(position,text.index('scp -O',position));self.assertLess(position,text.index('source-upload.py" --receipt "$SOURCE_TRANSFER_RECEIPT" send'));self.assertLess(position,text.index('completed-job.py" "${PROOF_CAPTURE_ARGS[@]}"'));self.assertLess(position,text.index('touch $LEASE',position))
 def test_provision_failure_is_durable_no_supervisor_no_source_transfer(self):
  class Args:root='/srv/amazing-proof';source=OPTIONS.source;host='user@127.0.0.1';port=19001;box='bx_fixture'
  parent,_=self.fixture();args=Args();args.receipt=str(parent/'failure.summary.json')
  reply=json.dumps({'schema':1,'phase':'captured-native-root','actual_exit':3,'error':{'message':'foreign or absent captured root'}}).encode()
  with patch.object(r.subprocess,'run',return_value=subprocess.CompletedProcess([],3,reply,b'actual refusal')):self.assertEqual(r.preflight(args),3)
  result=json.loads(Path(args.receipt).read_text());self.assertFalse(result['supervisor_created']);self.assertFalse(result['source_uploaded']);self.assertEqual(result['transport_exit'],3)
 def test_transport_failure_and_malformed_identity_never_claim_native_admission(self):
  class Args:root='/srv/amazing-proof';source=OPTIONS.source;host='user@127.0.0.1';port=19001;box='bx_fixture'
  for reply in (b'{}',b'not json',b'{"schema":1,"phase":"wrong","actual_exit":0}'):
   parent,_=self.fixture();args=Args();args.receipt=str(parent/'failure.summary.json')
   with patch.object(r.subprocess,'run',return_value=subprocess.CompletedProcess([],0,reply,b'')):self.assertEqual(r.preflight(args),3)
   self.assertEqual(json.loads(Path(args.receipt).read_text())['actual_exit'],3)
 def test_root_provision_source_is_exact_newonly_and_no_foreign_chmod_delete_or_lifecycle(self):
  text=(S/'native-proof-root.py').read_text();self.assertIn("p='/srv/amazing-proof';parent='/srv'",text);self.assertIn('except FileExistsError:',text);self.assertIn('os.O_NOFOLLOW',text);self.assertNotIn('os.chmod(',text);self.assertNotIn('shutil.rmtree(',text);self.assertNotIn('boat ',text)
class Results(unittest.TextTestResult):
 def __init__(self,*a,**k):super().__init__(*a,**k);self.classifications={}
 def addSuccess(self,t):super().addSuccess(t);self.classifications[t._testMethodName]='pass'
 def addFailure(self,t,e):super().addFailure(t,e);self.classifications[t._testMethodName]='fail'
 def addError(self,t,e):super().addError(t,e);self.classifications[t._testMethodName]='error'
result=unittest.TextTestRunner(verbosity=2,resultclass=Results).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls));row={'source':OPTIONS.source,'tests_run':result.testsRun,'failed':len(result.failures),'errors':len(result.errors),'actual_exit':int(not result.wasSuccessful()),'classifications':result.classifications,'diagnostics':{t.id():e[-4096:] for t,e in result.failures+result.errors}};Path(OPTIONS.output).write_text(json.dumps(row,separators=(',',':'))+'\n');print(json.dumps(row,separators=(',',':')));raise SystemExit(row['actual_exit'])
