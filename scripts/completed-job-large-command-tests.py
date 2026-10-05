#!/usr/bin/env python3
"""Real native large foreground commands, file capture, and custody controls."""
import importlib.util,json,os,pathlib,subprocess,sys,tempfile,unittest,uuid
S=pathlib.Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('completed_job',S/'completed-job.py');job=importlib.util.module_from_spec(spec);spec.loader.exec_module(job)
class LargeCommands(unittest.TestCase):
 def row(self,command):
  return dict(schema=2,lane='public-job-large-command-control',source_commit=os.environ.get('COMMIT','a'*40),box_id='bx_ownedcontrol',host='user@127.0.0.1',port=19001,job_id=uuid.uuid4().hex,output_root='amazing/out/public-job-large-command-control/'+uuid.uuid4().hex,command_sha256=job.digest(command.encode()),execution_sha256=job.digest(command.encode()),source_archive_sha256='1'*64)
 def execute(self,command,row):
  # This is a real bash foreground launch, with the same stdin envelope that
  # linux-run streams over SSH. Payload never appears in the process arguments.
  launch=job.launch(row,command)
  return subprocess.run(['bash','-s'],input=launch.encode(),capture_output=True,timeout=45)
 def test_full16_shaped_large_success_and_nonzero_commands(self):
  for actual in (0,101):
   with self.subTest(actual=actual):
    command='set -eu\n'+''.join('# phase build runner packaging shard '+str(k)+'/16 '+('x'*32768)+'\n' for k in range(16))
    command+='for shard in {0..15}; do for phase in build runner packaging; do printf \'phase %s %s\\n\' \"$shard\" \"$phase\"; done; done\n'
    command+='mkdir -p "$HOME/__ROOT__"\nprintf \'%s\\n\' \'{"full16_like":true}\' > "$HOME/__ROOT__/result.compact.json"\nprintf \'native-large-command __ACTUAL__\\n\'\nexit __ACTUAL__\n'
    row=self.row(command);command=command.replace('__ROOT__',row['output_root']).replace('__ACTUAL__',str(actual));row['command_sha256']=row['execution_sha256']=job.digest(command.encode())
    self.assertGreater(len(command.encode()),524288)
    result=self.execute(command,row);self.assertEqual(result.returncode,actual,result.stderr.decode())
    observed=job.observe(row);self.assertEqual(observed['state'],'TERMINAL');self.assertEqual(observed['actual_exit'],actual);self.assertEqual(observed['runner_exit'],actual)
    script=pathlib.Path(observed['native_diagnostics']['path']).parent/'command.sh';self.assertEqual(script.read_bytes(),command.encode());self.assertTrue(script.is_relative_to(pathlib.Path('/tmp')))
    raw=observed['execution_raw'];data=pathlib.Path(raw['path']).read_bytes();self.assertTrue(data.endswith(('native-large-command '+str(actual)+'\n').encode()));self.assertEqual(data.count(b'phase '),48);self.assertEqual(raw['sha256'],job.digest(data))
    fetched=job.verify(row,row['lane'],row['source_commit'],[row['output_root']+'/result.compact.json']);self.assertEqual(fetched['actual_exit'],actual)
 def test_multi_megabyte_payload_has_no_exec_argument_limit(self):
  command='# '+('q'*2200000)+'\nprintf \'multi-megabyte native\\n\'\nexit 0\n';row=self.row(command)
  result=self.execute(command,row);self.assertEqual(result.returncode,0,result.stderr.decode());self.assertEqual(job.observe(row)['actual_exit'],0)
 def test_capture_and_launch_files_preserve_exact_command_bytes(self):
  with tempfile.TemporaryDirectory(dir=str(pathlib.Path.home())) as temp:
   root=pathlib.Path(temp);command='# '+('y'*300000)+'\nprintf \'file transport\\n\'\nexit 101\n';p=root/'command.sh';p.write_bytes(command.encode());receipt=root/'endpoint.manifest.json'
   result=subprocess.run([sys.executable,'-I',str(S/'completed-job.py'),'capture','--receipt',str(receipt),'--lane','public-job-large-command-control','--source',os.environ.get('COMMIT','a'*40),'--box','bx_ownedcontrol','--host','user@127.0.0.1','--port','19001','--root','amazing/out/public-job-large-command-control/'+uuid.uuid4().hex,'--command-file',str(p),'--execution-file',str(p),'--source-archive-sha256','1'*64],capture_output=True,timeout=10)
   self.assertEqual(result.returncode,0,result.stderr.decode());row=json.loads(receipt.read_text());self.assertEqual(row['command_sha256'],job.digest(command.encode()));self.assertEqual(row['execution_sha256'],job.digest(command.encode()))
   emitted=subprocess.run([sys.executable,'-I',str(S/'completed-job.py'),'launch',job.encoded(row),'--execution-file',str(p)],capture_output=True,timeout=10)
   self.assertEqual(emitted.returncode,0,emitted.stderr.decode());native=subprocess.run(['bash','-s'],input=emitted.stdout,capture_output=True,timeout=30);self.assertEqual(native.returncode,101);self.assertEqual(job.observe(row)['actual_exit'],101)
 def test_large_changed_execution_is_rejected_before_start(self):
  command='# '+('z'*200000)+'\nexit 0\n';row=self.row(command)
  with self.assertRaisesRegex(ValueError,'command mismatch'):job.launch(row,command+'exit 101\n')
  self.assertEqual(job.observe(row)['state'],'UNKNOWN');self.assertFalse(job.ledger(row).exists())
if __name__=='__main__':unittest.main()
