#!/usr/bin/env python3
"""Native compiler/phase diagnostic controls; fixtures never qualify an engine."""
import hashlib, importlib.util, json, pathlib, subprocess, sys, tempfile, unittest
ROOT=pathlib.Path(__file__).resolve().parent

def module(name):
 s=importlib.util.spec_from_file_location(name,ROOT/(name+'.py'));m=importlib.util.module_from_spec(s);s.loader.exec_module(m);return m
D=module('wrapped-failure-diagnostic');G=module('gate-command')
SOURCE='a'*40;JOB='b'*32
class Controls(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory(prefix='amazing-wrapped-diagnostic-control-');self.owner=pathlib.Path(self.tmp.name);self.rp=self.owner/'build.phase.manifest.json';self.tp=self.owner/'terminal.json'
  # Both codes come from genuine native children. The compiler text is an
  # explicit contract fixture, never an alleged engine/compiler invocation.
  actual=subprocess.run([sys.executable,'-c','raise SystemExit(1)']).returncode
  self.terminal={'state':'TERMINAL','actual_exit':actual,'runner_exit':actual,'command_returncode':actual,'job':{'source_commit':SOURCE,'job_id':JOB}}
  G.run([sys.executable,'-c',r'import sys;sys.stderr.write("error[E0308]: controlled type mismatch\n  expected &[Token], found &Vec<Token>\nerror[E0282]: controlled ambiguous type\n");sys.exit(101)'],'build',SOURCE,self.owner/'build.log',self.rp)
  self.receipt=json.loads(self.rp.read_text());self.save()
 def tearDown(self):self.tmp.cleanup()
 def save(self):self.tp.write_text(json.dumps(self.terminal));self.rp.write_text(json.dumps(self.receipt))
 def call(self,source=SOURCE,job=JOB,owner=None,rsha=None,tsha=None):
  return D.produce(self.tp,tsha or hashlib.sha256(self.tp.read_bytes()).hexdigest(),self.rp,rsha or hashlib.sha256(self.rp.read_bytes()).hexdigest(),source,job,str(owner or self.owner))
 def test_all_compiler_clusters_and_original_native_codes_are_retained(self):
  s=json.loads(self.call());self.assertEqual(s['phase_actual_exit'],101);self.assertEqual(s['original_actual_exit'],1);self.assertFalse(s['counts_reconstructed']);self.assertEqual(sum(x['marker'] for x in s['snippets']),2)
 def test_successful_original_terminal_rejected(self):
  actual=subprocess.run([sys.executable,'-c','raise SystemExit(0)']).returncode
  self.terminal.update(actual_exit=actual,runner_exit=actual,command_returncode=actual);self.save()
  with self.assertRaises(ValueError):self.call()
 def test_original_source_rejected(self):
  with self.assertRaises(ValueError):self.call(source='c'*40)
 def test_original_job_rejected(self):
  with self.assertRaises(ValueError):self.call(job='c'*32)
 def test_terminal_unknown_rejected(self):
  self.terminal['state']='UNKNOWN';self.save()
  with self.assertRaises(ValueError):self.call()
 def test_original_terminal_hash_rejected(self):
  with self.assertRaises(ValueError):self.call(tsha='0'*64)
 def test_phase_hash_rejected(self):
  with self.assertRaises(ValueError):self.call(rsha='0'*64)
 def test_phase_source_rejected(self):
  self.receipt['source_commit']='c'*40;self.save()
  with self.assertRaises(ValueError):self.call()
 def test_phase_success_rejected(self):
  self.receipt.update(command_exit=0,engine_exit=0,wrapper_exit=0);self.save()
  with self.assertRaises(ValueError):self.call()
 def test_bool_phase_code_rejected(self):
  self.receipt.update(command_exit=True,engine_exit=True);self.save()
  with self.assertRaises(ValueError):self.call()
 def test_engine_phase_contradiction_rejected(self):
  self.receipt['engine_exit']=0;self.save()
  with self.assertRaises(ValueError):self.call()
 def test_raw_hash_rejected(self):
  self.receipt['raw_native']['sha256']='0'*64;self.save()
  with self.assertRaises(ValueError):self.call()
 def test_raw_size_rejected(self):
  self.receipt['raw_native']['bytes']+=1;self.save()
  with self.assertRaises(ValueError):self.call()
 def test_raw_alias_rejected(self):
  alias=self.owner/'alias.log';alias.symlink_to(self.receipt['raw_native']['path']);self.receipt['raw_native']['path']=str(alias);self.save()
  with self.assertRaises(ValueError):self.call()
 def test_no_marker_does_not_invent_compiler_cause(self):
  raw=pathlib.Path(self.receipt['raw_native']['path']);raw.write_bytes(b'ordinary output');self.receipt['raw_native'].update(bytes=15,sha256=hashlib.sha256(raw.read_bytes()).hexdigest());self.save()
  with self.assertRaises(ValueError):self.call()
 def test_output_context_is_bounded_and_declares_truncation(self):
  raw=pathlib.Path(self.receipt['raw_native']['path']);raw.write_bytes(b'error[E0308]: '+b'x'*9000+b'\n');self.receipt['raw_native'].update(bytes=raw.stat().st_size,sha256=hashlib.sha256(raw.read_bytes()).hexdigest());self.save();s=json.loads(self.call());self.assertTrue(s['snippets'][0]['truncated']);self.assertLess(len(self.call()),65536)
 def test_owner_alias_or_other_owner_rejected(self):
  alias=self.owner/'alias';alias.symlink_to(self.owner)
  with self.assertRaises(ValueError):self.call(owner=alias)
  with self.assertRaises(ValueError):self.call(owner=pathlib.Path('/tmp/not-this-owner'))
if __name__=='__main__':
 result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls))
 output={'native_controls':result.testsRun,'passed':result.testsRun-len(result.failures)-len(result.errors),'failed':len(result.failures)+len(result.errors),'failed_ids':[case.id() for case,_ in result.failures+result.errors],'fixture_only':True,'helper_sha256':hashlib.sha256((ROOT/'wrapped-failure-diagnostic.py').read_bytes()).hexdigest()}
 pathlib.Path(sys.argv[1]).write_text(json.dumps(output)+'\n');raise SystemExit(not result.wasSuccessful())
