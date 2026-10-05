#!/usr/bin/env python3
"""Whole public workspace launcher controls; child fixtures are not engine proof."""
import argparse
import contextlib
import importlib.util
import io
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

SCRIPTS = pathlib.Path(__file__).parent

def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), SCRIPTS / name)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value

launcher = load('linux-workspace-gate.py')
native = load('linux-workspace-native.py')
SOURCE = os.environ.get('COMMIT', 'a' * 40)
ROOT = pathlib.Path(tempfile.mkdtemp(prefix='workspace-launcher-controls-'))

class Controls(unittest.TestCase):
    def setUp(self):
        self.path = ROOT / self._testMethodName
        self.path.mkdir()
        self.request = launcher.prepare(SOURCE, 'workspace-controls', self.path / 'request')
        self.config = json.loads((self.path / 'request/config.json').read_text())
        self.config.update(raw_root=str(self.path / 'raw'), compact_absolute=str(self.path / 'compact'))
        self.env = dict(COMMIT=SOURCE, LANE='workspace-controls', **launcher.PROFILE)

    def execute(self, exits=(0,0)):
        commands = [[sys.executable, '-c',
            "import sys; print('Summary 2 tests run: 2 passed, 0 failed, 0 skipped' if " + str(i == 0) +
            " else 'test result: ok. 1 passed; 0 failed; 0 ignored;');sys.exit(" + str(code) + ")"] for i, code in enumerate(exits)]
        with patch.dict(os.environ, self.env), contextlib.redirect_stdout(io.StringIO()):
            return native.execute(self.config, commands)

    def test_canonical_lane_all_namespaces(self):
        plan = json.loads((self.path/'request/storage-plan.json').read_text())
        self.assertEqual(plan['ticket'], self.request['lane'])
        self.assertIn('/workspace-controls/', plan['output_root'])
        self.assertEqual(plan['compact_root'], '/home/user/'+self.request['compact_root'])
        self.assertEqual(self.request['argv'][2], self.request['lane'])

    def test_standalone_physical_before_without_corpus_baseline(self):
        plan = json.loads((self.path/'request/storage-plan.json').read_text())
        self.assertEqual(plan['phase'], 'before')
        self.assertNotIn('baseline_manifest', plan)
        self.assertEqual(self.request['corpus_role'], 'standalone workspace candidate')

    def test_initial_wrong_after_phase_still_rejected(self):
        path = self.path/'request/storage-plan.json'
        plan = json.loads(path.read_text()); plan['phase']='after'; path.write_text(json.dumps(plan))
        plan_module = launcher.load('native-storage-plan')
        with self.assertRaises(plan_module.retention.StorageError):
            plan_module.validate(path, native=False)

    def test_clean_alias_mismatch_still_rejected(self):
        with self.assertRaises(ValueError): launcher.load('completed-job').valid_output_root('fresh-clean-alias', self.request['compact_root'])

    def test_escape_and_invalid_alias_rejected(self):
        for lane in ('../escape', 'a/b', 'a b'):
            with self.assertRaises(ValueError): launcher.prepare(SOURCE,lane,self.path/'bad')

    def test_command_file_and_source_helper_hashes(self):
        self.assertIn('--command-file', self.request['argv'])
        self.assertEqual(self.request['command_sha256'],launcher.sha((self.path/'request/command.sh').read_bytes()))
        self.assertEqual(self.request['storage_sha256'],launcher.sha((self.path/'request/storage-plan.json').read_bytes()))
        for name in launcher.HELPERS:
            self.assertEqual(self.request['helper_sha256'][name], launcher.sha((SCRIPTS/name).read_bytes()))

    def test_fresh_job_and_request_cannot_reuse_directory(self):
        other=launcher.prepare(SOURCE,'workspace-controls',self.path/'other')
        self.assertNotEqual(other['job'],self.request['job'])
        with self.assertRaises(FileExistsError): launcher.prepare(SOURCE,'workspace-controls',self.path/'request')

    def test_generated_command_file_runs_actual_production_cli(self):
        # Real generated shell + embedded frozen helpers; the cargo protocol
        # fixture is deliberately not a Rust engine or workspace qualification.
        binary = self.path / 'bin'
        binary.mkdir()
        cargo = binary / 'cargo'
        cargo.write_text('#!' + sys.executable + '\n' +
            "import json,os,pathlib,sys; p=pathlib.Path(os.environ['FIXTURE_CALLS']); "
            "p.open('a').write(json.dumps(sys.argv[1:])+'\\n'); "
            "print('Summary 2 tests run: 2 passed, 0 failed, 0 skipped' if sys.argv[1]=='nextest' "
            "else 'test result: ok. 1 passed; 0 failed; 0 ignored;')\n")
        cargo.chmod(0o755)
        calls = self.path / 'calls.jsonl'
        env = dict(os.environ, **self.env, FIXTURE_CALLS=str(calls))
        env['PATH'] = str(binary) + os.pathsep + env['PATH']
        actual = subprocess.run(['bash', str(self.path/'request/command.sh')], env=env, capture_output=True)
        self.assertEqual(actual.returncode, 0, actual.stderr[-2000:])
        self.assertEqual([json.loads(x) for x in calls.read_text().splitlines()], [
            ['nextest','run','--workspace','--locked','--no-fail-fast'],
            ['test','--workspace','--locked','--doc']])
        compact = pathlib.Path('/home/user') / self.request['compact_root']
        row = json.loads((compact/'workspace.manifest.json').read_text())
        self.assertEqual(row['source_commit'], SOURCE)
        self.assertEqual(row['phase_exits'], {'nextest':0,'doctests':0})
        self.assertFalse(list(compact.glob('*.log')))
        # Keep this native raw/compact fixture for custody as well.
        (self.path/'generated-command-custody.summary.json').write_text(json.dumps({
            'raw_root': self.request['raw_root'], 'compact_root':str(compact),
            'manifest_sha256':launcher.sha((compact/'workspace.manifest.json').read_bytes())}))

    def test_actual_child_success_raw_native_only_counts(self):
        self.assertEqual(self.execute(),0)
        row=json.loads((self.path/'compact/workspace.manifest.json').read_text())
        self.assertEqual(row['counts']['nextest']['passed'],2)
        self.assertEqual(row['counts']['doctests']['passed'],1)
        self.assertFalse(list((self.path/'compact').glob('*.log')))
        for raw in row['raw']:
            self.assertEqual(raw['sha256'], launcher.sha(pathlib.Path(raw['path']).read_bytes()))

    def test_real_child_101_does_not_skip_docs_or_claim_success(self):
        self.assertEqual(self.execute((101,0)),1)
        row=json.loads((self.path/'compact/workspace.manifest.json').read_text())
        self.assertEqual(row['phase_exits'],{'nextest':101,'doctests':0})
        self.assertEqual(row['job_exit'],1)

    def test_real_child_signal_preserved(self):
        commands=[[sys.executable,'-c','import os,signal;os.kill(os.getpid(),signal.SIGTERM)'],[sys.executable,'-c','print("docs ran")']]
        with patch.dict(os.environ,self.env),contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(native.execute(self.config,commands),1)
        row=json.loads((self.path/'compact/workspace.manifest.json').read_text())
        self.assertEqual(row['phase_exits']['nextest'],143)
        self.assertEqual(row['phase_details'][0]['command_exit'],-15)

    def test_publisher_failure_after_real_children_is_not_success(self):
        # Phase receipts succeed; only final compact publication fails.
        real=native.load
        def fault(name,root):
            value=real(name,root)
            if name=='gate-summary': value.write=lambda *a: (_ for _ in ()).throw(OSError('publisher fixture'))
            return value
        with patch.object(native,'load',fault),self.assertRaises(OSError): self.execute()
        for name in ('nextest','doctests'):
            row=json.loads((self.path/'compact'/(name+'.phase.manifest.json')).read_text())
            self.assertEqual(row['engine_exit'],0)
        self.assertFalse((self.path/'compact/workspace.manifest.json').exists())

    def test_native_source_lane_profile_helper_identity_rejection(self):
        for field in ('source','lane'):
            config=dict(self.config); config[field]='incorrect'
            with patch.dict(os.environ,self.env),self.assertRaises(ValueError):native.execute(config)
        with patch.dict(os.environ,dict(self.env,CARGO_INCREMENTAL='1')),self.assertRaises(ValueError):native.execute(self.config)
        config=dict(self.config,helper_sha256={k:'0'*64 for k in self.config['helper_sha256']})
        with patch.dict(os.environ,self.env),self.assertRaises(ValueError):native.execute(config)

    def test_admission_28_125_gib_default_finite_profile(self):
        plan=json.loads((self.path/'request/storage-plan.json').read_text())
        total=sum(plan[k] for k in ('build_budget_bytes','retention_budget_bytes','bootstrap_budget_bytes','reserve_bytes'))
        self.assertEqual(total,28.125*1024**3)
        self.assertEqual(self.request['finite_run_ttl_seconds'],7200)
        self.assertFalse(plan['retire_after_retention'])


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--summary',required=True);args=parser.parse_args()
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(Controls)
    suite.addTests(unittest.defaultTestLoader.loadTestsFromModule(load('test262-job-state-tests.py')))
    expected=suite.countTestCases();started=time.time_ns()
    result=unittest.TextTestRunner(verbosity=2).run(suite)
    storage_path=pathlib.Path(args.summary).with_name('storage-controls.summary.json')
    storage=subprocess.run([sys.executable,str(SCRIPTS/'native-storage-retention-tests.py'),'--source',SOURCE,'--output',str(storage_path)],check=False)
    storage_row=json.loads(storage_path.read_text()) if storage_path.is_file() else None
    code=0 if result.wasSuccessful() and result.testsRun==expected and not result.skipped and storage.returncode==0 else 1
    value={'schema':1,'scope':'public tooling fixtures only; not engine workspace qualification',
        'source':SOURCE,'actual_exit':code,'expected_controls':expected,'executed_controls':result.testsRun,
        'whole_expected_controls':expected+43,
        'whole_executed_controls':result.testsRun+(storage_row.get('tests_run',0) if storage_row else 0),
        'failures':[{ 'id':t.id(),'detail':s[-4000:]} for t,s in result.failures],
        'errors':[{ 'id':t.id(),'detail':s[-4000:]} for t,s in result.errors],
        'skipped':[(t.id(),s) for t,s in result.skipped], 'storage_suite_exit':storage.returncode,
        'storage_suite':storage_row,'native_fixture_root':str(ROOT),
        'started_ns':started,'finished_ns':time.time_ns()}
    witnesses=[]
    for path in sorted(ROOT.rglob('*.phase.manifest.json')):
        row=json.loads(path.read_text());witnesses.append({'path':str(path),'sha256':launcher.sha(path.read_bytes()),'engine_exit':row.get('engine_exit'),'wrapper_exit':row.get('wrapper_exit'),'raw_native':row.get('raw_native')})
    value['phase_witnesses']=witnesses
    launcher.load('gate-summary').write(args.summary,value)
    return code

if __name__=='__main__':raise SystemExit(main())
