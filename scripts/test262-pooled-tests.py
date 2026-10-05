#!/usr/bin/env python3
"""Native tooling controls; the protocol fixture is not a JS engine proof."""
import argparse
import contextlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('pool_native', SCRIPTS / 'test262-pool-native.py')
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)

HOST = '''#!/usr/bin/env python3
import sys,json,os
if '--capabilities-v2' in sys.argv or '--capabilities' in sys.argv:
 print(json.dumps({'protocol':1,'async':False}));sys.exit(0)
payload=sys.stdin.buffer.read()
failed=os.environ.get('POOL_FIXTURE_LOST')=='1' and b'CASE_000' in payload
print('TEST262_RESULT '+json.dumps({'protocol':1,'phase':'runtime' if failed else 'complete','error_type':'Error' if failed else ''}))
'''


class PoolControls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix='amazing-pooled-controls-')
        cls.root = Path(cls.temp.name)
        cls.corpus = cls.root / 'corpus'
        (cls.corpus / 'test').mkdir(parents=True)
        (cls.corpus / '.git').mkdir()
        (cls.corpus / '.git/HEAD').write_text('0' * 40 + '\n')
        (cls.corpus / 'harness').mkdir()
        for i in range(64):
            (cls.corpus / f'test/case-{i:03}.js').write_text(f'/*---\nflags: [raw]\n---*/\nCASE_{i:03};\n')
        cls.bin = cls.root / 'bin'
        cls.bin.mkdir()
        cargo = cls.bin / 'cargo'
        cargo.write_text('#!' + sys.executable + '\n' +
            "import os,pathlib,sys\np=pathlib.Path(os.environ['CARGO_TARGET_DIR'])\np.mkdir(parents=True,exist_ok=True)\nwith (p/'build-count').open('a') as f:f.write('build\\n')\nif os.environ.get('POOL_BUILD_FAIL'):sys.exit(101)\nh=p/'release/examples/test262_host'\nh.parent.mkdir(parents=True,exist_ok=True)\nh.write_text(" + repr(HOST) + ")\nh.chmod(0o755)\n")
        cargo.chmod(0o755)
        cls.env = {'PATH': str(cls.bin) + os.pathsep + os.environ['PATH'],
                   'TEST262_ROOT': str(cls.corpus)}
        cls.before = cls.run_groups('before')

    @classmethod
    def tearDownClass(cls):
        # Raw tooling output has authenticated hashes in the outer qualification;
        # keep native fixtures/phase logs for custody, not local evidence commits.
        cls.temp._finalizer.detach()

    @classmethod
    def run_groups(cls, name, lost=False):
        reports = []
        for box, group in enumerate(native.partition(16, 4)):
            root = cls.root / name / str(box)
            args = argparse.Namespace(out=str(root / 'compact'), raw=str(root / 'raw'),
                scripts=str(SCRIPTS), source='1'*40, total=16, shards=group)
            env = dict(cls.env, CARGO_TARGET_DIR=str(root / 'target'), POOL_FIXTURE_LOST='1' if lost else '0')
            with patch.dict(os.environ, env), (cls.root / 'phase-stream.log').open('a') as log, contextlib.redirect_stdout(log):
                rc = native.execute(args, qualification_scope='tooling fixture: protocol producer, not engine corpus')
            if rc != 0:
                raise AssertionError(f'actual pool execution {rc}: {root}')
            reports.extend(root / 'compact' / f'shard-{k}.compact.json' for k in group)
        return reports

    def merge(self, reports, baseline=None):
        output = self.root / (self.id().split('.')[-1] + '-' + str(__import__('time').time_ns()) + '.summary.json')
        argv = [sys.executable, str(SCRIPTS / 'test262-merge.py'), str(output), *map(str, reports)]
        if baseline:
            argv += ['--baseline', str(baseline)]
        actual = subprocess.run(argv, capture_output=True)
        return actual.returncode, output

    def test_all_original_ids_once(self):
        self.assertEqual(sorted(k for group in native.partition(16, 4) for k in group), list(range(16)))

    def test_invalid_capacity_rejected(self):
        for shards, boxes in [(0,4),(16,0),(16,17),(17,4)]:
            with self.assertRaises(ValueError):native.partition(shards, boxes)

    def test_native_build_once_each_box(self):
        for i in range(4):
            self.assertEqual((self.root / 'before' / str(i) / 'target/build-count').read_text(), 'build\n')

    def test_same_real_fixture_binary_all_shards(self):
        self.assertEqual(len({json.loads(p.read_text())['host_sha256'] for p in self.before}), 1)

    def test_raw_never_enters_captured_OUT_and_native_hash_is_authentic(self):
        for i in range(4):
            root = self.root / 'before' / str(i)
            for path in (root / 'compact').rglob('*.json'):
                self.assertTrue(path.name.endswith(('.compact.json', '.summary.json', '.manifest.json')))
            for path in (root / 'compact').glob('*.compact.json'):
                row = json.loads(path.read_text())['raw']
                raw = Path(row['path'])
                self.assertTrue(raw.is_relative_to(root / 'raw'))
                self.assertEqual(native.digest(raw), row['sha256'])
                self.assertEqual(raw.stat().st_size, row['bytes'])

    def test_profile_and_compact_phase_witnesses_are_byte_exact(self):
        for i in range(4):
            row = json.loads((self.root / 'before' / str(i) / 'compact/pool.summary.json').read_text())
            self.assertEqual(len(row['shard_results']), 4)
            for result in row['shard_results']:
                self.assertTrue(result['profile_verified'])
                self.assertEqual(native.digest(result['compact_path']), result['compact_sha256'])
            for phase in row['phases']:
                self.assertEqual(native.digest(phase['receipt']), phase['receipt_sha256'])

    def test_original_jobs_timeout(self):
        for i in range(4):
            for row in json.loads((self.root / 'before' / str(i) / 'compact/pool.summary.json').read_text())['phases']:
                if row['phase'].endswith('-runner'):
                    argv=json.loads(Path(row['receipt']).read_text())['command']
                    self.assertEqual(argv[argv.index('--jobs')+1], '4')
                    self.assertEqual(argv[argv.index('--timeout')+1], '60')

    def test_positive_whole16_public_merge(self):
        rc,path=self.merge(self.before)
        self.assertEqual(rc,0)
        self.assertEqual(json.loads(path.read_text())['summary']['files']['pass'],64)

    def test_positive_zero_loss(self):
        rc,base=self.merge(self.before);self.assertEqual(rc,0)
        rc,after=self.merge(self.before,base);self.assertEqual(rc,0)
        self.assertEqual(json.loads(after.read_text())['lost_variants'],[])

    def test_negative_real_runner_lost_pass_exit1(self):
        # Actual protocol producer reports a runtime error, public runner classifies
        # it, compact writer preserves it, public merger refuses the lost pass.
        _,base=self.merge(self.before)
        copies=self.run_groups('lost',lost=True)
        rc,after=self.merge(copies,base)
        self.assertEqual(rc,1)
        self.assertEqual(json.loads(after.read_text())['lost_files'],['case-000.js'])

    def test_missing_shard_rejected(self):
        self.assertEqual(self.merge(self.before[:-1])[0],2)

    def test_duplicate_shard_rejected(self):
        self.assertEqual(self.merge(self.before[:-1]+self.before[:1])[0],2)

    def test_mixed_binary_rejected(self):
        row=json.loads(self.before[0].read_text());row['host_sha256']='f'*64
        p=self.root/'changed.compact.json';p.write_text(json.dumps(row))
        self.assertEqual(self.merge([p]+self.before[1:])[0],2)

    def test_actual_build101_no_runner(self):
        root=self.root/'build-fail'
        args=argparse.Namespace(out=str(root/'compact'),raw=str(root/'raw'),scripts=str(SCRIPTS),source='1'*40,total=16,shards=[0,4,8,12])
        with patch.dict(os.environ,dict(self.env,CARGO_TARGET_DIR=str(root/'target'),POOL_BUILD_FAIL='1')):
            self.assertEqual(native.execute(args, qualification_scope='tooling fixture: real build101 producer, not engine corpus'),3)
        row=json.loads((root/'compact/build.phase.manifest.json').read_text())
        self.assertEqual(row['engine_exit'],101)
        self.assertEqual(list((root/'compact').glob('*.compact.json')),[])

    def test_runner_rejects_missing_async_boolean_actual2(self):
        host = self.root / 'invalid-capabilities-host'
        host.write_text(HOST.replace("'async':False", "'missing_async':False"))
        host.chmod(0o755)
        output = self.root / 'invalid-capabilities.json'
        child = subprocess.run([sys.executable, str(SCRIPTS / 'test262.py'), str(self.corpus),
            '--binary', str(host), '--jobs', '4', '--timeout', '60', '--shard', '0/16',
            '--output', str(output)], capture_output=True)
        self.assertEqual(child.returncode, 2)
        self.assertIn(b'host async capability must be a boolean', child.stderr)
        self.assertFalse(output.exists())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--summary')
    parser.add_argument('--whole', action='store_true', help='include source/native-input/job-state public controls')
    args = parser.parse_args()
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(PoolControls)
    modules = [Path(__file__).name]
    if args.whole:
        for name in ('native-input-tests.py', 'source-upload-tests.py', 'test262-job-state-tests.py'):
            spec = importlib.util.spec_from_file_location(name.replace('-', '_'), SCRIPTS / name)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            suite.addTests(unittest.defaultTestLoader.loadTestsFromModule(module))
            modules.append(name)
    expected = suite.countTestCases()
    started = time.time_ns()
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    actual = 0 if result.wasSuccessful() and result.testsRun == expected and not result.skipped else 1
    if args.summary:
        witnesses = []
        if hasattr(PoolControls, 'root'):
            for path in sorted(PoolControls.root.rglob('*.phase.manifest.json')):
                row = json.loads(path.read_text())
                witnesses.append({'path': str(path), 'sha256': native.digest(path),
                    'phase': row['phase'], 'engine_exit': row['engine_exit'],
                    'wrapper_exit': row['wrapper_exit'], 'raw_native': row.get('raw_native'),
                    'output_copy_exit': row.get('output_copy_exit'),
                    'diagnostic_tail': row.get('diagnostic_tail', [])[-3:]})
        native.write(args.summary, {'schema': 1, 'scope': 'tooling controls only; synthetic protocol producer is not JS engine qualification',
            'actual_exit': actual, 'expected_controls': expected, 'executed_controls': result.testsRun,
            'failures': [{'id': t.id(), 'detail': detail[-4000:]} for t, detail in result.failures],
            'errors': [{'id': t.id(), 'detail': detail[-4000:]} for t, detail in result.errors],
            'skipped': [(t.id(), reason) for t, reason in result.skipped],
            'started_ns': started, 'finished_ns': time.time_ns(),
            'public_suite_sha256': {name: native.digest(SCRIPTS / name) for name in modules},
            'native_phase_witnesses': witnesses,
            'native_fixture_root': str(PoolControls.root) if hasattr(PoolControls, 'root') else None})
    return actual


if __name__ == '__main__':
    raise SystemExit(main())
