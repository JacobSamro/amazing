#!/usr/bin/env python3
"""Whole native lossless corpus capture/restoration controls; never Mac tests."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import uuid

p=argparse.ArgumentParser();p.add_argument('--source',required=True);p.add_argument('--output',required=True);OPTIONS=p.parse_args();S=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('archive',S/'native-proof-archive.py');a=importlib.util.module_from_spec(spec);spec.loader.exec_module(a)
RESULTS=[]
class Controls(unittest.TestCase):
 def fixture(self):
  root=Path(tempfile.mkdtemp(prefix='native-packed-controls.',dir='/tmp'));proof=root/'proof';proof.mkdir();(proof/'command.log').write_bytes(b'whole original failed command raw\n');shutil.copyfile('/usr/bin/true',proof/'measured.elf')
  target=root/'corpus';target.mkdir();(target/'nested').mkdir();(target/'empty').mkdir();(target/'nested'/'Unicode café.txt').write_bytes(bytes(range(256))*256);(target/'large').write_bytes(b'lossless corpus original\n'*(64*1024**2//24));(target/'large').chmod(0o640)
  source=root/'source';source.mkdir();(source/'corpus').symlink_to(target);(source/'Cargo.toml').write_text('[actual source]\n')
  token=uuid.uuid4().hex;terminal=root/'terminal.json';terminal.write_text(json.dumps({'completed':True,'source':OPTIONS.source,'job':token,'actual_exit':1}))
  return root,proof,target,source,terminal,token
 def capture(self,f,budget=16*1024**2,**kwargs):
  root,proof,target,source,terminal,token=f
  return a.archive('packed-controls',OPTIONS.source,token,{'proof':proof,'source':source},terminal,budget,64*1024**2,capture_links=True,targets={'corpus':target},**kwargs)
 def test_real_large_lossless_target_full_restore_hashes_modes_and_all_source_bytes(self):
  f=self.fixture();self.assertGreater(sum(x.stat().st_size for x in f[2].rglob('*') if x.is_file()),16*1024**2)
  with self.assertRaises(OSError):self.capture(f)
  result=self.capture(f,pack_targets=['corpus']);m=json.loads(Path(result['archive_manifest']['path']).read_text());self.assertGreater(m['logical_bytes'],m['required_bytes']);self.assertEqual(m['elf_count'],1)
  verified=a.verify(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0);self.assertEqual(verified['files'],m['file_count']);self.assertTrue((f[3]/'corpus').is_symlink());self.assertEqual((Path(result['destination'])/'payload/proof/command.log').read_bytes(),(f[1]/'command.log').read_bytes())
  restored=a.restore_target(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0,'corpus',f[0]/'restored',128*1024**2,64*1024**2);self.assertTrue(restored['restored']);self.assertTrue((f[0]/'restored/empty').is_dir())
  for original in f[2].rglob('*'):
   if original.is_file():copy=f[0]/'restored'/original.relative_to(f[2]);self.assertEqual(a.digest(original),a.digest(copy));self.assertEqual(original.stat().st_mode&0o777,copy.stat().st_mode&0o777)
  RESULTS.append(result)
 def test_targets_are_explicit_and_real_elf_payloads_never_packed(self):
  f=self.fixture()
  with self.assertRaisesRegex(ValueError,'explicitly declared'):self.capture(f,pack_targets=['source'])
  shutil.copyfile('/usr/bin/true',f[2]/'hidden-elf')
  with self.assertRaisesRegex(ValueError,'ELF'):self.capture(f,pack_targets=['corpus'])
  self.assertFalse((a.ROOT/'packed-controls'/OPTIONS.source/f[5]).exists())
 def test_compressed_capacity_failure_keeps_partial_and_every_original(self):
  f=self.fixture();(f[2]/'large').write_bytes(os.urandom(2*1024**2));before=a.digest(f[2]/'large')
  with self.assertRaises(OSError):self.capture(f,budget=1024**2,pack_targets=['corpus'])
  destination=a.ROOT/'packed-controls'/OPTIONS.source/f[5];self.assertTrue((destination/'failure.manifest.json').exists());self.assertFalse((destination/'archive.manifest.json').exists());self.assertEqual(a.digest(f[2]/'large'),before)
 def test_changed_original_or_link_cannot_publish_complete_capture(self):
  for link in (False,True):
   f=self.fixture();real=a.pack.write_pack
   def changed(*args,**kwargs):
    row=real(*args,**kwargs)
    if link:(f[3]/'corpus').unlink();(f[3]/'corpus').symlink_to(f[1])
    else:(f[2]/'large').write_bytes(b'changed original')
    return row
   with patch.object(a.pack,'write_pack',side_effect=changed):
    with self.assertRaises(ValueError):self.capture(f,pack_targets=['corpus'])
   self.assertFalse((a.ROOT/'packed-controls'/OPTIONS.source/f[5]/'archive.manifest.json').exists())
 def test_compressed_corruption_refuses_verification_and_restore(self):
  f=self.fixture();result=self.capture(f,pack_targets=['corpus']);manifest=Path(result['archive_manifest']['path']);m=json.loads(manifest.read_text());packed=Path(m['packs']['corpus']['path']);data=packed.read_bytes();packed.write_bytes(data[:-1]+bytes([data[-1]^1]))
  with self.assertRaises(ValueError):a.verify(manifest,result['archive_manifest']['sha256'],0)
  with self.assertRaises(ValueError):a.restore_target(manifest,result['archive_manifest']['sha256'],0,'corpus',f[0]/'restored',128*1024**2,64*1024**2)
  self.assertFalse((f[0]/'restored').exists())
 def test_full_restore_capacity_guard_is_not_compressed_size_or_remote_reference(self):
  f=self.fixture();result=self.capture(f,pack_targets=['corpus'])
  with self.assertRaises(OSError):a.restore_target(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0,'corpus',f[0]/'restored',1024**2,64*1024**2)
  self.assertFalse((f[0]/'restored').exists());self.assertTrue((f[2]/'large').exists())
 def test_omitted_duplicate_escaping_link_and_wrong_byte_members_reject(self):
  f=self.fixture();rows,dirs,_=a.inventory_details({'corpus':f[2]});path=f[0]/'bad.tar.gz'
  for kind in ('omitted','duplicate','escape','link','wrongbytes'):
   with tarfile.open(path,'w:gz') as t:
    if kind=='duplicate':
     for _ in range(2):m=tarfile.TarInfo('.');m.type=tarfile.DIRTYPE;t.addfile(m)
    elif kind=='escape':t.addfile(tarfile.TarInfo('../outside'))
    elif kind=='link':m=tarfile.TarInfo('large');m.type=tarfile.SYMTYPE;m.linkname='/etc/passwd';t.addfile(m)
    elif kind=='wrongbytes':t.addfile(tarfile.TarInfo('large'))
   with self.subTest(kind=kind):
    with self.assertRaises(ValueError):a.pack.readback(path,rows,dirs)
 def test_both_packed_and_legacy_verifier_contracts_preserve_actual_failed_exit(self):
  for packed in (False,True):
   f=self.fixture()
   if not packed:(f[2]/'large').write_bytes(b'small ordinary legacy')
   result=self.capture(f,pack_targets=['corpus'] if packed else []);manifest=Path(result['archive_manifest']['path']);m=json.loads(manifest.read_text());self.assertEqual(m['terminal_producer']['actual_exit'],1);self.assertTrue(a.verify(manifest,result['archive_manifest']['sha256'],0)['complete'])
   for exit in (1,3,124):
    with self.assertRaises(ValueError):a.verify(manifest,result['archive_manifest']['sha256'],exit)
class Results(unittest.TextTestResult):
 def __init__(self,*x,**kw):super().__init__(*x,**kw);self.classifications={}
 def addSuccess(self,t):super().addSuccess(t);self.classifications[t._testMethodName]='pass'
 def addFailure(self,t,e):super().addFailure(t,e);self.classifications[t._testMethodName]='fail'
 def addError(self,t,e):super().addError(t,e);self.classifications[t._testMethodName]='error'
r=unittest.TextTestRunner(verbosity=2,resultclass=Results).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls));row={'source':OPTIONS.source,'tests_run':r.testsRun,'failed':len(r.failures),'errors':len(r.errors),'actual_exit':int(not r.wasSuccessful()),'classifications':r.classifications,'captures':RESULTS,'diagnostics':{t.id():e[-4096:] for t,e in r.failures+r.errors}};Path(OPTIONS.output).write_text(json.dumps(row,separators=(',',':'))+'\n');print(json.dumps(row,separators=(',',':')));raise SystemExit(row['actual_exit'])
