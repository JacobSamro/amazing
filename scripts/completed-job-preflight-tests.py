#!/usr/bin/env python3
"""Bulk endpoint contract tests, including allocation-free wrapper rejection."""
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
from unittest import mock

SCRIPTS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('completed_job', SCRIPTS / 'completed-job.py')
job = importlib.util.module_from_spec(spec)
spec.loader.exec_module(job)
LANE = 'gate-tooling-validation-20261005'
ROOT = f'amazing/out/{LANE}/qualification-01'
INVALID = ['/home/user/' + ROOT, '/' + ROOT, 'amazing/out/other/job',
           f'amazing/out/{LANE}', f'amazing/out/{LANE}/',
           f'amazing/out/{LANE}/../other/job', f'amazing/out/{LANE}/job/../../escape',
           ROOT + ';echo injected', ROOT + '\nother', ROOT + '/é', '', 'job']


def row(command='exit 0', root=ROOT):
    return dict(schema=1, lane=LANE, source_commit='1' * 40, box_id='bx_fixture',
                host='user@127.0.0.1', port=19001, job_id='2' * 32,
                output_root=root, command_sha256=hashlib.sha256(command.encode()).hexdigest())


class Paths(unittest.TestCase):
    def test_identical_receiver_and_preflight_contract_for_all_roots(self):
        for root in INVALID:
            with self.subTest(root=root):
                with self.assertRaises(ValueError): job.valid_output_root(LANE, root)
                with self.assertRaises(ValueError): job.valid(row(root=root))
        for root in [ROOT, ROOT + '/nested', ROOT + '-retry_02']:
            with self.subTest(root=root):
                self.assertEqual(job.valid_output_root(LANE, root), root)
                self.assertEqual(job.valid(row(root=root))['output_root'], root)
        for lane in ['', '../other', 'bad lane', 'bad;lane', 'é']:
            with self.subTest(lane=lane), self.assertRaises(ValueError):
                job.valid_output_root(lane, ROOT)

    def test_cli_rejects_without_allocating_receipt_or_output(self):
        for root in INVALID + [ROOT]:
            with self.subTest(root=root), tempfile.TemporaryDirectory(dir='/tmp') as directory:
                result = subprocess.run([sys.executable, str(SCRIPTS / 'completed-job.py'),
                    'validate-request', '--lane', LANE, '--root', root], cwd=directory,
                    capture_output=True, text=True)
                self.assertEqual(result.returncode, 0 if root == ROOT else 3)
                self.assertEqual(list(Path(directory).iterdir()), [])

    def test_wrapper_rejects_before_lock_source_shipping_or_boat(self):
        for root in INVALID:
            with self.subTest(root=root), tempfile.TemporaryDirectory(dir='/tmp') as directory:
                base = Path(directory); scripts = base / 'scripts'; scripts.mkdir()
                for name in ['linux-run.sh', 'completed-job.py']:
                    (scripts / name).write_bytes((SCRIPTS / name).read_bytes())
                tools = base / 'tools'; tools.mkdir()
                git = tools / 'git'
                git.write_text('#!/bin/sh\nprintf "%s\\n" ' + shlex.quote(str(base / '.git')) + '\n')
                git.chmod(0o755)
                sentinel = base / 'allocation-attempt'
                (scripts / 'boat-lane.py').write_text('#!/bin/sh\ntouch ' + shlex.quote(str(sentinel)) + '\nexit 99\n')
                (scripts / 'boat-lane.py').chmod(0o755)
                env = {**os.environ, 'PATH': str(tools) + ':' + os.environ['PATH']}
                result = subprocess.run(['bash', str(scripts / 'linux-run.sh'), LANE,
                    '--host', 'boat', '--ref', '1' * 40, '--endpoint-receipt', str(base / 'endpoint.json'),
                    '--compact-root', root, '--', 'exit 0'], env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 2, result.stderr)
                diagnostic = ('endpoint capture requires a fresh receipt, exact ref and compact root'
                              if not root else 'invalid job output root')
                self.assertIn(diagnostic, result.stderr)
                self.assertFalse(sentinel.exists())
                self.assertFalse((base / '.agents').exists())
                self.assertFalse((base / 'endpoint.json').exists())

    def test_remaining_receipt_fields_remain_strict(self):
        for key, value in [('schema', 2), ('lane', 'bad lane'), ('source_commit', 'main'),
                           ('box_id', 'guessed'), ('host', 'shell;host'), ('port', 0),
                           ('port', 65536), ('job_id', 'old'), ('command_sha256', 'wrong')]:
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                job.valid({**row(), key: value})


class Completion(unittest.TestCase):
    def test_authentic_exit_compact_hash_and_mutation_rejection(self):
        for code in [0, 1, 7]:
            with self.subTest(code=code), tempfile.TemporaryDirectory(dir='/tmp') as directory:
                home = Path(directory); root = home / ROOT
                program = ('from pathlib import Path; import json,sys; p=Path(' + repr(str(root)) + ');'
                           'p.mkdir(parents=True);(p/"result.compact.json").write_text(json.dumps({"ready":False})+"\\n");'
                           '(p/"raw.log").write_text("authentic raw");sys.exit(' + str(code) + ')')
                command = shlex.quote(sys.executable) + ' -c ' + shlex.quote(program)
                receipt = row(command)
                with mock.patch.object(job.pathlib.Path, 'home', return_value=home), \
                     mock.patch.object(job, 'marker', return_value=home / 'completed.json'):
                    self.assertEqual(job.run(receipt, command), code)
                    final = json.loads((home / 'completed.json').read_text())
                    self.assertEqual(final['actual_exit'], code)
                    self.assertEqual(set(final['artifacts']), {ROOT + '/result.compact.json'})
                    actual = job.verify(receipt, LANE, '1' * 40, [ROOT + '/result.compact.json'])
                    self.assertEqual(actual['actual_exit'], code)
                    (root / 'result.compact.json').write_text('{"changed":true}')
                    with self.assertRaisesRegex(ValueError, 'changed'):
                        job.verify(receipt, LANE, '1' * 40, [ROOT + '/result.compact.json'])

    def test_command_or_preexisting_namespace_never_runs(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as directory:
            home = Path(directory)
            with mock.patch.object(job.pathlib.Path, 'home', return_value=home), \
                 mock.patch.object(job, 'marker', return_value=home / 'completed.json'):
                with self.assertRaisesRegex(ValueError, 'command mismatch'):
                    job.run(row('exit 0'), 'exit 1')
                (home / ROOT).mkdir(parents=True)
                with self.assertRaisesRegex(ValueError, 'already exists'):
                    job.run(row(), 'exit 0')
                self.assertFalse((home / 'completed.json').exists())


if __name__ == '__main__': unittest.main()
