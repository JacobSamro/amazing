#!/usr/bin/env python3
"""Actual Bash3.2 production argv carrier, not a Mac/engine test."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest
import uuid

p=argparse.ArgumentParser();p.add_argument('--bash32',required=True);p.add_argument('--source',required=True);p.add_argument('--output',required=True);OPTIONS=p.parse_args();S=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('job',S/'completed-job.py');job=importlib.util.module_from_spec(spec);spec.loader.exec_module(job)
ROOT=Path(tempfile.mkdtemp(prefix='public-bash32-controls.',dir='/tmp'))
CALLER=(S/'linux-run.sh').read_text();START=CALLER.index('    PROOF_CAPTURE_ARGS=');END=CALLER.index('\n    capture_rc=$?',START);BLOCK=CALLER[START:END]
# Exact original3d capture block, retained only as diagnostic input. No old
# runtime/engine source imported; successor still invokes actual public helper.
OLD=BLOCK.replace('PROOF_CAPTURE_ARGS=(capture)','PROOF_CAPTURE_ARGS=()').replace('PROOF_CAPTURE_ARGS+=(--captured-proof-root','PROOF_CAPTURE_ARGS=(--captured-proof-root').replace('completed-job.py" "${PROOF_CAPTURE_ARGS[@]}"','completed-job.py" capture "${PROOF_CAPTURE_ARGS[@]}"')
class Controls(unittest.TestCase):
 def fixture(self,optional='',block=BLOCK,text='printf actual-native-caller'):
  root=ROOT/uuid.uuid4().hex;root.mkdir();command=root/'command';execution=root/'execution';command.write_text(text);execution.write_text(text);receipt=root/'endpoint.manifest.json'
  values={'REPO':str(S.parent),'CAPTURED_PROOF_ROOT':optional,'ENDPOINT_RECEIPT':str(receipt),'LANE':'bash32-native-control','COMMIT':OPTIONS.source,'BOAT_ID':'bx_fixture','BOAT_HOST':'user@127.0.0.1','BOAT_PORT':'19001','COMPACT_ROOT':'amazing/out/bash32-native-control/'+uuid.uuid4().hex,'command_file':str(command),'execution_file':str(execution),'SOURCE_ARCHIVE_SHA':'a'*64,'MISSION_SHA256':'b'*64}
  script='set -euo pipefail\n'+'\n'.join(k+'='+shlex.quote(v) for k,v in values.items())+'\n'+block+'\nprintf "%s\\n" "$JOB_ROW"\n'
  result=subprocess.run([OPTIONS.bash32,'--noprofile','--norc','-s'],input=script,capture_output=True,text=True,timeout=30)
  return root,receipt,result,values
 def test_measured_native_shell_is_actual_gnu_bash32(self):
  result=subprocess.run([OPTIONS.bash32,'--version'],capture_output=True,text=True,timeout=10);self.assertEqual(result.returncode,0);self.assertIn('version 3.2.',result.stdout)
 def test_original_empty_optional_array_reproduces_unbound_without_receipt(self):
  root,receipt,result,values=self.fixture(block=OLD);self.assertNotEqual(result.returncode,0);self.assertIn('unbound variable',result.stderr);self.assertFalse(receipt.exists())
 def test_exact_production_block_both_optional_modes_capture_once(self):
  for optional in ('','/srv/amazing-proof'):
   root,receipt,result,values=self.fixture(optional);self.assertEqual(result.returncode,0,result.stderr);row=json.loads(receipt.read_text())
   self.assertEqual(row['schema'],2);self.assertEqual(row['source_commit'],OPTIONS.source);self.assertEqual(row['lane'],values['LANE']);self.assertEqual(row['command_sha256'],row['execution_sha256']);self.assertEqual(row['source_archive_sha256'],'a'*64)
   self.assertEqual(row.get('captured_proof_root'),optional or None);self.assertEqual(len(result.stdout.splitlines()),1);self.assertEqual(job.decode(result.stdout.strip()),row)
 def test_literal_quoted_unicode_multiline_command_digest_preserved(self):
  text="printf '%s' 'native space ; $HOME $(false) café 🦀'\n# literal\\\\backslash\n"
  for optional in ('','/srv/amazing-proof'):
   root,receipt,result,values=self.fixture(optional,text=text);self.assertEqual(result.returncode,0,result.stderr);row=json.loads(receipt.read_text());self.assertEqual(row['command_sha256'],job.digest(text.encode()));self.assertEqual(row['execution_sha256'],row['command_sha256'])
 def test_invalid_optional_root_refuses_receipt_with_actual_helper_status(self):
  root,receipt,result,values=self.fixture('/tmp/not-captured');self.assertEqual(result.returncode,2);self.assertFalse(receipt.exists());self.assertIn('invalid choice',result.stderr)
 def test_actual_large_native_launch_after_bash32_capture_preserves_exit(self):
  for actual in (0,101):
   text='set -eu\n'+('# preserved native large carrier\n'*16000)+'printf actual-native-large-caller\nexit '+str(actual)+'\n'
   root,receipt,result,values=self.fixture(text=text);self.assertEqual(result.returncode,0,result.stderr);row=json.loads(receipt.read_text());launch=job.launch(row,text)
   invoked=subprocess.run([OPTIONS.bash32,'--noprofile','--norc','-s'],input=launch,capture_output=True,text=True,timeout=40);self.assertEqual(invoked.returncode,actual,invoked.stderr);observed=job.observe(row);self.assertEqual(observed['state'],'TERMINAL');self.assertEqual(observed['actual_exit'],actual)

class Results(unittest.TextTestResult):
 def __init__(self,*a,**k):super().__init__(*a,**k);self.classifications={}
 def addSuccess(self,t):super().addSuccess(t);self.classifications[t._testMethodName]='pass'
 def addFailure(self,t,e):super().addFailure(t,e);self.classifications[t._testMethodName]='fail'
 def addError(self,t,e):super().addError(t,e);self.classifications[t._testMethodName]='error'
r=unittest.TextTestRunner(verbosity=2,resultclass=Results).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls));shell=Path(OPTIONS.bash32)
row={'schema':1,'source':OPTIONS.source,'tests_run':r.testsRun,'failures':len(r.failures),'errors':len(r.errors),'actual_exit':int(not r.wasSuccessful()),'classifications':r.classifications,'shell':{'path':str(shell),'bytes':shell.stat().st_size,'sha256':__import__('hashlib').sha256(shell.read_bytes()).hexdigest(),'version':subprocess.run([str(shell),'--version'],capture_output=True,text=True).stdout.splitlines()[0]},'production_block_sha256':job.digest(BLOCK.encode()),'old_block_sha256':job.digest(OLD.encode()),'scope':'Actual Linux Bash3.2 argv/carrier fixtures; no Mac tests/engine or Boat lifecycle','diagnostics':{t.id():e[-4096:] for t,e in r.failures+r.errors}}
output=Path(OPTIONS.output);output.parent.mkdir(parents=True,exist_ok=True);output.write_text(json.dumps(row,separators=(',',':'))+'\n');print(json.dumps(row,separators=(',',':')));raise SystemExit(row['actual_exit'])
