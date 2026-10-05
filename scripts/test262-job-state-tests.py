#!/usr/bin/env python3
"""Real native observer/owner controls through the qualified public wait API."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock
import uuid

SCRIPT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('shard_state', SCRIPT / 'test262-job-state.py')
state = importlib.util.module_from_spec(spec)
spec.loader.exec_module(state)
job = state.job


class ShardState(unittest.TestCase):
    def setUp(self):
        root = Path.home() / 'amazing/out/test262-observer-control'
        root.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=root)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.children = []
        self.addCleanup(self.cleanup)

    def cleanup(self):
        for child in self.children:
            if child.poll() is None:
                child.terminate()
            try:
                child.communicate(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.communicate()

    def start(self, code, hold=2):
        row = dict(schema=2, lane='test262-observer-control',
                   source_commit=os.environ.get('COMMIT', 'a' * 40),
                   box_id='bx_ownedcontrol', host='user@127.0.0.1', port=19001,
                   job_id=uuid.uuid4().hex,
                   output_root='amazing/out/test262-observer-control/' + uuid.uuid4().hex,
                   source_archive_sha256='1' * 64)
        root = str(Path.home() / row['output_root'])
        native = ('import pathlib,time,os,sys; p=pathlib.Path(' + repr(root) + '); '
                  'p.mkdir(parents=True); (p/"result.compact.json").write_text("{}\\n"); '
                  f'time.sleep({hold}); os.write(1,b"genuine native output after disconnect\\n"); '
                  f'sys.exit({code})')
        command = 'exec ' + shlex.quote(sys.executable) + ' -I -c ' + shlex.quote(native)
        row['command_sha256'] = row['execution_sha256'] = job.digest(command.encode())
        receipt = self.root / (row['job_id'] + '.endpoint.manifest.json')
        receipt.write_text(json.dumps(row))
        child = subprocess.Popen([sys.executable, '-I', str(SCRIPT / 'completed-job.py'),
                                  'run', job.encoded(row), command],
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.children.append(child)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            observed = job.observe(row)
            if observed['state'] == 'RUNNING' and observed.get('child'):
                return row, receipt, child, observed
            time.sleep(.01)
        self.fail('native owner did not start: ' + repr(observed))

    def resolve(self, row, receipt, observer):
        args = argparse.Namespace(receipt=str(receipt), lane=row['lane'], source=row['source_commit'],
                                  observer_exit=observer, seconds=15,
                                  output=str(self.root / (row['job_id'] + '.native.summary.json')),
                                  questions=str(self.root / (row['job_id'] + '.questions.md')))
        # Same native box: only SSH is elided. Every status/wait response comes
        # from the genuine public process/boot/exit witness, never a state table.
        with mock.patch.object(job, 'remote', side_effect=lambda row, action, **kwargs: job.observe(row)):
            code = state.resolve(args)
        return code, json.loads(Path(args.output).read_text()), args

    def test_closed_real_observer_waits_for_native_zero_and_101(self):
        for expected in (0, 101):
            with self.subTest(native_exit=expected):
                row, receipt, child, running = self.start(expected)
                self.assertIsNone(running['actual_exit'])
                observer = subprocess.Popen([sys.executable, '-I', '-c', 'import sys;sys.exit(3)'],
                                            stdin=child.stdout, stdout=subprocess.DEVNULL,
                                            stderr=subprocess.DEVNULL)
                child.stdout.close()
                child.stdout = None
                observer_exit = observer.wait(timeout=10)
                self.assertEqual(observer_exit, 3)
                code, result, args = self.resolve(row, receipt, observer_exit)
                child.communicate(timeout=10)
                self.assertEqual(code, expected)
                self.assertEqual(child.returncode, expected)
                self.assertEqual(result['state'], 'TERMINAL')
                self.assertEqual(result['actual_exit'], expected)
                self.assertEqual(result['runner_exit'], expected)
                self.assertTrue(result['compact_fetch_allowed'])
                self.assertTrue(result['observer_pipe_closed'])
                self.assertFalse(Path(args.questions).exists())
                verified = job.verify(row, row['lane'], row['source_commit'],
                                      [row['output_root'] + '/result.compact.json'])
                self.assertEqual(verified['actual_exit'], expected)

    def test_absent_actual_owner_without_exit_is_unknown_despite_compact(self):
        row, receipt, child, running = self.start(0, 30)
        child_witness = running['child']
        deadline = time.monotonic() + 2
        compact = Path.home() / row['output_root'] / 'result.compact.json'
        while not compact.exists() and time.monotonic() < deadline:
            time.sleep(.01)
        self.assertTrue(compact.exists())
        child.kill()
        child.wait(timeout=10)
        current = job.proc(child_witness['pid'])
        if current and current['start_ticks'] == child_witness['start_ticks']:
            os.killpg(child_witness['pid'], signal.SIGTERM)
        code, result, args = self.resolve(row, receipt, 3)
        self.assertEqual(code, 3)
        self.assertEqual(result['state'], 'UNKNOWN')
        self.assertIsNone(result['actual_exit'])
        self.assertIsNone(result['runner_exit'])
        self.assertFalse(result['compact_fetch_allowed'])
        self.assertTrue(Path(args.questions).exists())
        with self.assertRaises(FileNotFoundError):
            job.verify(row, row['lane'], row['source_commit'],
                       [row['output_root'] + '/result.compact.json'])

    def test_wrong_source_and_missing_receipt_never_reuse_previous_terminal(self):
        row, receipt, child, running = self.start(0, .5)
        child.communicate(timeout=10)
        code, result, args = self.resolve(row, receipt, 3)
        self.assertEqual(code, 0)
        wrong = dict(row, source_commit='b' * 40)
        code, result, args = self.resolve(wrong, receipt, 3)
        self.assertEqual(code, 3)
        self.assertEqual(result['state'], 'UNKNOWN')
        self.assertIsNone(result['actual_exit'])
        receipt.unlink()
        code, result, args = self.resolve(row, receipt, 3)
        self.assertEqual(code, 3)
        self.assertFalse(result['compact_fetch_allowed'])
        self.assertIsNone(result['actual_exit'])


if __name__ == '__main__':
    unittest.main()
