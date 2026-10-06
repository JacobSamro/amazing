#!/usr/bin/env python3
"""Native metadata-reference controls; real measured ELF, no engine build."""
import argparse
import base64
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest
import uuid
from unittest.mock import patch

P=Path(__file__).resolve().parent
parser=argparse.ArgumentParser();parser.add_argument('--output',required=True);parser.add_argument('--source',required=True);OPTIONS=parser.parse_args()
def load(name):
    spec=importlib.util.spec_from_file_location(name,P/(name+'.py'));module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
c=load('completed-job');r=load('native-storage-retention');p=load('native-storage-plan')
def pin(value):return hashlib.sha256((json.dumps(value,sort_keys=True,separators=(',',':'))+'\n').encode()).hexdigest()
ROOT=Path(tempfile.mkdtemp(prefix='remote-baseline-controls.',dir='/tmp'))

class Controls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.home=ROOT/'original-home';cls.home.mkdir()
        cls.row=dict(schema=2,lane='remote-baseline-controls',source_commit=OPTIONS.source,box_id='bx_original',host='user@127.0.0.1',port=22,job_id=uuid.uuid4().hex,output_root='amazing/out/remote-baseline-controls/original',command_sha256='a'*64,execution_sha256='b'*64,source_archive_sha256='c'*64)
        cls.name=cls.row['output_root']+'/storage.summary.json'
        cls.plan=dict(schema=1,ticket='remote-baseline-controls',source=OPTIONS.source,phase='before',job='original',scratch_parent=str(ROOT/'before-scratch'),output_root=str(ROOT/'before-out'),retained_root=str(ROOT/'before-out/retained'),build_budget_bytes=1048576,bootstrap_budget_bytes=1048576,retention_budget_bytes=1048576,reserve_bytes=4096,retention_method='copy',elf_policy='all',compact_root=str(cls.home/cls.row['output_root']),summary_path=str(cls.home/cls.name))
        plan=ROOT/'before-plan.json';plan.write_text(json.dumps(cls.plan));command=ROOT/'before-command.sh'
        script="import os,pathlib,shutil,subprocess;p=pathlib.Path(os.environ['CARGO_TARGET_DIR']);shutil.copy2('/usr/bin/true',p/'measured');assert subprocess.run([str(p/'measured')]).returncode==0;(p/'generated.h').write_text('original immutable source')"
        command.write_text(shlex.join([sys.executable,'-c',script])+'\n')
        invocation=shlex.join([sys.executable,str(P/'native-storage-plan.py'),'run','--plan',str(plan),'--command-file',str(command)])
        cls.row['execution_sha256']=hashlib.sha256(invocation.encode()).hexdigest()
        with patch.object(c.pathlib.Path,'home',return_value=cls.home):
            if c.run(cls.row,invocation)!=0:raise RuntimeError('real native fixture producer failed')
            cls.reference=c.export_baseline(cls.row,cls.name)
        cls.summary=json.loads(base64.b64decode(cls.reference['storage_summary']['payload']))
        cls.manifest=json.loads(Path(cls.summary['artifact_manifest']['path']).read_text())

    def valid(self,reference=None):
        value=reference or self.reference
        return r.validated_remote_reference(value,pin(value),OPTIONS.source)

    def test_real_completed_metadata_reference_preserves_original_source_profile_device(self):
        row=self.valid();self.assertEqual(row['original_job'],self.row);self.assertFalse(row['local_elf_readback'])
        self.assertEqual(row['elf_count'],1);self.assertEqual(row['profile'],r.PROFILE)
        self.assertEqual(row['path'],self.summary['artifact_manifest']['path']);self.assertEqual(row['original_device'],Path(row['path']).stat().st_dev)
        self.assertEqual(row['sha256'],self.summary['artifact_manifest']['sha256'])

    def test_reference_does_not_reopen_or_ferry_original_elf_blobs(self):
        blobs={a['blob'] for a in self.manifest['artifacts']};original=Path.open
        def opened(path,*args,**kwargs):
            if str(path) in blobs:raise AssertionError('receiver attempted remote payload access')
            return original(path,*args,**kwargs)
        with patch.object(Path,'open',opened):
            self.assertFalse(self.valid()['local_elf_readback'])
            with patch.object(c.pathlib.Path,'home',return_value=self.home):self.assertEqual(c.export_baseline(self.row,self.name)['retention_manifest']['elf_count'],1)
        self.assertFalse('payload' in self.reference['retention_manifest'])

    def test_remote_cli_after_runs_fresh_target_and_preserves_before(self):
        before={a['blob']:r.digest(Path(a['blob'])) for a in self.manifest['artifacts']}
        args=[sys.executable,str(P/'native-storage-retention.py'),'run','--ticket','remote-after','--source',OPTIONS.source,'--phase','after','--job',uuid.uuid4().hex,'--scratch-parent',str(ROOT/'after-scratch'),'--output-root',str(ROOT/'after-out'),'--retained-root',str(ROOT/'after-out/retained'),'--build-budget-bytes','1048576','--retention-budget-bytes','1048576','--reserve-bytes','4096','--baseline-source',OPTIONS.source,'--baseline-reference',json.dumps(self.reference),'--baseline-reference-sha256',pin(self.reference),'--','/usr/bin/true']
        result=subprocess.run(args,capture_output=True,text=True,timeout=30);self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        row=json.loads(result.stdout.splitlines()[-1]);self.assertEqual(row['baseline']['mode'],'remote-reference');self.assertFalse(row['baseline']['local_elf_readback'])
        self.assertEqual(before,{name:r.digest(Path(name)) for name in before});self.assertEqual(row['command_exit'],0)

    def test_local_readback_remains_strict_and_remote_does_not_replace_it(self):
        path=self.summary['artifact_manifest']['path'];self.assertEqual(r.validated_manifest(path,OPTIONS.source)['artifact_count'],2)
        original=r.digest
        with patch.object(r,'digest',return_value='0'*64):
            with self.assertRaises(r.StorageError):r.validated_manifest(path,OPTIONS.source)
        self.assertEqual(r.digest,original)
        with self.assertRaises(r.StorageError):r.resolve_baseline(path,OPTIONS.source,self.reference,pin(self.reference))

    def test_completed_source_job_exit_catalog_and_summary_cluster_rejected(self):
        mutations=[lambda x:x['completed'].update(actual_exit=1),lambda x:x['completed'].update(actual_exit=True),lambda x:x['completed'].update(runner_exit=None),lambda x:x['completed'].update(completed=False),lambda x:x['completed'].update(job_id='f'*32),lambda x:x['job'].update(source_commit='d'*40),lambda x:x.update(catalog_sha256='0'*64),lambda x:x['storage_summary'].update(sha256='0'*64),lambda x:x['storage_summary'].update(path='../foreign'),lambda x:x['completed'].update(catalog_error='failure')]
        for change in mutations:
            value=copy.deepcopy(self.reference);change(value)
            with self.subTest(change=change):
                with self.assertRaises((r.StorageError,KeyError,ValueError)):self.valid(value)

    def test_manifest_profile_device_count_and_reference_pin_cluster_rejected(self):
        for key,value in [('sha256','0'*64),('device',-1),('source','d'*40),('job','foreign'),('profile',{}),('elf_count',0),('artifact_count',1000),('metadata_readback',False),('path','../escape')]:
            row=copy.deepcopy(self.reference);row['retention_manifest'][key]=value
            with self.subTest(key=key):
                with self.assertRaises(r.StorageError):self.valid(row)
        with self.assertRaises(r.StorageError):r.validated_remote_reference(self.reference,'0'*64,OPTIONS.source)

    def test_storage_plan_remote_exclusivity_before_rejection_and_profile_no_execution(self):
        plan=dict(self.plan,phase='after',job='after-plan',baseline_source=OPTIONS.source,baseline_reference=self.reference,baseline_reference_sha256=pin(self.reference));file=ROOT/'after-plan.json';file.write_text(json.dumps(plan));self.assertEqual(p.validate(file,native=False)['phase'],'after')
        for fields in [dict(phase='before'),dict(baseline_manifest=self.summary['artifact_manifest']['path']),dict(baseline_reference_sha256='0'*64)]:
            file.write_text(json.dumps(dict(plan,**fields)))
            with self.assertRaises(p.retention.StorageError):p.validate(file,native=False)
        file.write_text(json.dumps(plan));env=dict(os.environ,CARGO_PROFILE_TEST_DEBUG='2')
        result=subprocess.run([sys.executable,str(P/'native-storage-plan.py'),'check','--plan',str(file)],env=env,capture_output=True,text=True,timeout=30);self.assertEqual(result.returncode,3);self.assertIn('profile must match',result.stdout)

    def test_remote_reference_does_not_reduce_capacity_guard(self):
        plan=dict(self.plan,phase='after',job='refuse-space',baseline_source=OPTIONS.source,baseline_reference=self.reference,baseline_reference_sha256=pin(self.reference),scratch_parent=str(ROOT/'refuse-scratch'))
        free=os.statvfs(ROOT).f_bavail*os.statvfs(ROOT).f_frsize;plan['build_budget_bytes']=free*2+1
        file=ROOT/'capacity-plan.json';file.write_text(json.dumps(plan))
        result=subprocess.run([sys.executable,str(P/'native-storage-plan.py'),'check','--plan',str(file)],capture_output=True,text=True,timeout=30);self.assertEqual(result.returncode,3);self.assertFalse(json.loads(result.stdout)['capacity']['admitted']);self.assertFalse((ROOT/'refuse-scratch').exists())

class Results(unittest.TextTestResult):
    def __init__(self,*a,**k):super().__init__(*a,**k);self.classifications={}
    def addSuccess(self,test):super().addSuccess(test);self.classifications[test._testMethodName]='pass'
    def addFailure(self,test,error):super().addFailure(test,error);self.classifications[test._testMethodName]='fail'
    def addError(self,test,error):super().addError(test,error);self.classifications[test._testMethodName]='error'
result=unittest.TextTestRunner(verbosity=2,resultclass=Results).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls))
row={'schema':1,'source':OPTIONS.source,'tests_run':result.testsRun,'failures':len(result.failures),'errors':len(result.errors),'classifications':result.classifications,'actual_exit':int(not result.wasSuccessful()),'fixture_root':str(ROOT),'engine_gate':False,'scope':'Actual native completed-job/storage metadata and measured /usr/bin/true fixtures; not a historical or receiver-side original ELF verification claim','diagnostics':{t._testMethodName:e[-4096:] for t,e in result.errors+result.failures}}
output=Path(OPTIONS.output);output.parent.mkdir(parents=True,exist_ok=True);output.write_text(json.dumps(row,separators=(',',':'))+'\n');print(json.dumps(row,separators=(',',':')));raise SystemExit(row['actual_exit'])
