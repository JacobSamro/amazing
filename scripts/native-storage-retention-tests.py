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
import importlib.util

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


    def policy_invocation(self, phase='before', baseline=None):
        args = self.invocation(phase, baseline)
        index = args.index('--')
        return args[:index] + ['--retention-method', 'hardlink', '--elf-policy', 'runnable'] + args[index:]

    def test_terminal_hardlink_retains_original_inode_without_duplicate_payload(self):
        inode = self.root / 'inode'
        args = self.policy_invocation()
        args[-1] += f"; pathlib.Path({str(inode)!r}).write_text(str((p/'measured').stat().st_ino))"
        receipt = self.invoke(args)
        manifest = json.loads(pathlib.Path(receipt['artifact_manifest']['path']).read_text())
        elf = next(row for row in manifest['artifacts'] if row['kind'] == 'elf')
        self.assertEqual(elf['retention_method'], 'hardlink')
        self.assertEqual(pathlib.Path(elf['blob']).stat().st_ino, int(inode.read_text()))
        self.assertEqual(receipt['copied_unique_bytes'], 0)
        self.assertFalse(pathlib.Path(receipt['target']).exists())
        self.assertEqual(subprocess.run([elf['blob']], check=False).returncode, 0)

    def test_real_relocatable_object_is_reconstructible_only_under_explicit_new_policy(self):
        code = self.root / 'fixture.c'
        code.write_text('int fixture(void) { return 37; }\n')
        obj = self.root / 'fixture.o'
        subprocess.run(['cc', '-c', str(code), '-o', str(obj)], check=True)
        for explicit in (False, True):
            args = self.policy_invocation() if explicit else self.invocation()
            args[-1] += f"; shutil.copy2({str(obj)!r},p/'compiler-object.o')"
            receipt = self.invoke(args)
            manifest = json.loads(pathlib.Path(receipt['artifact_manifest']['path']).read_text())
            objects = [a for a in manifest['artifacts'] if a['relative_path'] == 'compiler-object.o']
            self.assertEqual(len(objects), 0 if explicit else 1)
            if objects:
                self.assertEqual(objects[0]['elf_type'], 'relocatable')
            else:
                self.assertGreaterEqual(receipt['recoverable_bytes'], obj.stat().st_size)

    def test_actual_shared_library_and_exec_survive_runnable_policy(self):
        code = self.root / 'fixture.c'
        code.write_text('int fixture(void) { return 37; } int main(void) { return 0; }\n')
        library = self.root / 'fixture.so'
        executable = self.root / 'fixture-exec'
        subprocess.run(['cc', '-fPIC', '-shared', str(code), '-o', str(library)], check=True)
        subprocess.run(['cc', '-no-pie', str(code), '-o', str(executable)], check=True)
        args = self.policy_invocation()
        args[-1] += f"; shutil.copy2({str(library)!r},p/'library'); shutil.copy2({str(executable)!r},p/'executable')"
        receipt = self.invoke(args)
        rows = json.loads(pathlib.Path(receipt['artifact_manifest']['path']).read_text())['artifacts']
        shared = next(a for a in rows if a['relative_path'] == 'library')
        executable_row = next(a for a in rows if a['relative_path'] == 'executable')
        self.assertEqual(shared['elf_type'], 'shared-object')
        self.assertEqual(executable_row['elf_type'], 'executable')
        self.assertEqual(subprocess.run([executable_row['blob']], check=False).returncode, 0)
        result = subprocess.run([sys.executable, '-c', 'import ctypes,sys; assert ctypes.CDLL(sys.argv[1]).fixture()==37', shared['blob']], check=False)
        self.assertEqual(result.returncode, 0)

    def test_unknown_elf_bytes_remain_protected(self):
        args = self.policy_invocation()
        args[-1] += "; (p/'unknown.o').write_bytes(bytes([127,69,76,70,0,0,0]))"
        receipt = self.invoke(args)
        rows = json.loads(pathlib.Path(receipt['artifact_manifest']['path']).read_text())['artifacts']
        self.assertEqual(next(a for a in rows if a['relative_path']=='unknown.o')['elf_type'], 'unknown')

    def test_hardlink_pair_reuses_verified_frozen_baseline(self):
        before = self.invoke(self.policy_invocation())
        after = self.invoke(self.policy_invocation('after', before))
        self.assertEqual(after['copied_unique_bytes'], 0)
        self.assertEqual(before['profile'], after['profile'])
        self.assertEqual(after['baseline']['sha256'], before['artifact_manifest']['sha256'])

    def plan(self, budget=1048576):
        self.job += 1
        plan = {'schema':1, 'ticket':'native-plan-controls', 'source':SOURCE, 'phase':'before',
                'job':'plan-'+str(self.job), 'scratch_parent':str(self.scratch),
                'output_root':str(self.out), 'retained_root':str(self.retained),
                'build_budget_bytes':budget, 'retention_budget_bytes':budget,
                'bootstrap_budget_bytes':budget, 'reserve_bytes':4096,
                'retention_method':'hardlink', 'elf_policy':'runnable'}
        file = self.root / ('plan-'+str(self.job)+'.json')
        file.write_text(json.dumps(plan))
        return file, plan

    def plan_args(self, mode, file):
        return [sys.executable, str(HELPER.with_name('native-storage-plan.py')), mode, '--plan', str(file)]

    def test_plan_capacity_counts_simultaneous_bootstrap_build_and_retention(self):
        free = os.statvfs(ROOT).f_bavail * os.statvfs(ROOT).f_frsize
        file, plan = self.plan(free * 2 // 5)
        receipt = self.invoke(self.plan_args('check', file), 3)
        requirement = receipt['capacity']['shared_device_requirements'][0]['required_bytes']
        self.assertEqual(requirement, 3 * plan['build_budget_bytes'] + 4096)
        self.assertFalse(self.scratch.exists())

    def test_plan_prebootstrap_refusal_prevents_actual_next_shell_stage(self):
        free = os.statvfs(ROOT).f_bavail * os.statvfs(ROOT).f_frsize
        file, _ = self.plan(free * 2)
        marker = self.root / 'bootstrap-started'
        import shlex
        check = ' '.join(shlex.quote(a) for a in self.plan_args('check', file))
        command = check + ' || exit 3\n touch ' + shlex.quote(str(marker))
        result = subprocess.run(['bash', '-c', command], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 3)
        self.assertFalse(marker.exists())
        self.assertFalse(self.scratch.exists())

    def test_plan_actual_command_receives_fresh_target_and_profiles(self):
        file, plan = self.plan()
        plan['summary_path'] = str(self.out/'compact'/'job.summary.json')
        file.write_text(json.dumps(plan))
        self.invoke(self.plan_args('check', file))
        command = self.root / 'command.sh'
        command.write_text('cp /usr/bin/true "$CARGO_TARGET_DIR/measured"\n' +
                           'test "$CARGO_INCREMENTAL" = 0 && test "$CARGO_PROFILE_TEST_DEBUG" = line-tables-only\n')
        receipt = self.invoke(self.plan_args('run', file) + ['--command-file',str(command)])
        self.assertTrue(receipt['target_retired'])
        self.assertIn('/before/plan-', receipt['target'])
        summary=json.loads(pathlib.Path(plan['summary_path']).read_text())
        self.assertEqual(summary['actual_storage_exit'],0)
        self.assertTrue(summary['target_retired'])
        self.assertEqual(summary['session_raw']['sha256'],sha(pathlib.Path(receipt['session'])/'session.json'))

    def test_plan_rejects_missing_after_baseline_before_bootstrap(self):
        file, plan = self.plan()
        plan['phase'] = 'after'
        file.write_text(json.dumps(plan))
        rejected = self.invoke(self.plan_args('check',file),3)
        self.assertIn('exact baseline',rejected['error'])
        self.assertFalse(self.scratch.exists())

    def test_plan_rejects_ambiguous_or_unknown_capacity_inputs(self):
        file, plan = self.plan()
        plan['bootstrap_budget_bytes'] = True
        file.write_text(json.dumps(plan))
        self.assertIn('positive byte budgets',self.invoke(self.plan_args('validate',file),3)['error'])
        plan['bootstrap_budget_bytes'] = 1048576
        plan['implicit_cleanup'] = True
        file.write_text(json.dumps(plan))
        self.assertIn('fields/schema',self.invoke(self.plan_args('validate',file),3)['error'])

    def test_plan_after_runs_with_exact_baseline_and_separate_phase_target(self):
        file, plan = self.plan()
        command = self.root / 'pair.sh'
        command.write_text('cp /usr/bin/true "$CARGO_TARGET_DIR/measured"\n')
        before = self.invoke(self.plan_args('run',file)+['--command-file',str(command)])
        after_file, after_plan = self.plan()
        after_plan.update(phase='after',baseline_source=SOURCE,baseline_manifest=before['artifact_manifest']['path'])
        after_file.write_text(json.dumps(after_plan))
        self.invoke(self.plan_args('check',after_file))
        after = self.invoke(self.plan_args('run',after_file)+['--command-file',str(command)])
        self.assertNotEqual(before['target'],after['target'])
        self.assertEqual(after['baseline']['sha256'],before['artifact_manifest']['sha256'])
        self.assertEqual(after['copied_unique_bytes'],0)

    def test_reused_job_is_refused_before_bootstrap(self):
        file, _ = self.plan()
        command = self.root / 'job.sh'
        command.write_text('cp /usr/bin/true "$CARGO_TARGET_DIR/measured"\n')
        self.invoke(self.plan_args('run',file)+['--command-file',str(command)])
        refused = self.invoke(self.plan_args('check',file),3)
        self.assertIn('already exists',refused['error'])

    def test_hardlink_policy_refuses_distinct_native_filesystem_before_target(self):
        distinct = pathlib.Path(tempfile.mkdtemp(prefix='native-retention-cross-device.',dir='/dev/shm'))
        self.assertNotEqual(distinct.stat().st_dev, self.root.stat().st_dev)
        args = self.policy_invocation()
        args[args.index('--output-root')+1]=str(distinct/'out')
        args[args.index('--retained-root')+1]=str(distinct/'out'/'retained')
        refused = self.invoke(args,3)
        self.assertIn('same actual filesystem',refused['error'])
        self.assertFalse(self.scratch.exists())
        self.assertFalse((distinct/'out').exists())
        distinct.rmdir()

    def test_native_retention_and_separate_compact_device_preserve_raw_home(self):
        compact = pathlib.Path(tempfile.mkdtemp(prefix='native-compact-device.',dir='/dev/shm'))
        self.assertNotEqual(compact.stat().st_dev,self.root.stat().st_dev)
        raw = compact/'existing-raw.log'
        raw.write_bytes(b'raw custody intact')
        original = sha(raw)
        file, plan = self.plan()
        plan.update(compact_root=str(compact),summary_path=str(compact/'phase.summary.json'))
        file.write_text(json.dumps(plan))
        self.invoke(self.plan_args('check',file))
        command = self.root/'native.sh'
        command.write_text('cp /usr/bin/true "$CARGO_TARGET_DIR/measured"\n')
        result = self.invoke(self.plan_args('run',file)+['--command-file',str(command)])
        summary = json.loads((compact/'phase.summary.json').read_text())
        self.assertEqual(summary['actual_storage_exit'],0)
        self.assertTrue(result['target_retired'])
        self.assertEqual(sha(raw),original)
        rows=json.loads(pathlib.Path(result['artifact_manifest']['path']).read_text())['artifacts']
        self.assertEqual(rows[0]['retention_method'],'hardlink')
        self.assertEqual(subprocess.run([rows[0]['blob']],check=False).returncode,0)
        raw.unlink(); (compact/'phase.summary.json').unlink(); compact.rmdir()

    def test_published_wrapper_storage_admission_precedes_cargo_bootstrap(self):
        wrapper = HELPER.with_name('linux-run.sh').read_text()
        remote = wrapper[wrapper.index('  REMOTE="trap'):]
        self.assertLess(remote.index('$STORAGE_PRELUDE'),remote.index('bootstrap_native_cargo'))
        self.assertLess(remote.index('bootstrap_native_cargo'),remote.index('$STORAGE_COMMAND'))
        self.assertIn('CARGO_PROFILE_DEV_DEBUG',wrapper)
        self.assertIn('CARGO_PROFILE_TEST_DEBUG',wrapper)
        subprocess.run(['bash','-n',str(HELPER.with_name('linux-run.sh'))],check=True)


    def replay_saved_elf(self, path):
        # /dev/shm may be noexec. Verify and execute the retained native bytes
        # from this test's actual executable /tmp filesystem, without claiming
        # that the storage mount itself permits execution.
        import shutil
        replay = self.root / 'replay-retained-elf'
        shutil.copy2(path, replay)
        self.assertEqual(sha(replay), sha(path))
        self.assertEqual(subprocess.run([str(replay)], check=False).returncode, 0)

    def cross_device(self):
        destination = pathlib.Path(tempfile.mkdtemp(prefix='capability-copy-controls.', dir='/dev/shm'))
        self.assertNotEqual(destination.stat().st_dev, self.root.stat().st_dev)
        return destination

    def fallback_invocation(self, destination, budget=1048576, retire=False):
        args = self.invocation()
        args[args.index('--output-root') + 1] = str(destination / 'out')
        args[args.index('--retained-root') + 1] = str(destination / 'out' / 'retained')
        args[args.index('--retention-budget-bytes') + 1] = str(budget)
        index = args.index('--')
        flags = ['--retention-method', 'hardlink-or-copy', '--elf-policy', 'runnable']
        if retire:
            flags += ['--retire-after-retention']
        return args[:index] + flags + args[index:]

    def test_real_exdev_probe_never_changes_original_elf(self):
        import errno
        spec = importlib.util.spec_from_file_location('capability_probe', HELPER)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        source = self.root / 'original-elf'
        source.write_bytes(ELF.read_bytes())
        destination = self.cross_device()
        inode, original = source.stat().st_ino, sha(source)
        result = module.probe_hardlink(source, destination)
        self.assertFalse(result['supported'])
        self.assertEqual(result['errno'], errno.EXDEV)
        self.assertEqual(source.stat().st_ino, inode)
        self.assertEqual(sha(source), original)
        self.assertFalse(list(destination.iterdir()))

    def test_budgeted_actual_exdev_copy_preserves_source_target_by_default(self):
        receipt = self.invoke(self.fallback_invocation(self.cross_device()))
        self.assertFalse(receipt['hardlink_capability']['supported'])
        self.assertEqual(receipt['selected_retention_method'], 'copy')
        self.assertGreater(receipt['copied_unique_bytes'], 0)
        self.assertFalse(receipt['target_retired'])
        original = pathlib.Path(receipt['target']) / 'measured'
        self.assertEqual(sha(original), sha(ELF))
        rows = json.loads(pathlib.Path(receipt['artifact_manifest']['path']).read_text())['artifacts']
        saved = next(row for row in rows if row['kind'] == 'elf')
        self.assertEqual(saved['retention_method'], 'copy')
        self.assertEqual(sha(pathlib.Path(saved['blob'])), sha(original))
        self.replay_saved_elf(pathlib.Path(saved['blob']))

    def test_actual_exdev_small_copy_budget_refuses_payload_and_retirement(self):
        destination = self.cross_device()
        receipt = self.invoke(self.fallback_invocation(destination, budget=1), 3)
        self.assertIn('explicit retention budget', receipt['storage_error'])
        self.assertFalse(receipt['retention_complete'])
        self.assertFalse(receipt.get('target_retired', False))
        self.assertEqual(sha(pathlib.Path(receipt['target']) / 'measured'), sha(ELF))
        blobs = pathlib.Path(receipt['session']).parent  # Source retained, no published payload.
        self.assertTrue(blobs.exists())
        self.assertFalse(list((destination / 'out' / 'retained' / 'blobs').glob('*')))

    def test_copy_fallback_explicit_retirement_preserves_verified_original_elf(self):
        receipt = self.invoke(self.fallback_invocation(self.cross_device(), retire=True))
        self.assertTrue(receipt['retire_after_retention'])
        self.assertTrue(receipt['target_retired'])
        self.assertFalse(pathlib.Path(receipt['target']).exists())
        rows = json.loads(pathlib.Path(receipt['artifact_manifest']['path']).read_text())['artifacts']
        saved = next(row for row in rows if row['kind'] == 'elf')
        self.assertEqual(sha(pathlib.Path(saved['blob'])), sha(ELF))
        self.replay_saved_elf(pathlib.Path(saved['blob']))

    def test_fallback_uses_real_hardlink_when_available_without_implicit_retirement(self):
        args = self.invocation(); index = args.index('--')
        args = args[:index] + ['--retention-method', 'hardlink-or-copy'] + args[index:]
        receipt = self.invoke(args)
        self.assertTrue(receipt['hardlink_capability']['supported'])
        self.assertEqual(receipt['selected_retention_method'], 'hardlink')
        self.assertEqual(receipt['copied_unique_bytes'], 0)
        self.assertFalse(receipt['target_retired'])
        rows = json.loads(pathlib.Path(receipt['artifact_manifest']['path']).read_text())['artifacts']
        elf = next(row for row in rows if row['kind'] == 'elf')
        self.assertEqual(pathlib.Path(elf['blob']).stat().st_ino,
                         (pathlib.Path(receipt['target']) / 'measured').stat().st_ino)

    def raw_args(self, destination, budget=0):
        source = self.root / 'raw'
        source.mkdir(); (source / 'nested').mkdir()
        (source / 'actual.ppm').write_bytes(b'P6\n1 1\n255\n\x00\xff\x7f')
        (source / 'nested' / 'stderr.log').write_bytes(b'original error\x00')
        return [sys.executable, str(HELPER), 'publish-raw', '--directory', str(source),
                '--destination', str(destination), '--copy-budget-bytes', str(budget),
                '--reserve-bytes', '4096'], source

    def test_actual_raw_rename_capability_preserves_bytes_without_a_copy(self):
        destination = self.root / 'published'
        args, source = self.raw_args(destination)
        original = sha(source / 'actual.ppm')
        receipt = self.invoke(args)
        self.assertEqual(receipt['method'], 'rename')
        self.assertTrue(receipt['complete'])
        self.assertFalse(source.exists())
        self.assertEqual(sha(destination / 'actual.ppm'), original)

    def test_actual_raw_exdev_zero_copy_budget_preserves_original_and_rejects(self):
        destination = self.cross_device() / 'published'
        args, source = self.raw_args(destination)
        original = sha(source / 'actual.ppm')
        refused = self.invoke(args, 3)
        self.assertIn('explicit sufficient copy budget', refused['error'])
        self.assertFalse(destination.exists())
        self.assertEqual(sha(source / 'actual.ppm'), original)

    def test_actual_raw_exdev_budgeted_copy_verifies_all_files_and_keeps_source(self):
        destination = self.cross_device() / 'published'
        args, source = self.raw_args(destination, 1048576)
        receipt = self.invoke(args)
        self.assertEqual(receipt['rename_errno'], 18)
        self.assertEqual(receipt['method'], 'copy')
        self.assertTrue(receipt['source_retained'])
        self.assertTrue(receipt['complete'])
        for relative in ['actual.ppm', 'nested/stderr.log']:
            self.assertEqual(sha(source / relative), sha(destination / relative))
        self.assertEqual(receipt['raw_files'], 2)

    def test_raw_existing_destination_is_never_overwritten(self):
        destination = self.root / 'published'; destination.mkdir()
        (destination / 'old-proof').write_bytes(b'old intact')
        args, source = self.raw_args(destination, 1048576)
        refused = self.invoke(args, 3)
        self.assertIn('fresh destination', refused['error'])
        self.assertTrue(source.exists())
        self.assertEqual((destination / 'old-proof').read_bytes(), b'old intact')

    def test_plan_explicit_fallback_and_retirement_are_forwarded_without_fake_baseline(self):
        file, plan = self.plan()
        destination = self.cross_device()
        plan.update(output_root=str(destination / 'out'),retained_root=str(destination / 'out' / 'retained'),
                    retention_method='hardlink-or-copy',retire_after_retention=True)
        file.write_text(json.dumps(plan))
        self.invoke(self.plan_args('check', file))
        command = self.root / 'fallback.sh'
        command.write_text('cp /usr/bin/true "$CARGO_TARGET_DIR/measured"\n')
        receipt = self.invoke(self.plan_args('run', file) + ['--command-file', str(command)])
        self.assertEqual(receipt['selected_retention_method'], 'copy')
        self.assertTrue(receipt['target_retired'])
        for wrong in [dict(retention_method='copy', retire_after_retention=False),
                      dict(retention_method='hardlink-or-copy', retire_after_retention='false')]:
            invalid = dict(plan, **wrong)
            file.write_text(json.dumps(invalid))
            refused = self.invoke(self.plan_args('validate', file), 3)
            self.assertIn('retire_after_retention', refused['error'])


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
           'helper_sha256': sha(HELPER), 'plan_sha256': sha(HELPER.with_name('native-storage-plan.py')), 'wrapper_sha256':sha(HELPER.with_name('linux-run.sh')), 'controls_sha256': sha(pathlib.Path(__file__)),
           'native_elf': {'path': str(ELF), 'bytes': ELF.stat().st_size, 'sha256': sha(ELF)},
           'fixture_root': str(ROOT), 'engine_build_or_gate': False}
summary['diagnostics'] = {test._testMethodName: '\n'.join(trace.splitlines()[-24:])[-4096:]
                          for test, trace in result.failures + result.errors}
output.write_text(json.dumps(summary, sort_keys=True, indent=2) + '\n')
print(json.dumps(summary, separators=(',', ':')))
sys.exit(0 if result.wasSuccessful() else 1)
