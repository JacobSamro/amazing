#!/usr/bin/env python3
"""Bulk native tooling controls. Protocol fixtures are not engine pass evidence."""
import base64
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zlib

SCRIPTS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('summary', SCRIPTS / 'gate-summary.py')
summary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(summary)
HOST = '''#!/usr/bin/env python3
import json,os,sys
if '--capabilities' in sys.argv or '--capabilities-v2' in sys.argv:
 print(json.dumps({'protocol':1,'async':False}));sys.exit(0)
payload=sys.stdin.buffer.read()
failed=os.environ.get('SUMMARY_FAIL')=='1' and b'CASE_000' in payload
print('TEST262_RESULT '+json.dumps({'protocol':1,'phase':'runtime' if failed else 'complete','error_type':'Error' if failed else ''}))
'''


class SummaryControls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix='gate-summary-controls-')
        cls.root = Path(cls.temp.name)
        corpus = cls.root / 'corpus'
        (corpus / '.git').mkdir(parents=True)
        (corpus / '.git/HEAD').write_text('0' * 40 + '\n')
        (corpus / 'harness').mkdir()
        for n in range(25):
            directory = corpus / f'test/selector-{n:02}'
            directory.mkdir(parents=True)
            for i in range(2):
                (directory / f'case-{i}.js').write_text(f'/*---\nflags: [raw]\n---*/\nCASE_{n:03};')
            (directory / 'input_FIXTURE.js').write_text('not executable JavaScript')
        host = cls.root / 'host'
        host.write_text(HOST.replace('#!/usr/bin/env python3', '#!' + sys.executable))
        host.chmod(0o755)
        cls.corpus, cls.host = corpus, host
        cls.index = []
        for mode in ('default', 'vm'):
            for n in range(25):
                selector = f'selector-{n:02}'
                compact = cls.run_report(selector + '-' + mode, selector, '', mode)
                cls.index.append({'role': 'before', 'selector': selector, 'mode': mode, 'compact': str(compact)})
        cls.shards = [cls.run_report(f'shard-{i}', '', f'{i}/16') for i in range(16)]
        cls.single = cls.run_report('single', '', '0/1')
        cls.failed = cls.run_report('failed', 'selector-00', '', fail=True)

    @classmethod
    def tearDownClass(cls):
        # Keep real runner reports/journals on the native host; never commit raw.
        cls.temp._finalizer.detach()
        print('native tooling raw: ' + str(cls.root))

    @classmethod
    def run_report(cls, name, selector, shard, mode='default', fail=False):
        output = cls.root / (name + '.raw.json')
        env = dict(os.environ)
        env.pop('AMAZING_JS_VM', None)
        if mode == 'vm': env['AMAZING_JS_VM'] = '1'
        env['SUMMARY_FAIL'] = '1' if fail else '0'
        command = [sys.executable, str(SCRIPTS / 'test262.py'), str(cls.corpus),
                   '--binary', str(cls.host), '--output', str(output), '--only', selector,
                   '--shard', shard, '--jobs', '8', '--timeout', '60']
        with (cls.root / (name + '.log')).open('wb') as log:
            result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode != int(fail): raise AssertionError((command, result.returncode))
        destination = cls.root / (name + '.compact.json')
        summary.compact(output, destination, result.returncode, '1' * 40)
        return destination

    def test_all_fifty_directory_mode_identities_and_fixtures(self):
        self.assertEqual(len(self.index), 50)
        for row in self.index:
            receipt = json.loads(Path(row['compact']).read_text())
            self.assertEqual(receipt['shard'], '')
            records = summary.unpack(receipt)
            # Existing runner emits fixtures before selection; preserve all of
            # them for an unsharded report, even outside this --only directory.
            self.assertEqual(len(records), 27)
            self.assertEqual(sum(r['status'] == 'pass' for r in records), 2)
            self.assertEqual(sum('_FIXTURE' in r['path'] for r in records), 25)
            self.assertEqual(receipt['selection'], {'filter': row['selector'], 'limit': 0, 'jobs': 8})
            raw = Path(receipt['raw']['path'])
            self.assertEqual(hashlib.sha256(raw.read_bytes()).hexdigest(), receipt['raw']['sha256'])

    def test_real_sixteen_shards_merge_ownership_unchanged(self):
        destination = self.root / 'merged.json'
        result = subprocess.run([sys.executable, str(SCRIPTS / 'test262-merge.py'), str(destination),
                                 *map(str, self.shards)], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        reports = [json.loads(p.read_text()) for p in self.shards]
        paths = [r['path'] for report in reports for r in summary.unpack(report)]
        self.assertEqual(len(paths), 75)
        self.assertEqual(len(set(paths)), 75)
        self.assertEqual(summary.unpack(json.loads(self.single.read_text())),
                         sorted([r for report in reports for r in summary.unpack(report)], key=lambda r: r['path']))

    def test_empty_shard_is_not_a_fake_merge_shard(self):
        result = subprocess.run([sys.executable, str(SCRIPTS / 'test262-merge.py'), str(self.root / 'invalid-merge.json'),
                                 self.index[0]['compact']], capture_output=True)
        self.assertEqual(result.returncode, 2)

    def test_normal_runner_one_is_preserved(self):
        report = json.loads(self.failed.read_text())
        self.assertEqual(report['runner_exit'], 1)
        self.assertEqual(report['phase_exits']['runner'], 1)
        self.assertEqual(report['summary']['files']['fail'], 2)
        for code in (-9, 2, 101, 124):
            with self.assertRaises(ValueError):
                summary.compact(report['raw']['path'], self.root / 'bad-exit.json', code, '1' * 40)

    def test_malformed_shards_paths_variants_and_classifications_rejected(self):
        for shard in (None, '1/1', '-1/16', '0/0', '0', '0/16/1', '0/01'):
            with self.assertRaises(ValueError): summary.shard_selection(shard)
        record = {'path': 'dir/a.js', 'status': 'pass', 'variants': [{'variant': 'strict', 'status': 'pass'}]}
        for mutation in ('duplicate-path', 'parent', 'absolute', 'status', 'duplicate-variant', 'variant-status'):
            records = [copy.deepcopy(record)]
            if mutation == 'duplicate-path': records.append(copy.deepcopy(record))
            elif mutation == 'parent': records[0]['path'] = '../a.js'
            elif mutation == 'absolute': records[0]['path'] = '/a.js'
            elif mutation == 'status': records[0]['status'] = 'PASS'
            elif mutation == 'duplicate-variant': records[0]['variants'] *= 2
            else: records[0]['variants'][0]['status'] = 'invalid'
            with self.assertRaises(ValueError): summary.validate_records(records)

    def bundle_index(self, rows, name):
        index = self.root / (name + '.index.json')
        index.write_text(json.dumps(rows))
        destination = self.root / (name + '.bundle.manifest.json')
        summary.bundle(index, destination)
        return destination

    def test_lossless_fifty_report_aggregate_with_exact_part_hashes(self):
        destination = self.bundle_index(self.index, 'fifty')
        manifest = json.loads(destination.read_text())
        self.assertEqual(manifest['reports'], 50)
        self.assertEqual(len(manifest['identities']), 50)
        encoded, total = '', destination.stat().st_size
        for part in manifest['parts']:
            data = (destination.parent / part['name']).read_bytes()
            self.assertEqual(len(data), part['bytes'])
            self.assertLessEqual(len(data), summary.LIMIT)
            self.assertEqual(hashlib.sha256(data).hexdigest(), part['sha256'])
            encoded += json.loads(data)['data']; total += len(data)
        self.assertLessEqual(total, summary.TICKET_LIMIT)
        payload = zlib.decompress(base64.b64decode(encoded))
        self.assertEqual(hashlib.sha256(payload).hexdigest(), manifest['uncompressed_sha256'])
        decoded = json.loads(payload)
        for expected, actual in zip(self.index, decoded['reports']):
            set_id = actual['receipt'].pop('record_set')
            actual['receipt']['records'] = decoded['record_sets'][set_id]
            receipt = json.loads(Path(expected['compact']).read_text())
            records = summary.unpack(receipt); receipt.pop('compact_records'); receipt['records'] = records
            self.assertEqual(actual['receipt'], receipt)
            self.assertEqual(actual['identity'], [expected[k] for k in ('role', 'selector', 'mode')])
        with self.assertRaises(ValueError): self.bundle_index(self.index, 'fifty')

    def test_full_size_overlapping_fifty_selection_bundle(self):
        receipt = json.loads(Path(self.single).read_text())
        records = [{'path': f'language/case-{i:05}.js', 'status': 'pass', 'reason': '',
                    'variants': [{'variant': 'strict', 'status': 'pass'},
                                 {'variant': 'non-strict', 'status': 'pass'}]}
                   for i in range(60000)]
        receipt.pop('compact_records'); receipt['records'] = records
        large = self.root / 'full-size.compact.json'; large.write_text(json.dumps(receipt))
        rows = [dict(row, compact=str(large)) for row in self.index]
        destination = self.bundle_index(rows, 'full-size-fifty')
        manifest = json.loads(destination.read_text())
        total = destination.stat().st_size + sum(p['bytes'] for p in manifest['parts'])
        self.assertLessEqual(total, summary.TICKET_LIMIT)
        encoded = ''.join(json.loads((destination.parent / p['name']).read_text())['data'] for p in manifest['parts'])
        payload = json.loads(zlib.decompress(base64.b64decode(encoded)))
        self.assertEqual(len(payload['reports']), 50)
        self.assertEqual(len(payload['record_sets']), 1)
        self.assertEqual(payload['record_sets'][0], records)
        self.assertEqual({r['receipt']['record_set'] for r in payload['reports']}, {0})

    def test_bundle_manifest_and_chunks_fit_completed_job_transport(self):
        transport_spec = importlib.util.spec_from_file_location('completed_job', SCRIPTS / 'completed-job.py')
        transport = importlib.util.module_from_spec(transport_spec)
        transport_spec.loader.exec_module(transport)
        destination = self.bundle_index(self.index, 'transport')
        self.assertIsNotNone(transport.COMPACT.fullmatch(destination.name))
        manifest = json.loads(destination.read_text())
        self.assertTrue(manifest['parts'])
        for part in manifest['parts']:
            self.assertIsNotNone(transport.COMPACT.fullmatch(part['name']))
            self.assertEqual(hashlib.sha256((destination.parent / part['name']).read_bytes()).hexdigest(), part['sha256'])

    def test_invalid_bundle_roles_modes_duplicates_and_phase_failure(self):
        for field, value in [('role', 'current'), ('mode', 'treewalk'), ('selector', '')]:
            rows = copy.deepcopy(self.index[:1]); rows[0][field] = value
            with self.assertRaises(ValueError): self.bundle_index(rows, 'invalid-' + field)
        with self.assertRaises(ValueError): self.bundle_index(self.index[:1] * 2, 'duplicate')
        receipt = json.loads(Path(self.index[0]['compact']).read_text()); receipt['job_exit'] = 3
        invalid = self.root / 'invalid-phase.json'; invalid.write_text(json.dumps(receipt))
        rows = copy.deepcopy(self.index[:1]); rows[0]['compact'] = str(invalid)
        with self.assertRaises(ValueError): self.bundle_index(rows, 'phase')

    def test_evidence_limits_fail_without_dropping_rows_or_writing_parts(self):
        with self.assertRaises(ValueError): summary.write(self.root / 'large.json', {'data': 'x' * summary.LIMIT})
        # Exercise the real all-or-nothing ticket guard with the same complete
        # index; reduced test budget models an incompressible real ticket.
        with patch.object(summary, 'TICKET_LIMIT', 1):
            with self.assertRaises(ValueError): self.bundle_index(self.index, 'budget')
        self.assertFalse(list(self.root.glob('budget.bundle.manifest.json*')))


if __name__ == '__main__':
    unittest.main()
