#!/usr/bin/env python3
"""Whole native cache-classification, lease and selective-unlink controls."""
import argparse
import ctypes
import fcntl
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

parser=argparse.ArgumentParser()
parser.add_argument('--source',required=True);parser.add_argument('--output',required=True)
args=parser.parse_args()
HELPER=pathlib.Path(__file__).with_name('native-template-cache.py')
ROOT=pathlib.Path(tempfile.mkdtemp(prefix='native-template-controls.',dir='/tmp'))
SOURCE=args.source


def sha(path):
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


class Controls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture=ROOT/'compiler';cls.fixture.mkdir()
        rust=cls.fixture/'fixture.rs';rust.write_text('pub fn fixture() -> u32 { 37 }\n')
        subprocess.run(['rustc','--crate-name','fixture','--crate-type','rlib','--emit=link,metadata',
                        '-C','incremental='+str(cls.fixture/'incremental'),str(rust),'--out-dir',str(cls.fixture)],check=True)
        c=cls.fixture/'fixture.c';c.write_text('int fixture(void) { return 37; } int main(void) { return 0; }\n')
        for opts,name in [(['-c'],'fixture.o'),(['-shared','-fPIC'],'fixture.so'),(['-no-pie'],'fixture-exec')]:
            subprocess.run(['cc',*opts,str(c),'-o',str(cls.fixture/name)],check=True)

    def setUp(self):
        self.root=ROOT/self._testMethodName;self.root.mkdir()
        self.home=self.root/'home';self.home.mkdir()
        self.target=self.home/'amazing'/'target';self.target.mkdir(parents=True)
        self.raw=self.root/'inventory.raw.json';self.summary=self.root/'case.summary.json'
        self.environment=dict(os.environ,COMMIT=SOURCE)
        self.environment.pop('AMAZING_NATIVE_BUSY_LEASE',None)

    def command(self,mode='retire'):
        result=[sys.executable,str(HELPER),mode,'--source',SOURCE,'--home-root',str(self.home),
                '--raw-manifest',str(self.raw),'--output',str(self.summary),'--fixture-home']
        if mode=='retire':result+=['--confirm-reconstructible']
        return result

    def invoke(self,command=None,expected=0,environment=None):
        result=subprocess.run(command or self.command(),capture_output=True,text=True,env=environment or self.environment,check=False)
        self.assertEqual(result.returncode,expected,result.stdout+result.stderr)
        return json.loads(result.stdout.splitlines()[-1])

    def payloads(self):
        for name in ('libfixture.rlib','libfixture.rmeta','fixture.o','fixture.so','fixture-exec','fixture.c','fixture.rs'):
            shutil.copy2(self.fixture/name,self.target/name)
        for path in (self.fixture/'incremental').rglob('*.bin'):
            destination=self.target/'debug'/'incremental'/path.relative_to(self.fixture/'incremental')
            destination.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(path,destination)

    def test_production_cli_rejects_arbitrary_home_before_inspection(self):
        command=self.command();command.remove('--fixture-home')
        result=self.invoke(command,expected=3)
        self.assertIn('actual native user HOME',result['error'])
        self.assertFalse(self.raw.exists())

    def test_actual_rust_archive_metadata_and_incremental_are_recognized(self):
        self.payloads();result=self.invoke(self.command('inventory'))
        for kind in ('recoverable-rlib','recoverable-rmeta','recoverable-incremental','recoverable-elf-rel'):
            self.assertGreater(result['counts'][kind]['files'],0)
        self.assertEqual(result['removed_files'],0)
        self.assertTrue((self.target/'libfixture.rlib').exists())

    def test_selective_retirement_preserves_real_executable_shared_library_and_sources(self):
        self.payloads()
        preserved={name:sha(self.target/name) for name in ('fixture-exec','fixture.so','fixture.c','fixture.rs')}
        result=self.invoke()
        self.assertGreaterEqual(result['removed_files'],4)
        for name,value in preserved.items():self.assertEqual(sha(self.target/name),value)
        self.assertFalse((self.target/'fixture.o').exists());self.assertFalse((self.target/'libfixture.rlib').exists())
        self.assertTrue(self.target.is_dir())
        self.assertEqual(subprocess.run([str(self.target/'fixture-exec')],check=False).returncode,0)
        self.assertEqual(ctypes.CDLL(str(self.target/'fixture.so')).fixture(),37)
        self.assertFalse(result['whole_directories_deleted'])

    def test_unrecognized_archive_metadata_and_binary_names_are_protected(self):
        for name in ('fake.rlib','fake.rmeta','unknown.bin','raw.log','file.rs','weird.o'):
            (self.target/name).write_bytes(b'raw/source custody')
        hashes={p.name:sha(p) for p in self.target.iterdir()}
        result=self.invoke();self.assertEqual(result['removed_files'],0)
        for name,value in hashes.items():self.assertEqual(sha(self.target/name),value)

    def test_malformed_and_unknown_elf_never_becomes_reconstructible(self):
        file=self.target/'malformed.o';file.write_bytes(b'\x7fELF\0\0\0');original=sha(file)
        result=self.invoke();self.assertEqual(result['counts']['protected-unknown-elf']['files'],1)
        self.assertEqual(sha(file),original)

    def test_multiple_link_historical_artifact_is_retained(self):
        file=self.target/'fixture.o';shutil.copy2(self.fixture/'fixture.o',file)
        frozen=self.root/'frozen-object';os.link(file,frozen)
        result=self.invoke();self.assertEqual(result['removed_files'],0)
        self.assertEqual(sha(file),sha(frozen));self.assertEqual(result['counts']['protected-multiple-links']['files'],1)

    def test_explicit_historical_path_is_preserved(self):
        file=self.target/'fixture.o';shutil.copy2(self.fixture/'fixture.o',file);original=sha(file)
        result=self.invoke(self.command()+['--protected-path',str(file)])
        self.assertEqual(result['removed_files'],0);self.assertEqual(sha(file),original)

    def test_frozen_artifact_manifest_preserves_original_relative_operand(self):
        file=self.target/'fixture.o';shutil.copy2(self.fixture/'fixture.o',file)
        blob=self.root/'frozen';shutil.copy2(file,blob)
        manifest=self.root/'frozen.json';manifest.write_text(json.dumps({'schema':1,'target':str(self.target),
                     'artifacts':[{'relative_path':'fixture.o','blob':str(blob)}]}))
        result=self.invoke(self.command()+['--protected-manifest',str(manifest)])
        self.assertEqual(result['removed_files'],0);self.assertEqual(sha(file),sha(blob))

    def test_source_specific_native_target_is_outside_all_candidates(self):
        native=self.root/'amazing-native-target-source';native.mkdir()
        file=native/'fixture.o';shutil.copy2(self.fixture/'fixture.o',file);original=sha(file)
        result=self.invoke();self.assertEqual(result['removed_files'],0);self.assertEqual(sha(file),original)
        self.assertFalse(result['source_specific_targets_touched'])

    def test_only_two_exact_generic_roots_are_considered(self):
        second=self.home/'amazing'/'target-shell-shell';second.mkdir();shutil.copy2(self.fixture/'fixture.o',second/'fixture.o')
        third=self.home/'amazing'/'target-lane-retired';third.mkdir();shutil.copy2(self.fixture/'fixture.o',third/'fixture.o')
        result=self.invoke();self.assertEqual(result['removed_files'],1)
        self.assertTrue((third/'fixture.o').is_file());self.assertTrue(second.is_dir())

    def test_peer_live_or_unknown_lease_refuses_all_retirement(self):
        self.payloads();busy=self.home/'.amazing-busy';busy.mkdir();(busy/'unknown-peer.123').touch()
        result=self.invoke(expected=3);self.assertIn('live/UNKNOWN',result['error'])
        self.assertTrue((self.target/'fixture.o').is_file());self.assertFalse(self.raw.exists())

    def test_actual_native_wrapper_own_lease_is_accepted(self):
        self.payloads();busy=self.home/'.amazing-busy';busy.mkdir();own=busy/('fixture.'+str(os.getpid()));own.touch()
        environment=dict(self.environment,LANE='fixture',AMAZING_NATIVE_BUSY_LEASE=str(own))
        self.assertGreater(self.invoke(environment=environment)['removed_files'],0)
        self.assertTrue(own.is_file())

    def test_forged_own_lease_shape_does_not_bypass_unknown_peer(self):
        busy=self.home/'.amazing-busy';busy.mkdir();peer=busy/'peer.123';peer.touch()
        environment=dict(self.environment,LANE='fixture',AMAZING_NATIVE_BUSY_LEASE=str(peer))
        result=self.invoke(expected=3,environment=environment);self.assertIn('authentic wrapper',result['error'])

    def test_real_held_cargo_lock_refuses_retirement(self):
        self.payloads();lock=self.target/'debug'/'.cargo-lock';lock.parent.mkdir(exist_ok=True);lock.touch()
        with lock.open('r+') as stream:
            fcntl.flock(stream,fcntl.LOCK_EX)
            result=self.invoke(expected=3);self.assertIn('Cargo target lock',result['error'])
        self.assertTrue((self.target/'fixture.o').is_file())

    def test_symlink_target_and_subtree_refuse_retirement(self):
        self.payloads();outside=self.root/'outside';outside.write_bytes(b'source intact')
        (self.target/'external').symlink_to(outside)
        result=self.invoke(expected=3);self.assertIn('symlink',result['error'])
        self.assertEqual(outside.read_bytes(),b'source intact');self.assertTrue((self.target/'fixture.o').exists())

    def test_raw_output_cannot_be_inside_cache_candidates(self):
        command=self.command();command[command.index('--raw-manifest')+1]=str(self.target/'raw.json')
        result=self.invoke(command,expected=3);self.assertIn('outside generic',result['error'])
        self.assertFalse((self.target/'raw.json').exists())

    def test_inventory_then_terminal_retirement_records_actual_allocated_bytes(self):
        self.payloads();inventory=self.invoke(self.command('inventory'))
        result=self.invoke()
        self.assertGreater(result['removed_allocated_bytes'],0)
        self.assertEqual(result['raw']['sha256'],sha(self.raw))
        self.assertEqual(result['removed_logical_bytes'],sum(r['bytes'] for r in json.loads(self.raw.read_text())['removed']))
        self.assertEqual(len(result['free_before']),2);self.assertEqual(len(inventory['free_after']),2)

    def test_retirement_requires_explicit_policy_and_exact_launcher_source(self):
        command=self.command();command.remove('--confirm-reconstructible')
        self.assertIn('opt-in',self.invoke(command,expected=3)['error'])
        command=self.command();command[command.index('--source')+1]='1'*40
        self.assertIn('native launcher COMMIT',self.invoke(command,expected=3)['error'])

    def test_real_candidate_mutation_during_hash_preserves_changed_bytes_and_receipt(self):
        file=self.target/'libfixture.rlib';shutil.copy2(self.fixture/'libfixture.rlib',file)
        code=self.root/'mutation.py'
        code.write_text("import importlib.util,pathlib,sys\n"+
            "spec=importlib.util.spec_from_file_location('cache',sys.argv[1]);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)\n"+
            "original=m.sha\n"+
            "def changing(path):\n value=original(path)\n if path.name=='libfixture.rlib':path.write_bytes(path.read_bytes()+b'changed native cache')\n return value\n"+
            "m.sha=changing;sys.argv=[sys.argv[1]]+sys.argv[2:];raise SystemExit(m.main())\n")
        command=[sys.executable,str(code),str(HELPER),*self.command()[2:]]
        result=self.invoke(command,expected=3)
        self.assertIn('changed while hashing',result['error'])
        self.assertEqual(result['removed_files'],0)
        self.assertTrue(file.read_bytes().endswith(b'changed native cache'))
        self.assertEqual(result['raw']['sha256'],sha(self.raw))

    def test_footprint_measurement_records_real_command_exit_and_profile(self):
        raw=self.root/'measure.log';summary=self.root/'measure.summary.json'
        command=[sys.executable,str(HELPER),'measure','--source',SOURCE,'--output',str(summary),
                 '--raw-manifest',str(raw),'--native-root',str(self.root),'--',sys.executable,'-c','print("native footprint fixture"); raise SystemExit(101)']
        environment=dict(self.environment,CARGO_PROFILE_DEV_DEBUG='line-tables-only',CARGO_PROFILE_TEST_DEBUG='line-tables-only',CARGO_INCREMENTAL='0')
        result=self.invoke(command,expected=101,environment=environment)
        self.assertEqual(result['actual_exit'],101);self.assertGreater(result['samples'],0)
        self.assertEqual(result['raw']['sha256'],sha(raw));self.assertEqual(result['profile']['CARGO_INCREMENTAL'],'0')


class Results(unittest.TextTestResult):
    def __init__(self,*args,**kwargs):super().__init__(*args,**kwargs);self.classes={}
    def addSuccess(self,test):super().addSuccess(test);self.classes[test._testMethodName]='pass'
    def addFailure(self,test,error):super().addFailure(test,error);self.classes[test._testMethodName]='fail'
    def addError(self,test,error):super().addError(test,error);self.classes[test._testMethodName]='error'

result=unittest.TextTestRunner(verbosity=2,resultclass=Results).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls))
summary={'schema':1,'source':SOURCE,'tests_run':result.testsRun,'classifications':result.classes,
         'failures':len(result.failures),'errors':len(result.errors),'helper_sha256':sha(HELPER),
         'controls_sha256':sha(pathlib.Path(__file__)),'fixture_root':str(ROOT),'engine_gate':False,
         'diagnostics':{t._testMethodName:'\n'.join(trace.splitlines()[-24:])[-4096:] for t,trace in result.failures+result.errors}}
output=pathlib.Path(args.output);output.parent.mkdir(parents=True,exist_ok=True);output.write_text(json.dumps(summary,sort_keys=True,indent=2)+'\n')
print(json.dumps(summary,separators=(',',':')));sys.exit(0 if result.wasSuccessful() else 1)
