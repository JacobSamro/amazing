#!/usr/bin/env python3
"""Whole native filesystem/ELF lifecycle controls; never builds an engine.

All payload executables are exact copies of the native /usr/bin/true ELF.
Invalid fixtures and failed commands are tools controls, not gate results.
"""
import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import unittest

PARSER = argparse.ArgumentParser()
PARSER.add_argument('--output', required=True)
PARSER.add_argument('--source', required=True)
OPTIONS = PARSER.parse_args()
HELPER = pathlib.Path(__file__).with_name('native-storage-retention.py')
ROOT = pathlib.Path(tempfile.mkdtemp(prefix='native-retention-controls.', dir='/tmp'))
SOURCE = OPTIONS.source
BASE = SOURCE  # Tooling fixture source only; /usr/bin/true is never an engine baseline.
ELF = pathlib.Path('/usr/bin/true')


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


class Controls(unittest.TestCase):
    def setUp(self):
        self.root = ROOT / self._testMethodName
        self.root.mkdir()
        self.scratch = self.root / 'scratch'
        self.out = self.root / 'out'
        self.retained = self.out / 'retained'
        self.job = 0

    def invocation(self, phase='before', baseline=None, environment=None, budget=1024 * 1024):
        self.job += 1
        args = [sys.executable, str(HELPER), 'run', '--ticket', 'native-storage-controls',
                '--source', BASE if phase == 'before' else SOURCE, '--phase', phase,
                '--job', 'case-' + str(self.job), '--scratch-parent', str(self.scratch),
                '--output-root', str(self.out), '--retained-root', str(self.retained),
                '--build-budget-bytes', str(budget), '--retention-budget-bytes', str(budget),
                '--reserve-bytes', '4096']
        if baseline:
            args += ['--baseline-manifest', baseline['artifact_manifest']['path'], '--baseline-source', BASE]
        code = "import os,pathlib,shutil; p=pathlib.Path(os.environ['CARGO_TARGET_DIR']); shutil.copy2('/usr/bin/true',p/'measured'); (p/'generated.h').write_text('source intact\\n'); (p/'recoverable.rmeta').write_bytes(b'cache'); print(os.environ['CARGO_TARGET_DIR'])"
        return args + ['--', sys.executable, '-c', code]

    def invoke(self, args, expected=0, env=None):
        result = subprocess.run(args, capture_output=True, text=True, env=env, check=False)
        self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
        return json.loads(result.stdout.splitlines()[-1])

    def baseline(self):
        return self.invoke(self.invocation())

    def test_fresh_pair_preserves_frozen_executable_and_source_after_terminal_cache_release(self):
        before = self.baseline()
        after = self.invoke(self.invocation('after', before))
        self.assertNotEqual(before['target'], after['target'])
        for receipt in [before, after]:
            self.assertFalse(pathlib.Path(receipt['target']).exists())
            manifest = json.loads(pathlib.Path(receipt['artifact_manifest']['path']).read_text())
            self.assertEqual(manifest['command_exit'], 0)
            elf = next(row for row in manifest['artifacts'] if row['kind'] == 'elf')
            self.assertEqual(sha(pathlib.Path(elf['blob'])), sha(ELF))
            self.assertEqual(subprocess.run([elf['blob']], check=False).returncode, 0)
            source = next(row for row in manifest['artifacts'] if row['kind'] == 'generated-source')
            self.assertEqual(pathlib.Path(source['blob']).read_text(), 'source intact\n')
        self.assertEqual(before['profile'], after['profile'])
        self.assertEqual(after['copied_unique_bytes'], 0)

    def test_no_space_budget_refuses_command_and_target_creation(self):
        free = os.statvfs(ROOT).f_bavail * os.statvfs(ROOT).f_frsize
        result = self.invoke(self.invocation(budget=free * 2 + 1), 3)
        self.assertFalse(result['capacity']['admitted'])
        self.assertFalse(self.scratch.exists())
        self.assertFalse(self.out.exists())

    def test_shared_filesystem_budgets_are_summed_before_admission(self):
        free = os.statvfs(ROOT).f_bavail * os.statvfs(ROOT).f_frsize
        result = self.invoke(self.invocation(budget=free * 3 // 5), 3)
        groups = result['capacity']['shared_device_requirements']
        self.assertEqual(len(groups), 1)
        self.assertGreater(groups[0]['required_bytes'], groups[0]['available_bytes'])

    def test_before_after_profile_change_rejects_before_execution(self):
        before = self.baseline()
        environment = dict(os.environ, CARGO_PROFILE_TEST_DEBUG='2')
        result = self.invoke(self.invocation('after', before), 3, environment)
        self.assertIn('profile must match', result['error'])

    def test_native_launcher_source_mismatch_refuses_allocation(self):
        environment = dict(os.environ, COMMIT='2' * 40)
        result = self.invoke(self.invocation(), 3, environment)
        self.assertIn('native launcher COMMIT', result['error'])
        self.assertFalse(self.scratch.exists())
        self.assertFalse(self.out.exists())

    def test_raw_home_style_output_is_untouched_by_target_retirement(self):
        self.out.mkdir()
        raw = self.out / 'raw-phase.log'
        raw.write_bytes(b'original raw output\x00\xff\n')
        original = sha(raw)
        self.baseline()
        self.assertEqual(sha(raw), original)

    def test_changed_retained_baseline_bytes_reject_new_after_target(self):
        before = self.baseline()
        manifest = json.loads(pathlib.Path(before['artifact_manifest']['path']).read_text())
        blob = pathlib.Path(manifest['artifacts'][0]['blob'])
        blob.write_bytes(blob.read_bytes() + b'changed')
        result = self.invoke(self.invocation('after', before), 3)
        self.assertIn('bytes changed', result['error'])
        # Both tools fixture phases use the actual launcher source. The BEFORE
        # source directory remains; only a fresh AFTER must be absent.
        self.assertTrue(pathlib.Path(before['session']).is_dir())
        self.assertFalse((self.scratch / 'native-storage-controls' / SOURCE / 'after').exists())

    def test_baseline_commit_identity_is_exact(self):
        before = self.baseline()
        args = self.invocation('after', before)
        args[args.index('--baseline-source') + 1] = '1' * 40
        result = self.invoke(args, 3)
        self.assertIn('recorded commit', result['error'])

    def test_failed_native_command_records_actual_exit_and_keeps_measured_elf(self):
        args = self.invocation()
        args[-1] += '; raise SystemExit(101)'
        result = self.invoke(args, 101)
        self.assertEqual(result['command_exit'], 101)
        self.assertEqual(result['storage_exit'], 0)
        self.assertEqual(result['retained_elf_files'], 1)
        self.assertTrue(result['target_retired'])
        rejected = self.invoke(self.invocation('after', result), 3)
        self.assertIn('successful before', rejected['error'])

    def test_native_signal_exit_is_recorded_without_claiming_success(self):
        args = self.invocation()
        args[-1] += '; import signal; os.kill(os.getpid(), signal.SIGTERM)'
        result = self.invoke(args, 143)
        self.assertEqual(result['command_returncode'], -15)
        self.assertEqual(result['command_exit'], 143)
        self.assertTrue(result['target_retired'])

    def test_source_phase_job_target_identity_cannot_be_reused(self):
        args = self.invocation()
        first = self.invoke(args)
        second = self.invoke(args, 3)
        self.assertIn('already exists', second['error'])
        self.assertEqual(sha(pathlib.Path(first['artifact_manifest']['path'])), first['artifact_manifest']['sha256'])

    def test_target_symlink_leaves_terminal_cache_intact(self):
        original = self.root / 'source.txt'
        original.write_text('outside source')
        args = self.invocation()
        args[-1] += f"; (p/'external-link').symlink_to({str(original)!r})"
        result = self.invoke(args, 3)
        self.assertEqual(result['command_exit'], 0)
        self.assertFalse(result['retention_complete'])
        self.assertTrue(pathlib.Path(result['target']).is_dir())
        self.assertEqual(original.read_text(), 'outside source')
        rejected = self.invoke([sys.executable, str(HELPER), 'retire', '--session', result['session']], 3)
        self.assertIn('unretained', rejected['error'])

    def test_unknown_state_refuses_retirement_even_without_a_live_lock(self):
        before = self.baseline()
        session = pathlib.Path(before['session'])
        data = json.loads((session / 'session.json').read_text())
        data['state'] = 'RUNNING'  # Explicit invalid tools ledger fixture.
        target = session / 'target'
        target.mkdir()
        (target / 'held.txt').write_text('held unknown fixture')
        (session / 'session.json').write_text(json.dumps(data))
        rejected = self.invoke([sys.executable, str(HELPER), 'retire', '--session', str(session)], 3)
        self.assertIn('RUNNING/UNKNOWN', rejected['error'])
        self.assertEqual((target / 'held.txt').read_text(), 'held unknown fixture')

    def test_live_lease_refuses_cache_mutation_then_real_terminal_releases_it(self):
        args = self.invocation()
        release = self.root / 'release'
        ready = self.root / 'ready'
        args[-1] += f"; import time; pathlib.Path({str(ready)!r}).touch(); " + \
                    f"exec(\"while not pathlib.Path({str(release)!r}).exists():\\n time.sleep(.01)\")"
        process = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        deadline = time.monotonic() + 10
        while not ready.exists():
            self.assertLess(time.monotonic(), deadline)
            time.sleep(.01)
        session = next(self.scratch.glob('*/*/*/*'))
        rejected = self.invoke([sys.executable, str(HELPER), 'retire', '--session', str(session)], 3)
        self.assertIn('live lease', rejected['error'])
        self.assertTrue((session / 'target' / 'measured').exists())
        release.touch()
        stdout, stderr = process.communicate(timeout=10)
        self.assertEqual(process.returncode, 0, stdout + stderr)
        self.assertTrue(json.loads(stdout.splitlines()[-1])['target_retired'])

    def test_no_elf_baseline_cannot_authorize_after_execution(self):
        args = self.invocation()
        args[-1] = 'pass'
        before = self.invoke(args)
        rejected = self.invoke(self.invocation('after', before), 3)
        self.assertIn('no frozen measured executable', rejected['error'])

    def test_raw_output_cannot_be_inside_managed_scratch(self):
        args = self.invocation()
        args[args.index('--output-root') + 1] = str(self.scratch / 'out')
        args[args.index('--retained-root') + 1] = str(self.scratch / 'out' / 'retained')
        rejected = self.invoke(args, 3)
        self.assertIn('outside managed scratch', rejected['error'])
        self.assertFalse(self.scratch.exists())

    def test_retained_root_cannot_escape_raw_output_owner(self):
        args = self.invocation()
        args[args.index('--retained-root') + 1] = str(self.root / 'another-owner')
        rejected = self.invoke(args, 3)
        self.assertIn('explicit output root', rejected['error'])

    def test_prebootstrap_check_allocates_no_target_or_output(self):
        args = [sys.executable, str(HELPER), 'check', '--scratch-parent', str(self.scratch),
                '--output-root', str(self.out), '--build-budget-bytes', '1048576',
                '--retention-budget-bytes', '1048576', '--reserve-bytes', '4096']
        receipt = self.invoke(args)
        self.assertTrue(receipt['capacity']['admitted'])
        self.assertFalse(self.scratch.exists())
        self.assertFalse(self.out.exists())


class Results(unittest.TextTestResult):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.classifications = {}

    def addSuccess(self, test):
        super().addSuccess(test)
        self.classifications[test._testMethodName] = 'pass'

    def addFailure(self, test, error):
        super().addFailure(test, error)
        self.classifications[test._testMethodName] = 'fail'

    def addError(self, test, error):
        super().addError(test, error)
        self.classifications[test._testMethodName] = 'error'


suite = unittest.defaultTestLoader.loadTestsFromTestCase(Controls)
result = unittest.TextTestRunner(verbosity=2, resultclass=Results).run(suite)
output = pathlib.Path(OPTIONS.output)
output.parent.mkdir(parents=True, exist_ok=True)
summary = {'schema': 1, 'source': SOURCE, 'ticket': 'native-storage-retention',
           'tests_run': result.testsRun, 'classifications': result.classifications,
           'failures': len(result.failures), 'errors': len(result.errors),
           'helper_sha256': sha(HELPER), 'controls_sha256': sha(pathlib.Path(__file__)),
           'native_elf': {'path': str(ELF), 'bytes': ELF.stat().st_size, 'sha256': sha(ELF)},
           'fixture_root': str(ROOT), 'engine_build_or_gate': False}
summary['diagnostics'] = {test._testMethodName: '\n'.join(trace.splitlines()[-24:])[-4096:]
                          for test, trace in result.failures + result.errors}
output.write_text(json.dumps(summary, sort_keys=True, indent=2) + '\n')
print(json.dumps(summary, separators=(',', ':')))
sys.exit(0 if result.wasSuccessful() else 1)
