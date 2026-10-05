#!/usr/bin/env python3
"""Whole public native transport contracts; tooling fixtures, never engine tests."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

SCRIPTS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('native_inputs', SCRIPTS / 'native-inputs.py')
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)


class Sources(unittest.TestCase):
    def archive(self, root):
        archive = root / 'source.tgz'
        with tarfile.open(archive, 'w:gz') as stream:
            for name, data, mode in [('Cargo.toml', b'[workspace]\n', 0o644),
                ('examples/input.sdp', b'v=0\r\n', 0o644),
                ('fixtures/target-manifest/raw.bin', bytes(range(256)), 0o644),
                ('scripts/driver.sh', b'#!/bin/sh\n', 0o755)]:
                member = tarfile.TarInfo(name); member.size = len(data); member.mode = mode; member.mtime = 1
                stream.addfile(member, io.BytesIO(data))
        return archive

    def test_complete_nonrust_fixtures_and_modes_survive_native_stage(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); archive = self.archive(root); dest = root / 'native'
            row = stage.extract(archive, stage.digest(archive), dest)
            self.assertEqual(row['regular_files'], 4)
            self.assertEqual((dest / 'examples/input.sdp').read_bytes(), b'v=0\r\n')
            self.assertEqual((dest / 'fixtures/target-manifest/raw.bin').read_bytes(), bytes(range(256)))
            self.assertTrue((dest / 'scripts/driver.sh').stat().st_mode & 0o100)
            self.assertGreater((dest / 'Cargo.toml').stat().st_mtime, 1)

    def test_changed_archive_is_rejected_before_destination_creation(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); archive = self.archive(root); expected = stage.digest(archive)
            with archive.open('ab') as stream: stream.write(b'changed bytes')
            with self.assertRaisesRegex(ValueError, 'SHA256 mismatch'):
                stage.extract(archive, expected, root / 'native')
            self.assertFalse((root / 'native').exists())

    def test_existing_source_namespace_is_never_overwritten(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); archive = self.archive(root); dest = root / 'native'; dest.mkdir()
            protected = dest / 'prior'; protected.write_bytes(b'original proof')
            with self.assertRaises(FileExistsError): stage.extract(archive, stage.digest(archive), dest)
            self.assertEqual(protected.read_bytes(), b'original proof')

    def test_traversal_and_external_symlinks_fail_closed(self):
        for symlink in [False, True]:
            with self.subTest(symlink=symlink), tempfile.TemporaryDirectory(dir='/tmp') as d:
                root = Path(d); archive = root / 'bad.tgz'
                with tarfile.open(archive, 'w:gz') as stream:
                    row = tarfile.TarInfo('escape' if symlink else '../escape')
                    if symlink: row.type = tarfile.SYMTYPE; row.linkname = '/etc/passwd'; stream.addfile(row)
                    else: row.size = 1; stream.addfile(row, io.BytesIO(b'x'))
                with self.assertRaises(tarfile.FilterError): stage.extract(archive, stage.digest(archive), root / 'native')
                self.assertFalse((root / 'escape').exists())


class Corpus(unittest.TestCase):
    def test_input_identity_matches_actual_runner_path_and_byte_order(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d)
            paths = []
            for part, count in [('test', 150), ('harness', 43)]:
                for n in reversed(range(count)):
                    path = root / part / f'area/{n:04}.js'; path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes((f'// {part} {n}\n').encode()); paths.append(path)
            ordered = sorted(p for p in paths if '/test/' in str(p)) + sorted(p for p in paths if '/harness/' in str(p))
            expected = hashlib.sha256()
            for path in ordered:
                expected.update(path.relative_to(root).as_posix().encode() + b'\0' + hashlib.sha256(path.read_bytes()).digest())
            self.assertEqual(stage.inventory(root), (150, 43, expected.hexdigest()))
            ordered[0].write_bytes(b'changed'); self.assertNotEqual(stage.inventory(root)[2], expected.hexdigest())

    def test_snapshot_read_failure_is_not_converted_to_missing_test(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); (root / 'test').mkdir(); (root / 'test/a.js').write_bytes(b'1')
            with mock.patch.object(stage, 'digest', side_effect=OSError(107, 'Transport endpoint is not connected')):
                with self.assertRaises(OSError) as caught: stage.inventory(root)
            self.assertEqual(caught.exception.errno, 107)

    def test_wrong_revision_and_dirty_input_are_rejected(self):
        with mock.patch.object(stage.subprocess, 'check_output', return_value='wrong\n'):
            with self.assertRaisesRegex(ValueError, 'revision mismatch'): stage.validate_corpus(Path('/tmp'), {})
        with mock.patch.object(stage.subprocess, 'check_output', return_value=stage.REVISION+'\n'), \
             mock.patch.object(stage.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, ['git', 'diff'])):
            with self.assertRaises(subprocess.CalledProcessError): stage.validate_corpus(Path('/tmp'), {})


class Transfer(unittest.TestCase):
    def test_real_command_exit_is_not_replaced_by_wrapper_success(self):
        row = stage.transfer([sys.executable, '-c', 'raise SystemExit(255)'], 5)
        self.assertEqual(row['command_exit'], 255); self.assertEqual(row['actual_exit'], 255)
        self.assertFalse(row['timed_out'])

    def test_finite_deadline_preserves_timeout_and_actual_reaped_status(self):
        row = stage.transfer([sys.executable, '-c', 'import time;time.sleep(30)'], 1)
        self.assertTrue(row['timed_out']); self.assertEqual(row['actual_exit'], 124)
        self.assertIsInstance(row['command_exit'], int)

    def test_preengine_failure_receipt_is_authentic(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            receipt = Path(d) / 'failure.json'
            proc = subprocess.run([sys.executable, str(SCRIPTS / 'native-inputs.py'), '--receipt', str(receipt),
                'source', str(Path(d)/'absent'), '0'*64, str(Path(d)/'native')], capture_output=True, text=True)
            self.assertNotEqual(proc.returncode, 0)
            row = json.loads(receipt.read_text()); self.assertFalse(row['ready'])
            self.assertEqual(row['error']['errno'], 2)


class PublicBoundary(unittest.TestCase):
    def test_source_and_corpus_no_longer_traverse_home_before_execution(self):
        run = (SCRIPTS / 'linux-run.sh').read_text().split('# Bulk transfers')[0]
        self.assertNotIn('"$BOAT_HOST:amazing/wt/$LANE/"', run)
        self.assertIn('native-source-upload', run)
        self.assertIn('transfer --seconds 900', run)
        bootstrap = (SCRIPTS / 'linux-bootstrap.sh').read_text()
        self.assertIn('native-test262-input 1200', bootstrap)
        self.assertIn('TEST262_ROOT=/tmp/amazing-test262-3655e746', bootstrap)
        self.assertIn('PYTHONNOUSERSITE=1', bootstrap)
        self.assertIn('native-python-input 120 /usr/bin/python3 -s -S -P', bootstrap)
        shard = (SCRIPTS / 'test262-boat.sh').read_text()
        self.assertNotIn('ln -sfn \\\"\\$HOME/corpora', shard)


if __name__ == '__main__': unittest.main()
