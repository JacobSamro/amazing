#!/usr/bin/env python3
"""Whole native admission controls; fixtures do not execute browser cases."""
import base64
import copy
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
import uuid
import zlib

spec = importlib.util.spec_from_file_location('diagnostic', Path(__file__).with_name('structured-wpt-diagnostic.py'))
D = importlib.util.module_from_spec(spec)
spec.loader.exec_module(D)
SOURCE = 'a' * 40
JOB = 'b' * 32
CASE = 'custom-elements/registries/define.html'


class Controls(unittest.TestCase):
    def setUp(self):
        self.native = tempfile.TemporaryDirectory(prefix='structured-wpt-', dir='/tmp')
        self.raw = Path(self.native.name)
        self.compact = Path.home() / 'amazing/out/structured-wpt-controls' / uuid.uuid4().hex
        self.compact.mkdir(parents=True)
        self.record_path = self.raw / 'own-wpt/00000.record.json'
        self.record_path.parent.mkdir()
        self.capsule_path = self.compact / 'wpt.bundle.manifest.json'
        self.comparison_path = self.compact / 'comparison.summary.json'
        self.summary_path = self.compact / 'gate.manifest.json'
        self.terminal_path = self.raw / 'completion.json'
        sub = {'name': 'actual assertion', 'status': 1,
               'message': 'assert_throws_js: actual object mismatch', 'stack': 'original source:12:3'}
        self.record = {'case': CASE, 'source': SOURCE, 'binary_sha256': 'c' * 64,
                       'argv': [str(self.raw / 'target/release/amazing'), 'wpt-harness', '/tmp/pinned-wpt', CASE, '800', '600', '20000'],
                       'actual_exit': 1, 'worker_timed_out': False,
                       'report': {'test': CASE, 'wpt_revision': 'd' * 40, 'status': 'fail',
                                  'harness_status': 0, 'subtests': [sub]}}
        self.capsule = {'source': SOURCE, 'role': 'after', 'binary_sha256': 'c' * 64,
                        'pin': 'd' * 40, 'viewport': [800, 600], 'timeout_ms': 20000,
                        'cases': [[CASE, 'e' * 64]], 'records': [{'file': CASE, 'status': 'fail',
                            'native_status': 'fail', 'actual_exit': 1, 'harness_status': 0,
                            'subtests': [{'name': sub['name'], 'native_status': 1}], 'raw': {}}]}
        self.phase = {'phase': 'own-pair', 'source_commit': SOURCE, 'command_exit': 1,
                      'engine_exit': 1, 'wrapper_exit': 1,
                      'command': ['python3', '/tmp/pair.py', str(self.raw / 'original-own/wpt.bundle.manifest.json'),
                                  str(self.capsule_path), str(self.comparison_path)],
                      'raw_native': {'path': str(self.raw / 'pair.log')}}
        self.terminal = {'schema': 2, 'lane': 'structured-wpt-controls', 'source_commit': SOURCE,
                         'box_id': 'bx_controls', 'host': 'user@127.0.0.1', 'port': 22, 'job_id': JOB,
                         'output_root': self.compact.relative_to(Path.home()).as_posix(),
                         'command_sha256': 'f' * 64, 'execution_sha256': 'f' * 64,
                         'source_archive_sha256': 'f' * 64, 'boot_id': '11111111-2222-3333-4444-555555555555',
                         'completed': True, 'actual_exit': 100, 'runner_exit': 100,
                         'command_returncode': 100, 'catalog_error': None}
        self.publish()

    def tearDown(self):
        shutil.rmtree(self.compact)
        self.native.cleanup()

    def write(self, path, row):
        body = (json.dumps(row, ensure_ascii=True, separators=(',', ':')) + '\n').encode()
        path.write_bytes(body)
        return D.gate.digest(body)

    def publish(self):
        sha = self.write(self.record_path, self.record)
        self.capsule['records'][0]['raw']['record'] = {'path': str(self.record_path), 'bytes': self.record_path.stat().st_size, 'sha256': sha}
        self.capsule['input_sha256'] = D.gate.digest(json.dumps(self.capsule['cases'], separators=(',', ':')).encode())
        body = json.dumps(self.capsule, ensure_ascii=True, separators=(',', ':')).encode()
        text = base64.b64encode(zlib.compress(body)).decode()
        part_path = self.compact / 'wpt.bundle.manifest.json.part-0.compact.json'
        part_sha = self.write(part_path, {'schema': 1, 'part': 0, 'data': text})
        capsule_sha = self.write(self.capsule_path, {'schema': 1, 'encoding': 'base64-zlib-json',
            'parts': [{'name': part_path.name, 'bytes': part_path.stat().st_size, 'sha256': part_sha}],
            'uncompressed_bytes': len(body), 'uncompressed_sha256': D.gate.digest(body)})
        comparison_sha = self.write(self.comparison_path, {'source_after': SOURCE, 'zero_loss': False,
            'complete_comparable_measurements': True, 'files': len(self.capsule['cases']),
            'after_manifest': {'path': str(self.capsule_path), 'sha256': capsule_sha}})
        summary_sha = self.write(self.summary_path, {'source_commit': SOURCE, 'job_exit': 100, 'phase_details': [self.phase]})
        self.terminal['artifacts'] = {p.relative_to(Path.home()).as_posix(): {'bytes': p.stat().st_size, 'sha256': sha}
            for p, sha in [(self.summary_path, summary_sha), (self.comparison_path, comparison_sha), (self.capsule_path, capsule_sha)]}
        self.terminal_sha = self.write(self.terminal_path, self.terminal)
        self.args = [self.terminal_path, self.terminal_sha, self.summary_path, summary_sha,
                     self.comparison_path, comparison_sha, self.capsule_path, capsule_sha, SOURCE, JOB, CASE]

    def result(self):
        return json.loads(D.produce(*self.args))

    def reject(self):
        with self.assertRaises((ValueError, KeyError, TypeError, FileNotFoundError)):
            self.result()

    def test_actual_message_stack_and_no_reconstructed_counts(self):
        result = self.result()
        self.assertEqual(result['failures'][0]['message'], self.record['report']['subtests'][0]['message'])
        self.assertEqual(result['failures'][0]['stack'], 'original source:12:3')
        self.assertFalse(result['tests_rerun']); self.assertFalse(result['counts_reconstructed'])

    def test_retired_binary_path_is_metadata_not_fake_readback(self):
        self.assertFalse(Path(self.record['argv'][0]).exists())
        self.assertEqual(self.result()['binary_sha256'], 'c' * 64)

    def test_wrong_source_and_job_rejected(self):
        for index, value in [(8, 'f' * 40), (9, 'f' * 32)]:
            original = self.args[index]; self.args[index] = value; self.reject(); self.args[index] = original

    def test_wrong_marker_exit_or_live_marker_rejected(self):
        for key, value in [('actual_exit', 0), ('runner_exit', 3), ('completed', False)]:
            old = self.terminal[key]; self.terminal[key] = value; self.publish(); self.reject(); self.terminal[key] = old

    def test_uncatalogued_compact_rejected(self):
        self.terminal['artifacts'].pop(self.capsule_path.relative_to(Path.home()).as_posix())
        self.args[1] = self.write(self.terminal_path, self.terminal); self.reject()

    def test_comparison_not_complete_or_not_failure_rejected(self):
        for key, value in [('zero_loss', True), ('complete_comparable_measurements', False)]:
            row = json.loads(self.comparison_path.read_text()); row[key] = value
            self.args[5] = self.write(self.comparison_path, row)
            self.terminal['artifacts'][self.comparison_path.relative_to(Path.home()).as_posix()]['sha256'] = self.args[5]
            self.args[1] = self.write(self.terminal_path, self.terminal); self.reject(); self.publish()

    def test_actual_phase_exit_and_duplicate_receipt_rejected(self):
        self.phase['command_exit'] = 0; self.publish(); self.reject()
        self.phase['command_exit'] = 1; self.publish()
        row = json.loads(self.summary_path.read_text()); row['phase_details'].append(copy.deepcopy(self.phase))
        self.args[3] = self.write(self.summary_path, row)
        self.terminal['artifacts'][self.summary_path.relative_to(Path.home()).as_posix()]['sha256'] = self.args[3]
        self.args[1] = self.write(self.terminal_path, self.terminal); self.reject()

    def test_captured_operands_not_substituted(self):
        self.phase['command'][3] = '/tmp/foreign.json'; self.publish(); self.reject()

    def test_capsule_hash_and_part_bytes_enforced(self):
        self.capsule_path.write_bytes(self.capsule_path.read_bytes() + b' '); self.reject()
        self.publish(); part = self.compact / 'wpt.bundle.manifest.json.part-0.compact.json'
        part.write_bytes(part.read_bytes() + b' '); self.reject()

    def test_duplicate_case_and_wrong_inventory_hash_rejected(self):
        self.capsule['cases'].append(copy.deepcopy(self.capsule['cases'][0])); self.publish(); self.reject()

    def test_foreign_case_or_source_record_rejected(self):
        for key, value in [('source', 'f' * 40), ('case', 'dom/foreign.html')]:
            old = self.record[key]; self.record[key] = value; self.publish(); self.reject(); self.record[key] = old

    def test_original_binary_argv_pin_fenced(self):
        for mutate in [lambda: self.record.update(binary_sha256='f' * 64),
                       lambda: self.record['argv'].__setitem__(3, 'dom/foreign.html'),
                       lambda: self.record['report'].update(wpt_revision='f' * 40)]:
            old = copy.deepcopy(self.record); mutate(); self.publish(); self.reject(); self.record = old

    def test_record_size_and_hash_fenced(self):
        self.record_path.write_bytes(self.record_path.read_bytes() + b' '); self.reject()

    def test_symlink_record_rejected(self):
        other = self.raw / 'other.json'; other.write_bytes(self.record_path.read_bytes())
        self.record_path.unlink(); self.record_path.symlink_to(other); self.reject()

    def test_native_subtest_roster_and_status_rejected(self):
        self.capsule['records'][0]['subtests'][0]['native_status'] = 0; self.publish(); self.reject()

    def test_pass_case_is_not_failure_diagnostic(self):
        self.capsule['records'][0]['status'] = 'pass'; self.publish(); self.reject()

    def test_lone_surrogate_units_and_explicit_truncation(self):
        self.record['report']['subtests'][0]['message'] = '\ud800' * 10000
        self.publish(); result = self.result()
        self.assertIn('message', result['failures'][0]['truncated_fields'])
        self.assertTrue(result['failures'][0]['message'].startswith('\ud800'))
        self.assertLessEqual(len(json.dumps(result, ensure_ascii=True).encode()), D.MAX_OUTPUT)

    def test_partial_native_timeout_does_not_invent_assertion(self):
        self.record['report']['status'] = 'timeout'; self.record['report']['harness_status'] = None
        self.record['report']['subtests'][0]['status'] = 0
        row = self.capsule['records'][0]; row.update(status='timeout', native_status='timeout', harness_status=None)
        row['subtests'][0]['native_status'] = 0
        self.publish(); result = self.result()
        self.assertEqual(result['failures'], []); self.assertFalse(result['worker_timed_out'])


if __name__ == '__main__':
    import sys
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Controls)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if len(sys.argv) > 1:
        Path(sys.argv[1]).write_text(json.dumps({'controls_run': result.testsRun,
            'failures': len(result.failures), 'errors': len(result.errors),
            'browser_cases_executed': False}, sort_keys=True) + '\n')
    raise SystemExit(0 if result.wasSuccessful() else 1)
