#!/usr/bin/env python3
"""Native foreground ownership/exit tests; no simulated successful gate results."""
import base64,importlib.util,json,os,pathlib,shlex,signal,subprocess,sys,tempfile,time,unittest,uuid
from unittest import mock
S=pathlib.Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('completed_job',S/'completed-job.py');job=importlib.util.module_from_spec(spec);spec.loader.exec_module(job)

class Lifecycle(unittest.TestCase):
 def setUp(self):
  self.home=pathlib.Path.home()
  temporary=self.home/'amazing/out/public-job-lifecycle-control/temporary';temporary.mkdir(parents=True,exist_ok=True)
  self.tmp=tempfile.TemporaryDirectory(dir=str(temporary));self.addCleanup(self.tmp.cleanup);self.scratch=pathlib.Path(self.tmp.name)
  self.children=[];self.addCleanup(self.cleanup)
 def cleanup(self):
  for child in self.children:
   if child.poll() is None:child.send_signal(signal.SIGTERM)
   try:child.wait(timeout=15)
   except subprocess.TimeoutExpired:child.kill();child.wait()
 def row(self,command):
  return dict(schema=2,lane='public-job-lifecycle-control',source_commit=os.environ.get('COMMIT','a'*40),box_id='bx_ownedcontrol',host='user@127.0.0.1',port=19001,job_id=uuid.uuid4().hex,output_root='amazing/out/public-job-lifecycle-control/'+uuid.uuid4().hex,command_sha256=job.digest(command.encode()),execution_sha256=job.digest(command.encode()),source_archive_sha256='1'*64)
 def spawn(self,row,command):
  child=subprocess.Popen([sys.executable,'-I',str(S/'completed-job.py'),'run',job.encoded(row),command],env=dict(os.environ),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
  self.children.append(child);return child
 def await_running(self,row):
  deadline=time.monotonic()+10
  while time.monotonic()<deadline:
   observed=job.observe(row)
   if observed['state']=='RUNNING' and 'child' in observed:return observed
   time.sleep(.02)
  self.fail('real foreground child did not become owned RUNNING: '+repr(observed))
 def command(self,row,exit_code,hold=0):
  root=self.home/row['output_root']
  code='import pathlib,time,sys; p=pathlib.Path('+repr(str(root))+'); p.mkdir(parents=True); (p/"result.compact.json").write_text("{}\\n"); (p/"raw.log").write_text("native control raw\\n"); time.sleep('+str(hold)+'); sys.exit('+str(exit_code)+')'
  return shlex.quote(sys.executable)+' -I -c '+shlex.quote(code)
 def test_genuine_success_and_every_nonzero_exit_are_fetchable(self):
  for code in (0,1,7,100,101,110):
   with self.subTest(code=code):
    row=self.row('placeholder');command=self.command(row,code);row['command_sha256']=row['execution_sha256']=job.digest(command.encode());child=self.spawn(row,command)
    out,err=child.communicate(timeout=15);self.assertEqual(child.returncode,code,err.decode())
    observed=job.observe(row);self.assertEqual(observed['state'],'TERMINAL');self.assertEqual(observed['actual_exit'],code)
    result=job.verify(row,row['lane'],row['source_commit'],[row['output_root']+'/result.compact.json'])
    self.assertEqual(result['actual_exit'],code);self.assertEqual(set(result['artifacts']),{row['output_root']+'/result.compact.json'})
    self.assertTrue((job.ledger(row)/'start.manifest.json').is_relative_to(self.home));self.assertTrue(job.marker(row).is_relative_to(self.home))
 def test_existing_output_file_does_not_mean_completion(self):
  row=self.row('placeholder');command=self.command(row,0,2);row['command_sha256']=row['execution_sha256']=job.digest(command.encode());child=self.spawn(row,command)
  observed=self.await_running(row);self.assertTrue(observed['owner_alive']);self.assertIsNone(observed['actual_exit'])
  deadline=time.monotonic()+1
  while not (self.home/row['output_root']/'result.compact.json').exists() and time.monotonic()<deadline:time.sleep(.01)
  self.assertTrue((self.home/row['output_root']/'result.compact.json').exists());self.assertEqual(job.observe(row)['state'],'RUNNING')
  with self.assertRaises(FileNotFoundError):job.verify(row,row['lane'],row['source_commit'],[row['output_root']+'/result.compact.json'])
  child.communicate(timeout=10);self.assertEqual(job.observe(row)['actual_exit'],0)
 def test_real_hangup_is_forwarded_and_actual_exit_retained(self):
  row=self.row('placeholder');command=self.command(row,0,30);row['command_sha256']=row['execution_sha256']=job.digest(command.encode());child=self.spawn(row,command)
  observed=self.await_running(row);self.assertEqual(observed['owner']['pid'],child.pid)
  child.send_signal(signal.SIGHUP);out,err=child.communicate(timeout=15)
  final=job.observe(row);self.assertEqual(final['state'],'TERMINAL');self.assertNotEqual(final['actual_exit'],0);self.assertEqual(child.returncode,final['actual_exit'])
  actual=json.loads((job.ledger(row)/'exit.manifest.json').read_text());self.assertIn(signal.SIGHUP,actual['signals']);self.assertEqual(actual['actual_exit'],final['actual_exit'])
 def test_missing_start_disconnected_observation_and_stale_identity_stay_unknown(self):
  row=self.row('exit 0');self.assertEqual(job.observe(row)['state'],'UNKNOWN')
  args=type('Args',(),dict(receipt=str(self.scratch/'receipt.json'),lane=row['lane'],source=row['source_commit'],output=str(self.scratch/'observation.summary.json'),seconds=1))()
  pathlib.Path(args.receipt).write_text(json.dumps(row))
  with mock.patch.object(job,'remote',side_effect=RuntimeError('SSH observation255')):
   self.assertEqual(job.status(args,True),3)
  observed=json.loads(pathlib.Path(args.output).read_text());self.assertEqual(observed['state'],'UNKNOWN');self.assertIsNone(observed['actual_exit'])
  command=self.command(row,0,2);row['command_sha256']=row['execution_sha256']=job.digest(command.encode());child=self.spawn(row,command);self.await_running(row)
  for key,value in [('source_commit','b'*40),('command_sha256','c'*64),('execution_sha256','d'*64),('box_id','bx_other'),('port',19002)]:
   with self.subTest(key=key):self.assertEqual(job.observe(dict(row,**{key:value}))['state'],'UNKNOWN')
  child.communicate(timeout=10)
 def test_current_native_boot_and_pid_start_ticks_are_required(self):
  row=self.row('placeholder');command=self.command(row,0,2);row['command_sha256']=row['execution_sha256']=job.digest(command.encode());child=self.spawn(row,command);self.await_running(row)
  p=job.ledger(row)/'start.manifest.json';original=json.loads(p.read_text());self.assertEqual(original['boot_id'],job.boot_id())
  altered=json.loads(json.dumps(original));altered['boot_id']='different-native-boot';p.write_text(json.dumps(altered));self.assertEqual(job.observe(row)['state'],'UNKNOWN')
  altered=json.loads(json.dumps(original));altered['owner']['start_ticks']+=1;p.write_text(json.dumps(altered));self.assertEqual(job.observe(row)['state'],'UNKNOWN')
  p.write_text(json.dumps(original));self.assertEqual(job.observe(row)['state'],'RUNNING');child.communicate(timeout=10)
 def test_actual_exit_survives_catalog_failure(self):
  command='python3 -I -c '+shlex.quote('import pathlib,os; p=pathlib.Path(os.environ["HOME"])/"'+ 'PLACEHOLDER' +'";p.mkdir(parents=True);(p/"huge.compact.json").write_bytes(b"x"*524289)')
  row=self.row(command);command=command.replace('PLACEHOLDER',row['output_root']);row['command_sha256']=row['execution_sha256']=job.digest(command.encode());child=self.spawn(row,command);out,err=child.communicate(timeout=10)
  self.assertEqual(child.returncode,3,err.decode());observed=job.observe(row);self.assertEqual(observed['state'],'TERMINAL');self.assertEqual(observed['actual_exit'],0);self.assertEqual(observed['runner_exit'],3)
  with self.assertRaisesRegex(ValueError,'catalog failed'):job.verify(row,row['lane'],row['source_commit'],[row['output_root']+'/huge.compact.json'])
 def test_closed_observation_pipe_keeps_real_command_exit(self):
  for expected in (0,101):
   with self.subTest(native_exit=expected):
    row=self.row('placeholder')
    code='import time,os,sys;time.sleep(.5);os.write(1,b"native output after observer disconnect");sys.exit('+str(expected)+')'
    command='exec '+shlex.quote(sys.executable)+' -I -c '+shlex.quote(code)
    row['command_sha256']=row['execution_sha256']=job.digest(command.encode());child=self.spawn(row,command);self.await_running(row)
    child.stdout.close();child.stdout=None
    child.communicate(timeout=10);observed=job.observe(row)
    self.assertEqual(observed['state'],'TERMINAL');self.assertEqual(observed['actual_exit'],expected)
    self.assertEqual(child.returncode,expected);self.assertTrue(observed['observer_pipe_closed'])
    raw=observed['execution_raw'];data=pathlib.Path(raw['path']).read_bytes()
    self.assertEqual(data,b'native output after observer disconnect');self.assertEqual(raw['sha256'],job.digest(data));self.assertTrue(pathlib.Path(raw['path']).is_relative_to(self.home))
 def test_log_sync_failure_keeps_real_exit_without_fetch_permission(self):
  for expected in (0,101):
   with self.subTest(native_exit=expected):
    row=self.row('exit '+str(expected));command='exit '+str(expected)
    code="import importlib.util,os,pathlib; spec=importlib.util.spec_from_file_location('job',"+repr(str(S/'completed-job.py'))+");j=importlib.util.module_from_spec(spec);spec.loader.exec_module(j);real=os.fsync\ndef sync(fd):\n p=os.readlink('/proc/self/fd/'+str(fd))\n if p.endswith('/command.log'):raise OSError('controlled diagnostic fsync failure')\n return real(fd)\nos.fsync=sync\nj.run(j.decode("+repr(job.encoded(row))+"),"+repr(command)+")"
    child=subprocess.Popen([sys.executable,'-I','-c',code],stdout=subprocess.PIPE,stderr=subprocess.PIPE);self.children.append(child)
    out,err=child.communicate(timeout=10);self.assertNotEqual(child.returncode,0)
    native=json.loads((job.ledger(row)/'exit.manifest.json').read_text());self.assertEqual(native['actual_exit'],expected)
    observed=job.observe(row);self.assertEqual(observed['state'],'TERMINAL');self.assertEqual(observed['actual_exit'],expected);self.assertIsNone(observed['runner_exit'])
    self.assertFalse((job.ledger(row)/'terminal.reserve').exists());self.assertFalse(job.marker(row).exists())
    with self.assertRaises(FileNotFoundError):job.verify(row,row['lane'],row['source_commit'],[])
 def test_missing_final_catalog_keeps_durable_native_exit_without_guessed_runner_exit(self):
  row=self.row('exit 7');child=self.spawn(row,'exit 7');child.communicate(timeout=10);job.marker(row).unlink()
  observed=job.observe(row);self.assertEqual(observed['state'],'TERMINAL');self.assertEqual(observed['actual_exit'],7);self.assertIsNone(observed['runner_exit'])
 def test_wrong_child_start_identity_is_unknown(self):
  row=self.row('placeholder');command=self.command(row,0,2);row['command_sha256']=row['execution_sha256']=job.digest(command.encode());child=self.spawn(row,command);self.await_running(row)
  p=job.ledger(row)/'child.manifest.json';original=json.loads(p.read_text());altered=dict(original,started_ns=original['started_ns']+1);p.write_text(json.dumps(altered))
  self.assertEqual(job.observe(row)['state'],'UNKNOWN');p.write_text(json.dumps(original));child.communicate(timeout=10)
 def test_execution_changes_and_occupied_namespace_never_spawn(self):
  row=self.row('exit 0')
  with mock.patch.object(job.subprocess,'Popen') as spawn:
   with self.assertRaisesRegex(ValueError,'command mismatch'):job.run(row,'exit 1')
   (self.home/row['output_root']).mkdir(parents=True)
   with self.assertRaisesRegex(ValueError,'already exists'):job.run(row,'exit 0')
   spawn.assert_not_called()
 def test_completion_with_wrong_boot_source_or_execution_never_reports_success(self):
  row=self.row('exit 0');child=self.spawn(row,'exit 0');child.communicate(timeout=10);p=job.marker(row);original=json.loads(p.read_text())
  for key,value in [('boot_id','different-native-boot'),('source_commit','b'*40),('execution_sha256','c'*64),('actual_exit',None)]:
   with self.subTest(key=key):
    p.write_text(json.dumps(dict(original,**{key:value})));self.assertEqual(job.observe(row)['state'],'UNKNOWN')
  p.write_text(json.dumps(original));self.assertEqual(job.observe(row)['actual_exit'],0)

if __name__=='__main__':unittest.main()
