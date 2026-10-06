#!/usr/bin/env python3
"""Native link and stop-admission controls; no engine or Boat lifecycle."""
import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest
import uuid
from unittest.mock import patch
P=Path(__file__).resolve().parent
parser=argparse.ArgumentParser();parser.add_argument('--source',required=True);parser.add_argument('--output',required=True);OPTIONS=parser.parse_args()
def load(name):
 s=importlib.util.spec_from_file_location(name,P/(name+'.py'));m=importlib.util.module_from_spec(s);s.loader.exec_module(m);return m
archive=load('native-proof-archive');guard=load('native-stop-admission');reaper=load('boat-idle-reaper');job=load('completed-job')
ROOT=Path(tempfile.mkdtemp(prefix='native-proof-stop-controls.',dir='/tmp'))
class Controls(unittest.TestCase):
 def fixture(self,code=0):
  root=ROOT/uuid.uuid4().hex;root.mkdir();home=root/'home';home.mkdir();proof=root/'proof';proof.mkdir();retained=proof/'retained';retained.mkdir();source=root/'source';source.mkdir()
  (source/'Cargo.toml').write_text('[fixture]\n');(source/'source.tar.gz').write_bytes(b'immutable fixture source packet')
  packet=archive.digest(source/'source.tar.gz');raw=proof/'command.log';outcome=subprocess.run(['/usr/bin/true' if code==0 else '/usr/bin/false'],capture_output=True);raw.write_bytes(outcome.stdout+outcome.stderr)
  elf=retained/'measured';shutil.copy2('/usr/bin/true',elf);self.assertEqual(subprocess.run([str(elf)]).returncode,0);generated=retained/'generated.h';generated.write_bytes(b'generated provenance unchanged')
  rows=[{'blob':str(f),'kind':kind,'bytes':f.stat().st_size,'sha256':archive.digest(f)} for f,kind in [(elf,'elf'),(generated,'generated-source')]]
  token=uuid.uuid4().hex;profile={'CARGO_PROFILE_DEV_DEBUG':'line-tables-only','CARGO_PROFILE_TEST_DEBUG':'line-tables-only','CARGO_INCREMENTAL':'0'}
  manifest=proof/'retention.json';manifest.write_text(json.dumps({'source':OPTIONS.source,'job':'native-fixture','profile':profile,'artifacts':rows}))
  name='amazing/out/stop-controls/'+token+'/storage.summary.json';compact=home/name;compact.parent.mkdir(parents=True);compact.write_text(json.dumps({'source':OPTIONS.source,'job':'native-fixture','profile':profile,'artifact_manifest':{'path':str(manifest),'bytes':manifest.stat().st_size,'sha256':archive.digest(manifest)}}))
  row={'source_commit':OPTIONS.source,'job_id':token,'box_id':'bx_fixture','source_archive_sha256':packet,'completed':True,'actual_exit':outcome.returncode,'output_root':str(Path(name).parent),'execution_raw':{'path':str(raw),'bytes':raw.stat().st_size,'sha256':archive.digest(raw)},'artifacts':{name:{'bytes':compact.stat().st_size,'sha256':archive.digest(compact)}},'catalog_error':None}
  terminal=home/('amazing-completed-'+token+'.json');terminal.write_text(json.dumps(row));(home/'.amazing-jobs'/token).mkdir(parents=True)
  return root,home,proof,source,terminal,row
 def capture(self,fixture,**kwargs):
  root,home,proof,source,terminal,row=fixture
  return archive.archive('stop-controls',OPTIONS.source,row['job_id'],{'proof':proof,'source':source},terminal,32*1024**2,64*1024**2,**kwargs)
 def register(self,fixture,result):
  return guard.register(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0,'bx_fixture',fixture[1])
 def test_original_source_links_require_explicit_capture_and_preserve_metadata(self):
  f=self.fixture();source=f[3];(source/'alias').symlink_to('Cargo.toml');(source/'directory').symlink_to('.')
  with self.assertRaisesRegex(ValueError,'alias|directory'):self.capture(f)
  result=self.capture(f,capture_links=True);manifest=json.loads(Path(result['archive_manifest']['path']).read_text());self.assertEqual(len(manifest['links']),2)
  self.assertEqual({r['target'] for r in manifest['links']},{'Cargo.toml','.'});self.assertTrue((source/'alias').is_symlink())
  self.assertTrue(archive.verify(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0)['complete'])
 def test_declared_external_regular_and_directory_targets_are_captured_not_dereferenced_links(self):
  f=self.fixture();target=f[0]/'target';target.mkdir();(target/'real').write_bytes(b'actual external bytes');file=f[0]/'single';file.write_bytes(b'only declared file')
  (f[3]/'external').symlink_to(target);(f[3]/'single-alias').symlink_to(file)
  with self.assertRaisesRegex(ValueError,'undeclared/escaping'):self.capture(f,capture_links=True)
  result=self.capture(f,capture_links=True,targets={'target':target,'single':file});m=json.loads(Path(result['archive_manifest']['path']).read_text())
  self.assertEqual(len(m['links']),2);self.assertTrue(archive.verify(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0)['complete'])
  self.assertEqual((Path(result['destination'])/'payload/single/single').read_bytes(),b'only declared file')
 def test_dangling_loop_fifo_and_undeclared_targets_have_exact_path_diagnostics(self):
  for kind in ('dangling','loop','fifo','escape'):
   f=self.fixture();link=f[3]/kind
   if kind=='dangling':link.symlink_to('missing')
   elif kind=='loop':link.symlink_to(link)
   elif kind=='fifo':os.mkfifo(f[3]/'pipe');link.symlink_to('pipe')
   else:link.symlink_to('/usr/bin/true')
   with self.subTest(kind=kind):
    with self.assertRaises(ValueError) as caught:self.capture(f,capture_links=True)
    self.assertIn(str(link),str(caught.exception));self.assertTrue(link.is_symlink())
 def test_target_and_link_mutation_never_publish_complete(self):
  f=self.fixture();(f[3]/'link').symlink_to('Cargo.toml');real=archive.shutil.copyfileobj;changed=False
  def copy(source,output,*args):
   nonlocal changed
   real(source,output,*args)
   if not changed:changed=True;(f[3]/'link').unlink();(f[3]/'link').symlink_to('source.tar.gz')
  with patch.object(archive.shutil,'copyfileobj',side_effect=copy):
   with self.assertRaises(ValueError):self.capture(f,capture_links=True)
  path=archive.ROOT/'stop-controls'/OPTIONS.source/f[5]['job_id'];self.assertFalse((path/'archive.manifest.json').exists());self.assertTrue((path/'failure.manifest.json').exists())
 def test_registered_complete_all_byte_closure_admits_failed_producer_without_pass_claim(self):
  f=self.fixture(1);result=self.capture(f);registered=self.register(f,result);self.assertTrue(registered['failed_producer_preserved'])
  row=guard.admission('bx_fixture',f[1]);self.assertTrue(row['admitted'],row);self.assertEqual(row['jobs'][0]['producer_exit'],1);self.assertEqual(row['jobs'][0]['closure']['measured_elf_records'],1);self.assertEqual(row['jobs'][0]['closure']['generated_source_records'],1)
 def test_absent_partial_bad_exit_and_foreign_archive_failclosed(self):
  f=self.fixture();guard.REGISTRY.mkdir(mode=0o700,exist_ok=True);self.assertFalse(guard.admission('bx_fixture',f[1])['admitted'])
  result=self.capture(f)
  for code in (1,3,124):
   with self.subTest(code=code):
    with self.assertRaises(ValueError):guard.register(result['archive_manifest']['path'],result['archive_manifest']['sha256'],code,'bx_fixture',f[1])
  with self.assertRaises(ValueError):guard.register(result['archive_manifest']['path'],'0'*64,0,'bx_fixture',f[1])
  with self.assertRaises(ValueError):guard.register(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0,'bx_foreign',f[1])
 def test_uncaptured_source_packet_retained_elf_generated_or_whole_raw_rejects_registration(self):
  for omitted in ('source','command.log','measured','generated.h'):
   f=self.fixture();roots={} if omitted=='source' else {'source':f[3]}
   targets={'raw':f[2]/'command.log','manifest':f[2]/'retention.json','elf':f[2]/'retained/measured','generated':f[2]/'retained/generated.h'}
   names={'command.log':'raw','measured':'elf','generated.h':'generated'}
   if omitted in names:del targets[names[omitted]]
   result=archive.archive('stop-controls',OPTIONS.source,f[5]['job_id'],roots,f[4],32*1024**2,64*1024**2,targets=targets)
   with self.subTest(omitted=omitted):
    with self.assertRaises(ValueError):self.register(f,result)
 def test_original_terminal_catalog_or_captured_payload_change_rejects_stop(self):
  for mutation in ('terminal','catalog','payload'):
   f=self.fixture();result=self.capture(f);self.register(f,result)
   if mutation=='terminal':f[4].write_text('{}')
   elif mutation=='catalog':next((f[1]/'amazing/out').rglob('*.json')).write_text('{}')
   else:(Path(result['destination'])/'payload/proof/retained/measured').write_bytes(b'changed')
   with self.subTest(mutation=mutation):self.assertFalse(guard.admission('bx_fixture',f[1])['admitted'])
 def test_live_or_new_partial_job_inhibits_stop_without_archival_or_signals(self):
  f=self.fixture();result=self.capture(f);self.register(f,result);(f[1]/'.amazing-jobs'/uuid.uuid4().hex).mkdir();row=guard.admission('bx_fixture',f[1]);self.assertFalse(row['admitted']);self.assertIn('live/partial',row['errors'][0])
 def test_reaper_production_verifier_executes_without_boat_lifecycle(self):
  f=self.fixture();result=self.capture(f);self.register(f,result);calls=[]
  def native(*args,**kwargs):
   self.assertEqual(args[:2],('exec','bx_fixture'));calls.append(args)
   command=shlex.split(args[2]);completed=subprocess.run(command,env=dict(os.environ,HOME=str(f[1])),capture_output=True,text=True,timeout=30)
   return {'success':completed.returncode==0,'stdout':completed.stdout,'stderr':completed.stderr}
  with patch.object(reaper,'boat',side_effect=native):self.assertTrue(reaper.archive_admission('bx_fixture')['admitted'])
  self.assertEqual(len(calls),1);self.assertNotIn('stop',calls[0][:2]);self.assertNotIn('extend',calls[0][:2])
 def test_both_idle_and_resize_paths_require_archives_before_any_stop(self):
  class EndLoop(Exception):pass
  for resize in (False,True):
   for admitted in (False,True):
    calls=[];sandbox={'id':'bx_fixture','name':'amazing-builder','type':'large' if resize else 'default','state':'idle'}
    patches=[patch.object(reaper,'running_sandboxes',return_value=[sandbox]),patch.object(reaper,'managed_ids',return_value={'bx_fixture'}),patch.object(reaper,'wanted_types',return_value={'bx_fixture':'default'}),patch.object(reaper,'starts_left_this_hour',return_value=100),patch.object(reaper,'is_busy',return_value=False),patch.object(reaper,'custody_status',return_value={'held':False}),patch.object(reaper,'archive_admission',return_value={'admitted':admitted}),patch.object(reaper,'boat',side_effect=lambda *a,**k:calls.append(a) or {'success':True}),patch.object(reaper.time,'sleep',side_effect=EndLoop),patch('sys.argv',['boat-idle-reaper.py','--idle-minutes','0','--scarce-idle-minutes','0','--no-auto-extend'])]
    import contextlib
    with contextlib.ExitStack() as stack:
     for item in patches:stack.enter_context(item)
     with self.assertRaises(EndLoop):reaper.main()
    with self.subTest(resize=resize,admitted=admitted):self.assertEqual(calls,[('stop','bx_fixture')] if admitted else [])
 def test_public_captured_raw_optin_actual_native_bytes_without_legacy_relocation(self):
  row={'job_id':uuid.uuid4().hex,'captured_proof_root':'/srv/amazing-proof'};path=job.diagnostic_file(row);self.assertTrue(archive.ROOT in path.parents)
  path.write_bytes(b'actual controlled stdout');with_size=path.stat().st_size;self.assertEqual(with_size,24)
  legacy=job.diagnostic_file({'job_id':uuid.uuid4().hex});self.assertTrue(Path('/tmp') in legacy.parents)

class Results(unittest.TextTestResult):
 def __init__(self,*a,**k):super().__init__(*a,**k);self.classifications={}
 def addSuccess(self,test):super().addSuccess(test);self.classifications[test._testMethodName]='pass'
 def addFailure(self,test,error):super().addFailure(test,error);self.classifications[test._testMethodName]='fail'
 def addError(self,test,error):super().addError(test,error);self.classifications[test._testMethodName]='error'
result=unittest.TextTestRunner(verbosity=2,resultclass=Results).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls))
row={'schema':1,'source':OPTIONS.source,'actual_exit':int(not result.wasSuccessful()),'tests_run':result.testsRun,'failures':len(result.failures),'errors':len(result.errors),'classifications':result.classifications,'fixture_root':str(ROOT),'engine_or_boat_lifecycle':False,'diagnostics':{t._testMethodName:e[-4096:] for t,e in result.failures+result.errors}}
p=Path(OPTIONS.output);p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(row,separators=(',',':'))+'\n');print(json.dumps(row,separators=(',',':')));raise SystemExit(row['actual_exit'])
