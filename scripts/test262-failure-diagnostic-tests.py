#!/usr/bin/env python3
"""Whole native tooling controls; fixture reports never qualify an engine."""
import base64
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zlib

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('diagnostic', ROOT / 'test262-failure-diagnostic.py')
D = importlib.util.module_from_spec(spec)
spec.loader.exec_module(D)
SOURCE = '360fb21ebe6b5faa4a6554dec4c01a24e972e983'
JOB = '6a69b6b0e2ed460b8bfdc5f1fb1158ca'
CASE = 'built-ins/TypedArray/prototype/toString.js'


class Controls(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='amazing-test262-diagnostic-controls-', dir='/tmp')
        self.root = Path(self.tmp.name).resolve()
        self.terminal = {'state': 'TERMINAL', 'actual_exit': 0, 'runner_exit': 0,
                         'command_returncode': 0, 'job': {'source_commit': SOURCE, 'job_id': JOB}}
        self.report = {'test262_revision': '3655e7464de3d52643ecddd4b5f9f4f3e7f62398',
                       'input_sha256': '1' * 64, 'host_sha256': '2' * 64,
                       'runner_sha256': '3' * 64, 'shard': '0/1', 'jobs': 4,
                       'filter': 'built-ins/TypedArray/', 'limit': 0, 'timeout_seconds': 60,
                       'records': [{'path': CASE, 'status': 'fail', 'reason': 'host.abnormal-exit',
                                    'variants': [{'variant': name, 'status': 'fail',
                                                  'reason': 'host.abnormal-exit', 'returncode': -6,
                                                  'stderr': 'fixture native panic', 'stdout': ''}
                                                 for name in ('non-strict', 'strict')]},
                                   {'path': 'built-ins/TypedArray/prototype/map/valid.js',
                                    'status': 'pass', 'reason': 'completed',
                                    'variants': [{'variant': 'strict', 'status': 'pass', 'reason': 'completed'}]}]}
        self.phase = {'source_commit': SOURCE, 'phase': 'default-built-ins-TypedArray',
                      'command_exit': 1, 'engine_exit': 1, 'wrapper_exit': 1,
                      'output_copy_exit': 0,
                      'command': ['python3', 'scripts/test262.py', '/tmp/corpus', '--binary', '/tmp/host',
                                  '--output', str(self.root / 'report.json')]}
        self.compact = {'schema': 2, 'source_commit': SOURCE, 'job_exit': 0, 'runner_exit': 1}
        self.write()

    def tearDown(self):
        self.tmp.cleanup()

    def file(self, name, value):
        data = (json.dumps(value, separators=(',', ':')) + '\n').encode()
        path = self.root / name
        path.write_bytes(data)
        return {'path': str(path), 'bytes': len(data), 'sha256': D.sha(data)}

    def write(self):
        stable = [[r['path'], r['status'], r['reason'],
                   [[v['variant'], v['status'], v['reason']] for v in r['variants']]]
                  for r in self.report['records']]
        self.report['classification_sha256'] = D.sha(json.dumps(stable, separators=(',', ':'), ensure_ascii=True).encode())
        self.compact.update({k: self.report[k] for k in
                             ('test262_revision', 'input_sha256', 'host_sha256', 'runner_sha256',
                              'shard', 'timeout_seconds', 'classification_sha256')})
        self.compact['selection'] = {k: self.report[k] for k in ('filter', 'jobs', 'limit')}
        self.compact['raw'] = self.file('report.json', self.report)
        records = [{'path': r['path'], 'status': r['status'],
                    'variants': [{'variant': v['variant'], 'status': v['status']} for v in r['variants']]}
                   for r in self.report['records']]
        self.compact['compact_records'] = base64.b64encode(zlib.compress(json.dumps(records).encode())).decode()
        log = self.root / 'phase.log'; log.write_bytes(b'fixture normal runner exit one\n')
        self.phase['raw_native'] = {'path': str(log), 'bytes': log.stat().st_size, 'sha256': D.sha(log.read_bytes())}
        self.save()

    def save(self):
        self.t = self.file('terminal.json', self.terminal)
        self.p = self.file('phase.json', self.phase)
        self.c = self.file('compact.json', self.compact)

    def produce(self, cases=None):
        return json.loads(D.produce(self.t['path'], self.t['sha256'], self.p['path'], self.p['sha256'],
                                    self.c['path'], self.c['sha256'], SOURCE, JOB, cases or [CASE]))

    def reject(self, action, write=False):
        action()
        self.write() if write else self.save()
        with self.assertRaises((ValueError, KeyError, TypeError)):
            self.produce()

    def test_original_complete_bulk_zero_and_failing_corpus_are_distinct(self):
        r = self.produce()
        self.assertEqual(r['original_actual_exit'], 0)
        self.assertEqual(r['phase_actual_exit'], 1)
        self.assertEqual([v['variant'] for v in r['cases'][0]['failed_variants']], ['non-strict', 'strict'])
        self.assertFalse(r['tests_rerun']); self.assertFalse(r['qualification_claimed'])
        self.assertTrue(r['classification_unchanged'])

    def test_outer_one_is_also_truthful_and_observable(self):
        self.terminal.update(actual_exit=1, runner_exit=1, command_returncode=1); self.save()
        self.assertEqual(self.produce()['original_actual_exit'], 1)

    def test_running_or_unknown_never_authenticates(self):
        self.reject(lambda: self.terminal.update(state='RUNNING'))

    def test_wrong_original_source(self):
        self.reject(lambda: self.terminal['job'].update(source_commit='0' * 40))

    def test_wrong_original_job(self):
        self.reject(lambda: self.terminal['job'].update(job_id='0' * 32))

    def test_bool_actual_exit(self):
        self.reject(lambda: self.terminal.update(actual_exit=False))

    def test_contradictory_command_exit(self):
        self.reject(lambda: self.terminal.update(command_returncode=101))

    def test_bool_outer_command_exit(self):
        self.reject(lambda: self.terminal.update(command_returncode=False))

    def test_bool_outer_wrapper_exit(self):
        self.reject(lambda: self.terminal.update(runner_exit=False))

    def test_missing_outer_witness(self):
        self.reject(lambda: self.terminal.pop('command_returncode'))

    def test_passing_phase_cannot_produce_failure_details(self):
        self.reject(lambda: self.phase.update(command_exit=0, engine_exit=0, wrapper_exit=0))

    def test_abnormal_runner_phase_is_not_normal_classified_completion(self):
        self.reject(lambda: self.phase.update(command_exit=101))

    def test_bool_phase_witness(self):
        self.reject(lambda: self.phase.update(engine_exit=True))

    def test_missing_phase_witness(self):
        self.reject(lambda: self.phase.pop('wrapper_exit'))

    def test_output_copy_failure(self):
        self.reject(lambda: self.phase.update(output_copy_exit=1))

    def test_bool_output_copy(self):
        self.reject(lambda: self.phase.update(output_copy_exit=False))

    def test_raw_log_hash_contradiction(self):
        self.reject(lambda: self.phase['raw_native'].update(sha256='0' * 64))

    def test_raw_report_hash_contradiction(self):
        self.reject(lambda: self.compact['raw'].update(sha256='0' * 64))

    def test_bool_report_size(self):
        self.reject(lambda: self.compact['raw'].update(bytes=True))

    def test_wrong_actual_command_report(self):
        self.reject(lambda: self.phase['command'].__setitem__(-1, '/tmp/other.json'))

    def test_wrong_report_host_projection(self):
        self.reject(lambda: self.compact.update(host_sha256='0' * 64))

    def test_wrong_corpus_projection(self):
        self.reject(lambda: self.compact.update(test262_revision='0' * 40))

    def test_wrong_selection_projection(self):
        self.reject(lambda: self.compact['selection'].update(jobs=2))

    def test_raw_classification_hash_contradiction(self):
        self.reject(lambda: self.compact.update(classification_sha256='0' * 64))

    def test_duplicate_raw_file(self):
        self.reject(lambda: self.report['records'].append(copy.deepcopy(self.report['records'][0])), True)

    def test_duplicate_raw_variant(self):
        self.reject(lambda: self.report['records'][0]['variants'].append(copy.deepcopy(self.report['records'][0]['variants'][0])), True)

    def test_duplicate_original_compact_file(self):
        records = D.inflate(self.compact['compact_records']); records.append(copy.deepcopy(records[0]))
        self.compact['compact_records'] = base64.b64encode(zlib.compress(json.dumps(records).encode())).decode(); self.save()
        with self.assertRaises(ValueError): self.produce()

    def test_changed_original_variant_status(self):
        records = D.inflate(self.compact['compact_records']); records[0]['variants'][0]['status'] = 'pass'
        self.compact['compact_records'] = base64.b64encode(zlib.compress(json.dumps(records).encode())).decode(); self.save()
        with self.assertRaises(ValueError): self.produce()

    def test_case_selection_requires_actual_failure(self):
        for cases in ([self.report['records'][1]['path']], ['missing.js'], [CASE, CASE]):
            with self.assertRaises(ValueError): self.produce(cases)

    def test_symlink_native_report(self):
        alias = self.root / 'alias.json'; alias.symlink_to(self.root / 'report.json')
        self.compact['raw']['path'] = str(alias); self.phase['command'][-1] = str(alias); self.save()
        with self.assertRaises(ValueError): self.produce()

    def test_trailing_compact_stream(self):
        data = base64.b64decode(self.compact['compact_records']) + b'trailing'
        self.compact['compact_records'] = base64.b64encode(data).decode(); self.save()
        with self.assertRaises(ValueError): self.produce()

    def test_bounded_multibyte_details_keep_status_and_truncation(self):
        self.report['records'][0]['variants'][0]['stderr'] = '\u65e5' * 20000; self.write()
        r = self.produce(); details = r['cases'][0]['failed_variants'][0]['details']
        self.assertTrue(details['truncated']); self.assertLessEqual(len(details['text'].encode()), 8192)
        self.assertEqual(r['cases'][0]['status'], 'fail')


if __name__ == '__main__':
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls))
    print(json.dumps({'native_controls': result.testsRun,
                      'passed': result.testsRun - len(result.failures) - len(result.errors),
                      'failed': len(result.failures) + len(result.errors),
                      'fixture_only': True,
                      'helper_sha256': hashlib.sha256((ROOT / 'test262-failure-diagnostic.py').read_bytes()).hexdigest()}))
    raise SystemExit(0 if result.wasSuccessful() else 1)
