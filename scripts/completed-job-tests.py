#!/usr/bin/env python3
"""Public completed-job identity, stale endpoint and compact-only controls."""
import importlib.util,json,pathlib,tempfile,unittest,uuid
from unittest import mock
S=pathlib.Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('completed_job',S/'completed-job.py');job=importlib.util.module_from_spec(spec);spec.loader.exec_module(job)
class Completed(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup);self.home=pathlib.Path(self.tmp.name)
  self.home_patch=mock.patch.object(pathlib.Path,'home',return_value=self.home);self.home_patch.start();self.addCleanup(self.home_patch.stop)
  self.row={'schema':1,'lane':'css','source_commit':'a'*40,'box_id':'bx_fixture','host':'user@127.0.0.1','port':1234,'job_id':uuid.uuid4().hex,'output_root':'amazing/out/css/job/'+'a'*40,'command_sha256':job.digest(b'true')}
  self.path=self.row['output_root']+'/gate.summary.json';p=self.home/self.path;p.parent.mkdir(parents=True);p.write_text('{"source_commit":"'+'a'*40+'"}')
  self.final=dict(self.row,boot_id=pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),completed=True,actual_exit=100,artifacts={self.path:{'bytes':p.stat().st_size,'sha256':job.digest(p.read_bytes())}})
  self.save();self.addCleanup(lambda:job.marker(self.row).unlink(missing_ok=True))
 def save(self):job.marker(self.row).write_text(json.dumps(self.final))
 def verify(self,row=None,lane='css',source='a'*40,path=None):return job.verify(row or self.row,lane,source,[path or self.path])
 def test_completed_failed_engine_still_transfers_authentic_compact(self):
  self.assertEqual(self.verify()['actual_exit'],100)
 def test_all_endpoint_identity_fields_must_match(self):
  for key,value in [('box_id','bx_other'),('host','user@192.0.2.1'),('port',4321),('command_sha256','b'*64),('output_root','amazing/out/css/other/'+'a'*40)]:
   with self.subTest(key=key),self.assertRaises(ValueError):self.verify(dict(self.row,**{key:value}))
 def test_expected_source_and_lane_are_required(self):
  for lane,source in [('layout','a'*40),('css','b'*40)]:
   with self.assertRaises(ValueError):self.verify(lane=lane,source=source)
 def test_resumed_missing_or_different_boot_rejects(self):
  self.final['boot_id']='old-boot';self.save()
  with self.assertRaises(ValueError):self.verify()
  job.marker(self.row).unlink()
  with self.assertRaises(FileNotFoundError):self.verify()
 def test_incomplete_job_rejects(self):
  self.final['completed']=False;self.save()
  with self.assertRaises(ValueError):self.verify()
 def test_changed_artifact_rejects(self):
  (self.home/self.path).write_text('{}')
  with self.assertRaises(ValueError):self.verify()
 def test_raw_traversal_and_other_job_paths_reject(self):
  for path in [self.row['output_root']+'/raw.log','/tmp/gate.summary.json','amazing/out/css/../gate.summary.json','amazing/out/css/other/gate.summary.json']:
   with self.subTest(path=path),self.assertRaises(ValueError):self.verify(path=path)
 def test_stale_output_never_executes_command(self):
  with mock.patch.object(job.subprocess,'run') as run,self.assertRaises(ValueError):job.run(self.row,'true')
  run.assert_not_called()
 def test_symlink_operand_rejects(self):
  p=self.home/self.path;p.unlink();p.symlink_to(self.home/'outside')
  with self.assertRaises(ValueError):self.verify()
 def test_fresh_job_runs_and_captures_real_exit(self):
  import shlex
  other=dict(self.row,job_id=uuid.uuid4().hex,output_root='amazing/out/css/new/'+'a'*40)
  p=self.home/other['output_root']/'gate.summary.json'
  command='mkdir -p '+shlex.quote(str(p.parent))+'; printf \'{}\' > '+shlex.quote(str(p))+'; exit 7'
  other['command_sha256']=job.digest(command.encode())
  self.addCleanup(lambda:job.marker(other).unlink(missing_ok=True))
  self.assertEqual(job.run(other,command),7)
  self.assertEqual(job.verify(other,'css','a'*40,[str(p.relative_to(self.home))])['actual_exit'],7)
if __name__=='__main__':unittest.main()
