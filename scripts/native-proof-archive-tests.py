#!/usr/bin/env python3
"""Native filesystem/archival controls; no engine or restored snapshot claim."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import uuid
from unittest.mock import patch

S=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('archive',S/'native-proof-archive.py')
a=importlib.util.module_from_spec(spec);spec.loader.exec_module(a)
SOURCE=os.environ.get('COMMIT','a'*40)
RESULTS=[]

class NativeArchive(unittest.TestCase):
    def fixture(self):
        temp=tempfile.TemporaryDirectory(dir='/tmp');self.addCleanup(temp.cleanup)
        root=Path(temp.name);raw=root/'raw';elf=root/'elf';raw.mkdir();elf.mkdir()
        (raw/'nested').mkdir();(raw/'nested/report.json').write_bytes(b'{"actual_exit":1,"failed":["original"]}')
        (raw/'empty').write_bytes(b'');(raw/'binary.raw').write_bytes(bytes(range(256))*4096)
        shutil.copyfile('/usr/bin/true',elf/'measured');(elf/'measured').chmod(0o755)
        actual=subprocess.run([str(elf/'measured')]).returncode
        job='native-'+uuid.uuid4().hex
        terminal=root/'terminal.manifest.json';terminal.write_text(json.dumps({'state':'TERMINAL','source':SOURCE,'job':job,'actual_exit':actual}))
        return root,{'raw':raw,'elf':elf},terminal,job

    def perform(self,roots,terminal,job,budget=128*1024**2):
        return a.archive('archive-controls',SOURCE,job,roots,terminal,budget,128*1024**2)

    def test_real_native_root_exact_files_real_measured_elf_and_source_unchanged(self):
        root,inputs,terminal,job=self.fixture();before=a.inventory(inputs)
        result=self.perform(inputs,terminal,job);self.assertTrue(result['complete'])
        self.assertEqual(result['files'],4);self.assertEqual(result['elf_files'],1)
        self.assertIn(result['mount']['filesystem'],a.NATIVE_TYPES)
        self.assertEqual(before,a.inventory(inputs));manifest=Path(result['archive_manifest']['path'])
        self.assertEqual(a.digest(manifest),result['archive_manifest']['sha256'])
        copied=json.loads(manifest.read_text())
        for record in copied['files']:
            file=Path(result['destination'])/'payload'/record['label']/record['path']
            self.assertEqual(a.digest(file),record['sha256']);self.assertEqual(file.stat().st_size,record['bytes'])
            self.assertEqual(file.stat().st_mode&0o777,record['mode'])
        RESULTS.append(result)

    def test_capacity_failures_do_not_publish_or_change_originals(self):
        root,inputs,terminal,job=self.fixture();before=a.inventory(inputs)
        with self.assertRaises(OSError):self.perform(inputs,terminal,job,0)
        self.assertFalse((a.ROOT/'archive-controls'/SOURCE/job).exists());self.assertEqual(before,a.inventory(inputs))
        with patch.object(a.shutil,'disk_usage',return_value=type('Space',(),{'free':0})()):
            with self.assertRaises(OSError):self.perform(inputs,terminal,job)
        self.assertFalse((a.ROOT/'archive-controls'/SOURCE/job).exists())

    def test_symlinks_fifo_exclusions_and_non_native_mount_rejected(self):
        root,inputs,terminal,job=self.fixture()
        link=inputs['raw']/'link';link.symlink_to(inputs['raw']/'empty')
        with self.assertRaises(ValueError):self.perform(inputs,terminal,job)
        link.unlink();fifo=inputs['raw']/'fifo';os.mkfifo(fifo)
        with self.assertRaises(ValueError):self.perform(inputs,terminal,job)
        fifo.unlink();policy=inputs['raw']/'.boxignore';policy.write_text('*.raw\n')
        with self.assertRaises(ValueError):self.perform(inputs,terminal,job)
        policy.unlink()
        for kind in ['fuse','fuse.asciifs','tmpfs','ramfs']:
            with patch.object(a.Path,'read_text',return_value=f'1 0 0:1 / / rw - {kind} none rw\n'):
                with self.assertRaises(ValueError):a.native_mount(a.ROOT)

    def test_producer_must_be_authentic_terminal_source_job_and_int_exit(self):
        root,inputs,terminal,job=self.fixture();original=json.loads(terminal.read_text())
        for change in [{'state':'RUNNING'},{'source':'b'*40},{'job':'foreign'},{'actual_exit':None},{'actual_exit':True}]:
            terminal.write_text(json.dumps(dict(original,**change)))
            with self.assertRaises(ValueError):self.perform(inputs,terminal,job)
            self.assertFalse((a.ROOT/'archive-controls'/SOURCE/job).exists())
        terminal.write_text(json.dumps(dict(original,actual_exit=100)))
        result=self.perform(inputs,terminal,job)
        self.assertEqual(json.loads(Path(result['archive_manifest']['path']).read_text())['terminal_producer']['actual_exit'],100)
        RESULTS.append(result)

    def test_collision_never_overwrites_prior_archived_proof(self):
        root,inputs,terminal,job=self.fixture();first=self.perform(inputs,terminal,job)
        saved=Path(first['archive_manifest']['path']).read_bytes()
        with self.assertRaises(ValueError):self.perform(inputs,terminal,job)
        self.assertEqual(Path(first['archive_manifest']['path']).read_bytes(),saved)

    def test_mutation_cluster_preserves_original_and_partial_without_complete(self):
        for mutation in ['input','destination','added-input','added-destination']:
            root,inputs,terminal,job=self.fixture();real=a.shutil.copyfileobj
            changed=False
            def copy(source,output,*args):
                nonlocal changed
                real(source,output,*args)
                if not changed:
                    changed=True
                    if mutation=='input':(inputs['raw']/'empty').write_bytes(b'actual-later-input')
                    elif mutation=='destination':
                        output.flush();Path(output.name).write_bytes(b'destination-changed')
                    elif mutation=='added-input':(inputs['raw']/'new').write_bytes(b'new')
                    else:(Path(output.name).parent/'unknown').write_bytes(b'unknown')
            with patch.object(a.shutil,'copyfileobj',side_effect=copy):
                with self.assertRaises(ValueError):self.perform(inputs,terminal,job)
            destination=a.ROOT/'archive-controls'/SOURCE/job
            self.assertTrue((destination/'start.manifest.json').exists())
            self.assertTrue((destination/'failure.manifest.json').exists())
            self.assertFalse((destination/'archive.manifest.json').exists())
            self.assertTrue((inputs['elf']/'measured').exists())

    def test_fsync_failure_is_never_a_qualified_archive(self):
        root,inputs,terminal,job=self.fixture()
        with patch.object(a.os,'fsync',side_effect=OSError(5,'controlled sync failure')):
            with self.assertRaises(OSError):self.perform(inputs,terminal,job)
        self.assertFalse((a.ROOT/'archive-controls'/SOURCE/job/'archive.manifest.json').exists())
        self.assertTrue((inputs['raw']/'binary.raw').exists())

    def test_stop_verification_requires_actual_success_and_all_original_bytes(self):
        root,inputs,terminal,job=self.fixture();result=self.perform(inputs,terminal,job)
        path=result['archive_manifest']['path'];sha=result['archive_manifest']['sha256']
        self.assertTrue(a.verify(path,sha,0)['complete'])
        for code in [1,3,124,143]:
            with self.assertRaises(ValueError):a.verify(path,sha,code)
        with self.assertRaises(ValueError):a.verify(path,'0'*64,0)
        (Path(result['destination'])/'payload/raw/empty').write_bytes(b'changed')
        with self.assertRaises(ValueError):a.verify(path,sha,0)

    def test_full_size_native_bytes_bounded_cli_and_immutable_source(self):
        root,inputs,terminal,job=self.fixture()
        with (inputs['raw']/'full64MiB.raw').open('xb') as stream:
            for _ in range(64):stream.write(bytes(range(256))*4096)
        before=a.inventory(inputs)
        command=[sys.executable,'-I',str(S/'native-proof-archive.py'),'--lane','archive-controls','--source',SOURCE,'--job',job,'--terminal-receipt',str(terminal),'--input','raw='+str(inputs['raw']),'--input','elf='+str(inputs['elf']),'--copy-budget-bytes',str(128*1024**2),'--reserve-bytes',str(128*1024**2),'--seconds','60']
        result=subprocess.run(command,capture_output=True,timeout=70)
        self.assertEqual(result.returncode,0,result.stderr)
        reply=json.loads(result.stdout);self.assertEqual(reply['actual_exit'],0);self.assertTrue(reply['complete'])
        self.assertEqual(before,a.inventory(inputs));self.assertEqual(reply['files'],5)
        RESULTS.append(reply)

if __name__=='__main__':unittest.main(verbosity=2)
