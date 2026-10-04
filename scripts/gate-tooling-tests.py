#!/usr/bin/env python3
"""Metadata-only tooling checks; no engine build, corpus or Boat calls."""
import contextlib
import importlib.util
import io
import json
import multiprocessing
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

SCRIPTS = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('boat_lane', SCRIPTS / 'boat-lane.py')
boat_lane = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boat_lane)


def write_rows(path, lane):
    boat_lane.RUNS = pathlib.Path(path)
    for i in range(40):
        boat_lane.append_run(100 + i, 101 + i, lane, 'pool-test', i % 2)


class Reporting(unittest.TestCase):
    def test_malformed_rows_and_summary(self):
        valid = ['100 120 css pool-test 0', '110 130 layout pool-test 101',
                 '140 150 html pool-test']
        invalid = ['linux-run[t262-gate6-2]: sandbox bx pool-test',
                   '100 120 css pool-test 0 noisy', 'nan 120 css pool-test 0',
                   '100 inf css pool-test 0', '120 100 css pool-test 0',
                   '100 120 css pool-test NaN', '100 120 css pool-test 256',
                   '100 120 css pool_test 0']
        for line in valid:
            self.assertIsNotNone(boat_lane.parse_run(line))
        for line in invalid:
            self.assertIsNone(boat_lane.parse_run(line))
        with tempfile.TemporaryDirectory() as d:
            path = pathlib.Path(d) / 'runs'
            path.write_text('\n'.join(valid + invalid))
            output, errors = io.StringIO(), io.StringIO()
            with mock.patch.object(boat_lane, 'RUNS', path), mock.patch.object(boat_lane.time, 'time', return_value=200), contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
                boat_lane.pool_report(100 / 3600)
            row = output.getvalue().splitlines()[1].split()
            self.assertEqual(row[:6], ['pool-test', '3', '40%', '10%', '2', '17'])
            self.assertIn('ignored 8 malformed', errors.getvalue())

    def test_concurrent_writes_keep_complete_rows(self):
        with tempfile.TemporaryDirectory() as d:
            path = pathlib.Path(d) / 'runs'
            path.write_text('original partial row')
            processes = [multiprocessing.Process(target=write_rows, args=(str(path), f'lane-{n}')) for n in range(4)]
            for process in processes: process.start()
            for process in processes:
                process.join(10)
                self.assertEqual(process.exitcode, 0)
            lines = path.read_text().splitlines()
            self.assertEqual(lines[0], 'original partial row')
            self.assertEqual(len(lines), 161)
            self.assertTrue(all(boat_lane.parse_run(line) for line in lines[1:]))
            self.assertEqual({boat_lane.parse_run(line)[2] for line in lines[1:]}, {f'lane-{n}' for n in range(4)})


class SetupReceipt(unittest.TestCase):
    def test_phase_exit_is_distinct_from_wrapper_exit(self):
        with tempfile.TemporaryDirectory() as d:
            path = pathlib.Path(d) / 'setup.manifest.json'
            subprocess.check_call([sys.executable, str(SCRIPTS / 'gate-summary.py'),
                'failure', '--source-commit', 'fixture-source', '--shard', 'fixture-lane',
                '--wrapper-exit', '3', str(path), 'rsync', '12'])
            receipt = json.loads(path.read_text())
            self.assertEqual(receipt['actual_exit'], 12)
            self.assertEqual(receipt['wrapper_exit'], 3)
            self.assertEqual(receipt['phase'], 'rsync')
            self.assertEqual(receipt['raw'], [])


class CommandDiagnostics(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('gate_command', SCRIPTS / 'gate-command.py')
        cls.helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.helper)

    def test_failing_command_preserves_actual_exit_and_tail(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            result = self.helper.run([sys.executable, '-c', 'print("actionable failure"); raise SystemExit(37)'],
                                     'build', 'fixture', root / 'log', root / 'phase.manifest.json')
            self.assertEqual(result['command_exit'], 37)
            self.assertEqual(result['engine_exit'], 37)
            self.assertEqual(result['wrapper_exit'], 37)
            self.assertEqual(result['output_copy_exit'], 0)
            self.assertIn('actionable failure', result['diagnostic_tail'])
            self.assertEqual(json.loads((root / 'phase.manifest.json').read_text())['engine_exit'], 37)

    def test_missing_and_truncated_copy_are_failures(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d); source = root / 'source'; destination = root / 'dest'
            missing = self.helper.copy_output(source, destination)
            self.assertEqual(missing['copy_exit'], 1)
            self.assertEqual(missing['error']['errno'], 2)
            source.write_bytes(b'complete output')
            def truncate(a, b): pathlib.Path(b).write_bytes(b'partial')
            with mock.patch.object(self.helper.shutil, 'copyfile', side_effect=truncate):
                truncated = self.helper.copy_output(source, destination)
            self.assertEqual(truncated['copy_exit'], 1)
            self.assertNotEqual(truncated['native']['sha256'], truncated['durable']['sha256'])

    def test_copy_failure_does_not_rewrite_engine_exit(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            for code in (0, 37):
                result = self.helper.run([sys.executable, '-c', f'print("output"); raise SystemExit({code})'],
                                         'runner', 'fixture', root / 'missing-parent' / 'log', root / 'phase.manifest.json')
                self.assertEqual(result['engine_exit'], code)
                self.assertEqual(result['wrapper_exit'], code or 3)
                self.assertEqual(result['output_copy_exit'], 1)
                self.assertTrue(pathlib.Path(result['raw_native']['path']).is_file())

    def test_cli_keeps_literal_failure_exit(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            process = subprocess.run([sys.executable, str(SCRIPTS / 'gate-command.py'),
                '--phase', 'runner', '--source-commit', 'fixture', '--output', str(root / 'log'),
                '--receipt', str(root / 'phase.manifest.json'), '--', sys.executable, '-c',
                'raise SystemExit(37)'], capture_output=True, text=True)
            self.assertEqual(process.returncode, 37)
            self.assertEqual(json.loads(process.stdout)['engine_exit'], 37)

    def test_signal_return_is_preserved_separately_from_shell_exit(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            result = self.helper.run([sys.executable, '-c', 'import os,signal; os.kill(os.getpid(),signal.SIGTERM)'],
                                     'runner', 'fixture', root / 'log', root / 'phase.manifest.json')
            self.assertEqual(result['command_exit'], -15)
            self.assertEqual(result['engine_exit'], 143)
            self.assertEqual(result['wrapper_exit'], 143)


class SourceFilter(unittest.TestCase):
    def test_exact_ref_keeps_includes_fixtures_tsv_and_source(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d); repo = root / 'repo'; out = root / 'out'
            repo.mkdir(); out.mkdir()
            big = b'x' * (512 * 1024 + 1)
            files = {
                'src/lib.rs': b'const A: &str = include_str!("../evidence/used.jsonl");\nconst B: &[u8] = include_bytes!( "../evidence/large-used.json" );',
                'evidence/used.jsonl': b'literal include',
                'evidence/large-used.json': big,
                'evidence/raw.jsonl': b'historical raw',
                'evidence/oversized.json': big,
                'docs/lanes/oversized.json': big,
                'evidence/input.tsv': big,
                'evidence/fixtures/input.jsonl': b'fixture',
                'evidence/testdata/input.json': big,
                'evidence/tests/input.jsonl': b'test input',
                'evidence/compact.json': b'{}',
                'src/input.jsonl': b'source-owned input',
            }
            for name, data in files.items():
                path = repo / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(data)
            def git(*args):
                return subprocess.check_output(['git', '-C', str(repo), *args], stderr=subprocess.PIPE)
            git('init', '-q'); git('add', '.')
            git('-c', 'user.name=Tooling Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'source fixture')
            commit = git('rev-parse', 'HEAD').decode().strip()
            facts = json.loads(subprocess.check_output([sys.executable, str(SCRIPTS / 'gate-source.py'), str(repo), commit, str(out)]))
            omitted = {'evidence/raw.jsonl', 'evidence/oversized.json', 'docs/lanes/oversized.json'}
            self.assertEqual(facts['omitted_raw_files'], len(omitted))
            for name, data in files.items():
                if name in omitted: self.assertFalse((out / name).exists(), name)
                else: self.assertEqual((out / name).read_bytes(), data, name)
            self.assertEqual(facts['retained_literal_includes'], 2)
            self.assertTrue(facts['tsv_retained'])


if __name__ == '__main__':
    unittest.main()
