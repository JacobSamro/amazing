#!/usr/bin/env python3
"""Native byte/exit/fault controls; synthetic packets, not engine results."""
import concurrent.futures
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

S = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('source_upload', S / 'source-upload.py')
stage = importlib.util.module_from_spec(spec); spec.loader.exec_module(stage)
spec = importlib.util.spec_from_file_location('native_inputs', S / 'native-inputs.py')
native = importlib.util.module_from_spec(spec); spec.loader.exec_module(native)
CONTROL_RECEIPTS = []
SOURCE = os.environ.get('COMMIT', 'a' * 40)


class Packets(unittest.TestCase):
    def receiver(self, destination, data, size=None, digest=None):
        return stage.receive(destination, len(data) if size is None else size,
                             hashlib.sha256(data).hexdigest() if digest is None else digest,
                             SOURCE, io.BytesIO(data))

    def test_native_packet_is_private_and_byteexact_before_publish(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); data = bytes(range(256)) * 32768
            path = root / 'source.tar.gz'; result = self.receiver(path, data)
            self.assertTrue(result['published']); self.assertEqual(path.read_bytes(), data)
            self.assertEqual(result['archive_sha256'], stage.sha(path))
            self.assertEqual(result['archive_bytes'], 8 * 1024 * 1024)
            self.assertEqual(result['native_device'], Path('/tmp').stat().st_dev)
            self.assertFalse(list(root.glob('*.partial')))
            CONTROL_RECEIPTS.append({'control': self._testMethodName, 'receiver': result})

    def test_truncation_extra_bytes_and_mismatch_never_publish(self):
        for size, digest in [(4, None), (2, None), (3, '0' * 64)]:
            with self.subTest(size=size, digest=digest), tempfile.TemporaryDirectory(dir='/tmp') as d:
                path = Path(d) / 'source.tar.gz'
                with self.assertRaises(ValueError): self.receiver(path, b'abc', size, digest)
                self.assertFalse(path.exists()); self.assertEqual(len(list(Path(d).glob('*.partial'))), 1)

    def test_prior_packet_and_symlink_namespace_are_preserved(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); path = root / 'source.tar.gz'; path.write_bytes(b'prior')
            with self.assertRaises(ValueError): self.receiver(path, b'new')
            self.assertEqual(path.read_bytes(), b'prior')
            path.unlink(); path.symlink_to(root / 'uncreated')
            with self.assertRaises(ValueError): self.receiver(path, b'new')
            self.assertTrue(path.is_symlink()); self.assertFalse((root / 'uncreated').exists())
            real = root / 'real'; real.mkdir(); link = root / 'link'; link.symlink_to(real, target_is_directory=True)
            with self.assertRaises(ValueError): self.receiver(link / 'source.tar.gz', b'new')
            self.assertFalse((real / 'source.tar.gz').exists())

    def test_HOME_receiver_and_insufficient_native_capacity_are_rejected(self):
        with tempfile.TemporaryDirectory(dir=Path.home()) as d:
            with self.assertRaises(ValueError): self.receiver(Path(d) / 'source.tar.gz', b'bytes')
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            with mock.patch.object(stage.shutil, 'disk_usage', return_value=type('Space', (), {'free': 4})()):
                with self.assertRaises(OSError) as caught: self.receiver(Path(d) / 'source.tar.gz', b'bytes')
                self.assertEqual(caught.exception.errno, 28)
            self.assertFalse(list(Path(d).iterdir()))

    def test_archive_modes_bytes_and_nonzero_staging_timestamps_remain_identical(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); archive = root / 'local.tar.gz'
            with tarfile.open(archive, 'w:gz') as out:
                for name, data, mode in [('Cargo.toml', b'[workspace]\n', 0o644),
                                         ('scripts/run.sh', b'#!/bin/sh\n', 0o755),
                                         ('fixture/source.bin', bytes(range(256)), 0o644)]:
                    item = tarfile.TarInfo(name); item.size = len(data); item.mode = mode; item.mtime = 0
                    out.addfile(item, io.BytesIO(data))
            packet = root / 'source.tar.gz'; row = self.receiver(packet, archive.read_bytes())
            staged = native.extract(packet, row['archive_sha256'], root / 'source')
            self.assertGreater(staged['source_timestamps']['stamp_ns'], 0)
            self.assertEqual((root / 'source/fixture/source.bin').read_bytes(), bytes(range(256)))
            self.assertEqual((root / 'source/scripts/run.sh').stat().st_mode & 0o777, 0o755)
            self.assertEqual(staged['regular_files'], 3)
            CONTROL_RECEIPTS.append({'control': self._testMethodName, 'receiver': row, 'staging': staged})


class Sender(unittest.TestCase):
    def run_receiver(self, root, size=8 * 1024 * 1024):
        packet = root / 'local.tar.gz'; packet.write_bytes(bytes(range(256)) * (size // 256))
        facts = stage.describe(packet, SOURCE)
        command = [sys.executable, '-I', str(S / 'source-upload.py'), 'receive',
                   str(root / 'received.tar.gz'), str(facts['archive_bytes']), facts['archive_sha256'], SOURCE]
        return packet, command

    def test_real_native_receiver_and_local_sender_preserve_exact_packet(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); packet, command = self.run_receiver(root)
            row = stage.upload(packet, SOURCE, command, 15)
            self.assertTrue(row['ready']); self.assertEqual(row['actual_exit'], 0)
            self.assertEqual(row['runner_exit'], 0); self.assertGreater(row['finished_ns'], row['started_ns'])
            self.assertEqual((root / 'received.tar.gz').read_bytes(), packet.read_bytes())
            CONTROL_RECEIPTS.append({'control': self._testMethodName, 'transfer': row})

    def test_actual_lost_connection_255_is_not_a_timeout_or_receiver_success(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); packet, command = self.run_receiver(root, 1024)
            row = stage.upload(packet, SOURCE, [sys.executable, '-c', 'import sys;sys.stderr.write("lost connection\\n");sys.exit(255)'], 5)
            self.assertFalse(row['ready']); self.assertEqual(row['actual_exit'], 255)
            self.assertEqual(row['command_exit'], 255); self.assertFalse(row['timed_out']); self.assertIsNone(row['receiver'])
            self.assertEqual(row['archive_bytes'], 1024); self.assertIn('lost connection', row['diagnostics']['stderr_tail'])
            CONTROL_RECEIPTS.append({'control': self._testMethodName, 'transfer': row})

    def test_deadline_terminates_and_reaps_only_owned_transfer(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); packet, unused = self.run_receiver(root, 1024)
            row = stage.upload(packet, SOURCE, [sys.executable, '-c', 'import time;time.sleep(30)'], 1)
            self.assertFalse(row['ready']); self.assertTrue(row['timed_out']); self.assertEqual(row['actual_exit'], 124)
            self.assertEqual(row['command_exit'], -signal.SIGTERM)
            CONTROL_RECEIPTS.append({'control': self._testMethodName, 'transfer': row})

    def test_zero_process_exit_requires_real_receiver_identity(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); packet, unused = self.run_receiver(root, 1024)
            for output in ['{}', 'not JSON', json.dumps({'actual_exit': 0, 'published': True, 'source_commit': 'wrong'})]:
                with self.subTest(output=output):
                    row = stage.upload(packet, SOURCE, [sys.executable, '-c', 'print(' + repr(output) + ')'], 5)
                    self.assertEqual(row['actual_exit'], 0); self.assertEqual(row['runner_exit'], 3)
                    self.assertFalse(row['ready']); self.assertTrue(row['verification_error'])

    def test_native_receiver_failure_keeps_actual_exit_and_error_metadata(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); packet, command = self.run_receiver(root, 1024); command[-2] = '0' * 64
            row = stage.upload(packet, SOURCE, command, 5)
            self.assertFalse(row['ready']); self.assertEqual(row['actual_exit'], 3)
            self.assertEqual(row['receiver']['error']['type'], 'ValueError')
            self.assertFalse(row['receiver']['published']); self.assertFalse((root / 'received.tar.gz').exists())
            CONTROL_RECEIPTS.append({'control': self._testMethodName, 'transfer': row})

    def test_real_receiver_filesystem_disconnect_has_truthful_nonzero_metadata(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); packet, unused = self.run_receiver(root, 1024)
            code = ('import importlib.util,pathlib,sys;'
                    's=importlib.util.spec_from_file_location("upload",' + repr(str(S / 'source-upload.py')) + ');'
                    'm=importlib.util.module_from_spec(s);s.loader.exec_module(m);'
                    'real=pathlib.Path.open\n'
                    'def failed(path,*args,**kwargs):\n'
                    ' if path.name.endswith(".partial"):raise OSError(107,"controlled native receiver disconnect")\n'
                    ' return real(path,*args,**kwargs)\n'
                    'pathlib.Path.open=failed\n'
                    'sys.argv=["source-upload.py","receive",' + repr(str(root / 'received.tar.gz')) +
                    ',"1024",' + repr(stage.sha(packet)) + ',' + repr(SOURCE) + '];sys.exit(m.main())')
            row = stage.upload(packet, SOURCE, [sys.executable, '-I', '-c', code], 5)
            self.assertEqual(row['actual_exit'], 3); self.assertEqual(row['runner_exit'], 3)
            self.assertEqual(row['receiver']['error']['errno'], 107); self.assertFalse(row['ready'])
            self.assertFalse((root / 'received.tar.gz').exists())
            CONTROL_RECEIPTS.append({'control': self._testMethodName, 'transfer': row})

    def test_directory_sync_failure_does_not_claim_packet_was_never_published(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); packet, unused = self.run_receiver(root, 1024)
            code = ('import importlib.util,os,stat,sys;'
                    's=importlib.util.spec_from_file_location("upload",' + repr(str(S / 'source-upload.py')) + ');'
                    'm=importlib.util.module_from_spec(s);s.loader.exec_module(m);real=os.fsync\n'
                    'def failed(fd):\n'
                    ' if stat.S_ISDIR(os.fstat(fd).st_mode):raise OSError(107,"controlled directory custody disconnect")\n'
                    ' return real(fd)\n'
                    'os.fsync=failed\n'
                    'sys.argv=["source-upload.py","receive",' + repr(str(root / 'received.tar.gz')) +
                    ',"1024",' + repr(stage.sha(packet)) + ',' + repr(SOURCE) + '];sys.exit(m.main())')
            row = stage.upload(packet, SOURCE, [sys.executable, '-I', '-c', code], 5)
            self.assertEqual(row['actual_exit'], 3); self.assertFalse(row['ready'])
            self.assertEqual(row['receiver']['error']['errno'], 107); self.assertIsNone(row['receiver']['published'])
            self.assertEqual((root / 'received.tar.gz').read_bytes(), packet.read_bytes())
            CONTROL_RECEIPTS.append({'control': self._testMethodName, 'transfer': row})

    def test_local_packet_mutation_is_not_qualified_after_receiver_zero(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); packet, unused = self.run_receiver(root, 1024)
            facts = stage.describe(packet, SOURCE)
            reply = {'actual_exit': 0, 'published': True, **{k: facts[k] for k in ('source_commit', 'archive_bytes', 'archive_sha256')}}
            code = 'import pathlib,json;pathlib.Path(' + repr(str(packet)) + ').write_bytes(b"changed");print(' + repr(json.dumps(reply)) + ')'
            row = stage.upload(packet, SOURCE, [sys.executable, '-c', code], 5)
            self.assertEqual(row['actual_exit'], 0); self.assertEqual(row['runner_exit'], 3); self.assertFalse(row['ready'])

    def test_compact_setup_failure_retains_pre_job_packet_provenance(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root = Path(d); packet, unused = self.run_receiver(root, 1024)
            receipt = root / 'transfer.summary.json'; summary = root / 'setup.summary.json'
            facts = stage.upload(packet, SOURCE, [sys.executable, '-c', 'raise SystemExit(255)'], 5)
            stage.write_receipt(receipt, facts)
            result = subprocess.run([sys.executable, str(S / 'gate-summary.py'), 'failure', '--source-commit', SOURCE,
                                     '--shard', 'source-tooling-control', '--wrapper-exit', '3', '--phase-receipt', str(receipt),
                                     str(summary), 'native-source-upload', '255'], capture_output=True, timeout=15)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            row = json.loads(summary.read_text()); self.assertEqual(row['actual_exit'], 255)
            self.assertEqual(row['phase_detail']['archive_sha256'], facts['archive_sha256'])
            self.assertEqual(row['phase_detail']['archive_bytes'], 1024)
            self.assertLess(summary.stat().st_size, 512 * 1024)

    def test_public_wrapper_uses_one_bounded_native_receiver_and_keeps_hash_guard(self):
        source = (S / 'linux-run.sh').read_text()
        self.assertIn('ATTEMPTS="1"', source)
        self.assertIn('source-upload.py" --receipt "$SOURCE_TRANSFER_RECEIPT" send', source)
        self.assertIn('--seconds 900', source)
        self.assertIn('--phase-receipt "$SOURCE_TRANSFER_RECEIPT"', source)
        self.assertIn('source-upload.py" --receipt "$SOURCE_TRANSFER_RECEIPT" describe', source)
        self.assertIn('native-source-sha256', source)
        self.assertIn('sha256sum -c -', source)


class FiniteStreaming(unittest.TestCase):
    def packet(self, root):
        path = root / 'full-size-immutable.packet'
        with path.open('xb') as stream:
            for _ in range(64): stream.write(bytes(range(256)) * 4096)
        return path

    def receiver_command(self, path, packet, seconds):
        return [sys.executable, '-I', str(S / 'source-upload.py'), 'receive',
                str(path), str(packet.stat().st_size), stage.sha(packet), SOURCE,
                '--seconds', str(seconds)]

    def test_full_size_immutable_transfer_and_native_stage_diagnostics(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root=Path(d);packet=self.packet(root);destination=root/'received.packet'
            before=stage.describe(packet,SOURCE)
            row=stage.upload(packet,SOURCE,self.receiver_command(destination,packet,60),60)
            self.assertTrue(row['ready']);self.assertEqual(row['actual_exit'],0)
            self.assertEqual(stage.sha(destination),before['archive_sha256'])
            self.assertEqual(destination.stat().st_size,64*1024*1024)
            self.assertEqual(stage.describe(packet,SOURCE),before)
            self.assertIn('hash-readback',row['diagnostics']['stderr_tail'])
            self.assertIn('publishing',row['diagnostics']['stderr_tail'])
            CONTROL_RECEIPTS.append({'control':self._testMethodName,'transfer':row})

    def test_receiver_cutoff_while_waiting_for_eof_never_publishes(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root=Path(d);packet=root/'small.packet';packet.write_bytes(b'bytes')
            child=subprocess.Popen(self.receiver_command(root/'received.packet',packet,1),
                                   stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            child.stdin.write(b'bytes');child.stdin.flush()
            # Keep writer open: the previous receiver would remain blocked
            # until an unrelated SSH lifecycle ended it, with no native bound.
            actual=child.wait(timeout=10)
            child.stdin.close();child.stdin=None
            stdout,stderr=child.communicate(timeout=5)
            self.assertEqual(actual,124)
            reply=json.loads(stdout);self.assertEqual(reply['actual_exit'],124)
            self.assertFalse(reply['ready']);self.assertIsNone(reply['published'])
            self.assertFalse((root/'received.packet').exists())
            self.assertEqual(len(list(root.glob('*.partial'))),1)
            self.assertIn('awaiting-eof',stderr.decode())
            CONTROL_RECEIPTS.append({'control':self._testMethodName,'receiver':reply})

    def test_receiver_cutoff_before_first_byte_is_finite(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root=Path(d);packet=root/'small.packet';packet.write_bytes(b'bytes')
            child=subprocess.Popen(self.receiver_command(root/'received.packet',packet,1),
                                   stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            actual=child.wait(timeout=10);child.stdin.close();child.stdin=None
            stdout,stderr=child.communicate(timeout=5)
            self.assertEqual(actual,124);self.assertEqual(json.loads(stdout)['actual_exit'],124)
            self.assertFalse((root/'received.packet').exists())
            CONTROL_RECEIPTS.append({'control':self._testMethodName,'actual_exit':actual})

    def test_three_concurrent_full_size_packets_preserve_source_and_distinct_namespace(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            root=Path(d);packet=self.packet(root);before=stage.describe(packet,SOURCE)
            def transfer(number):
                destination=root/f'received-{number}.packet'
                return stage.upload(packet,SOURCE,self.receiver_command(destination,packet,60),60)
            with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
                rows=list(pool.map(transfer,range(3)))
            self.assertTrue(all(row['ready'] and row['actual_exit']==0 for row in rows))
            self.assertEqual(stage.describe(packet,SOURCE),before)
            self.assertEqual({row['receiver']['archive_sha256'] for row in rows},{before['archive_sha256']})
            self.assertEqual(len({row['receiver']['native_packet'] for row in rows}),3)
            CONTROL_RECEIPTS.append({'control':self._testMethodName,'transfers':rows})

    def test_receiver_deadline_restores_only_its_owned_timer_and_handler(self):
        old=signal.getsignal(signal.SIGALRM)
        with stage.receiver_deadline(1):self.assertNotEqual(signal.getitimer(signal.ITIMER_REAL),(0.0,0.0))
        self.assertEqual(signal.getsignal(signal.SIGALRM),old)
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL),(0.0,0.0))
        for seconds in [0,-1,1201]:
            with self.assertRaises(ValueError):
                with stage.receiver_deadline(seconds):pass
        signal.setitimer(signal.ITIMER_REAL,60)
        try:
            with self.assertRaises(ValueError):
                with stage.receiver_deadline(1):pass
        finally:signal.setitimer(signal.ITIMER_REAL,0)


if __name__ == '__main__':
    unittest.main()
