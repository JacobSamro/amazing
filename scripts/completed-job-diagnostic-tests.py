#!/usr/bin/env python3
"""Real foreground native children; diagnostic faults never qualify an engine."""
import importlib.util,json,os,pathlib,shlex,signal,subprocess,sys,time,unittest,uuid

S=pathlib.Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('completed_job',S/'completed-job.py')
job=importlib.util.module_from_spec(spec);spec.loader.exec_module(job)
CONTROL_RECEIPTS=[]

class DiagnosticBoundary(unittest.TestCase):
 def setUp(self):
  self.children=[];self.addCleanup(self.cleanup)
 def cleanup(self):
  for child in self.children:
   if child.poll() is None:child.send_signal(signal.SIGTERM)
   try:child.communicate(timeout=15)
   except subprocess.TimeoutExpired:child.kill();child.communicate()
 def prepare(self,exit_code=0,hold=0):
  row=dict(schema=2,lane='public-job-diagnostic-control',source_commit=os.environ.get('COMMIT','a'*40),box_id='bx_ownedcontrol',host='user@127.0.0.1',port=19001,job_id=uuid.uuid4().hex,output_root='amazing/out/public-job-diagnostic-control/'+uuid.uuid4().hex,source_archive_sha256='1'*64)
  output=pathlib.Path.home()/row['output_root']
  payload=json.dumps({'actual_control_exit':exit_code})+'\n'
  code='import pathlib,time,sys; print("real child started",flush=True); time.sleep('+str(hold)+'); p=pathlib.Path('+repr(str(output))+'); p.mkdir(parents=True); (p/"result.compact.json").write_text('+repr(payload)+'); print("real child finished",flush=True); sys.exit('+str(exit_code)+')'
  command='exec '+shlex.quote(sys.executable)+' -I -c '+shlex.quote(code)
  row['command_sha256']=row['execution_sha256']=job.digest(command.encode())
  return row,command
 def spawn(self,row,command,mode='none'):
  # Fault injection is confined to this actual supervisor process. It does
  # not alter mounts, provider lifecycle, TTL, or any other job's filesystem.
  patch=''
  if mode=='native-stream':
   patch='''
real=pathlib.Path.open
class Fault:
 def __init__(self,file):self.file=file
 def write(self,data):raise OSError(107,"controlled native diagnostic stream disconnect")
 def flush(self):return self.file.flush()
 def fileno(self):return self.file.fileno()
 def close(self):return self.file.close()
def opening(path,*args,**kwargs):
 file=real(path,*args,**kwargs)
 return Fault(file) if path.name=="command.log" else file
pathlib.Path.open=opening
'''
  elif mode=='home-log':
   patch='''
real=pathlib.Path.open
def opening(path,*args,**kwargs):
 if path.name=="command.log" and path.is_relative_to(pathlib.Path.home()):raise OSError(107,"controlled HOME diagnostic mount disconnect")
 return real(path,*args,**kwargs)
pathlib.Path.open=opening
try:(j.ledger(row)/"command.log").open("xb")
except OSError as error:assert error.errno==107
else:raise AssertionError("HOME fault was not armed")
'''
  program='import importlib.util,pathlib,sys; s=importlib.util.spec_from_file_location("job",'+repr(str(S/'completed-job.py'))+');j=importlib.util.module_from_spec(s);s.loader.exec_module(j);row=j.decode('+repr(job.encoded(row))+')\n'+patch+'\nsys.exit(j.run(row,'+repr(command)+'))'
  child=subprocess.Popen([sys.executable,'-I','-c',program],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
  self.children.append(child);return child
 def await_status(self,row,predicate,seconds=15):
  deadline=time.monotonic()+seconds;observed=None
  while time.monotonic()<deadline:
   observed=job.observe(row)
   if predicate(observed):return observed
   time.sleep(.02)
  self.fail('actual status condition not reached: '+repr(observed))
 def record(self,row):
  CONTROL_RECEIPTS.append({'control':self._testMethodName,'endpoint_identity':job.identity(row),'observation':job.observe(row),'native_exit_witness':json.loads((job.ledger(row)/'exit.manifest.json').read_text())})
 def test_long_native_child_survives_stream_fault_with_honest_tooling_failure(self):
  row,command=self.prepare(0,70);child=self.spawn(row,command,'native-stream')
  observed=self.await_status(row,lambda x:x['state']=='RUNNING' and x.get('diagnostic_error',{}).get('errno')==107)
  self.assertTrue(observed['owner_alive']);self.assertTrue(observed['child_alive']);self.assertIsNone(observed['actual_exit'])
  self.assertTrue(pathlib.Path(observed['native_diagnostics']['path']).is_relative_to(pathlib.Path('/tmp')))
  self.assertFalse((pathlib.Path.home()/row['output_root']/'result.compact.json').exists())
  out,err=child.communicate(timeout=90);self.assertEqual(child.returncode,3,err.decode())
  observed=job.observe(row);self.assertEqual(observed['actual_exit'],0);self.assertEqual(observed['runner_exit'],3);self.assertEqual(observed['signals'],[])
  self.assertEqual(observed['diagnostic_error']['errno'],107)
  ended=json.loads((job.ledger(row)/'exit.manifest.json').read_text())
  self.assertNotIn('execution_raw',ended);self.assertTrue(ended['execution_raw_partial']['hash_not_claimed'])
  self.assertGreaterEqual((ended['exited_ns']-ended['started_ns'])/1e9,70)
  self.assertTrue((pathlib.Path.home()/row['output_root']/'result.compact.json').exists())
  with self.assertRaisesRegex(ValueError,'catalog failed'):job.verify(row,row['lane'],row['source_commit'],[row['output_root']+'/result.compact.json'])
  self.record(row)
 def test_failed_child_keeps_actual_exit_after_stream_fault(self):
  row,command=self.prepare(101,.2);child=self.spawn(row,command,'native-stream');out,err=child.communicate(timeout=15)
  self.assertEqual(child.returncode,3,err.decode());observed=job.observe(row)
  self.assertEqual(observed['actual_exit'],101);self.assertEqual(observed['runner_exit'],3);self.assertEqual(observed['signals'],[]);self.record(row)
 def test_HOME_log_disconnect_does_not_touch_native_sink_or_compact_custody(self):
  row,command=self.prepare(0,.2);child=self.spawn(row,command,'home-log');out,err=child.communicate(timeout=15)
  self.assertEqual(child.returncode,0,err.decode());observed=job.observe(row)
  self.assertEqual(observed['actual_exit'],0);self.assertIsNone(observed['diagnostic_error'])
  raw=observed['execution_raw'];path=pathlib.Path(raw['path']);self.assertTrue(path.is_relative_to(pathlib.Path('/tmp')))
  self.assertEqual(raw['sha256'],job.digest(path.read_bytes()));self.assertEqual(raw['bytes'],path.stat().st_size)
  self.assertEqual(path.parent.stat().st_mode&0o077,0)
  verified=job.verify(row,row['lane'],row['source_commit'],[row['output_root']+'/result.compact.json'])
  self.assertEqual(verified['actual_exit'],0);self.assertEqual(set(verified['artifacts']),{row['output_root']+'/result.compact.json'});self.record(row)
 def test_real_external_termination_remains_forwarded_and_distinguishable(self):
  row,command=self.prepare(0,30);child=self.spawn(row,command)
  self.await_status(row,lambda x:x['state']=='RUNNING' and x.get('child_alive'))
  child.send_signal(signal.SIGTERM);out,err=child.communicate(timeout=15)
  observed=job.observe(row);self.assertEqual(observed['actual_exit'],143);self.assertEqual(observed['runner_exit'],143)
  self.assertEqual(observed['signals'],[signal.SIGTERM]);self.assertEqual(observed['command_returncode'],-signal.SIGTERM)
  self.assertIsNone(observed['diagnostic_error']);self.record(row)

if __name__=='__main__':unittest.main()
