#!/usr/bin/env python3
"""Native mixed corpus DATA packing with every ELF independently readable."""
import argparse
import importlib.util
import json
from pathlib import Path
import os
import shutil
import tempfile
import unittest
import uuid
from unittest.mock import patch

p=argparse.ArgumentParser();p.add_argument('--source',required=True);p.add_argument('--output',required=True);OPTIONS=p.parse_args();S=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('a',S/'native-proof-archive.py');a=importlib.util.module_from_spec(spec);spec.loader.exec_module(a)
RESULTS=[]
class Controls(unittest.TestCase):
 def fixture(self):
  root=Path(tempfile.mkdtemp(prefix='native-mixed.',dir='/tmp'));proof=root/'proof';proof.mkdir();(proof/'whole.log').write_bytes(b'whole original authentic failed raw\n');shutil.copyfile('/usr/bin/true',proof/'measured.elf')
  target=root/'corpus';target.mkdir();(target/'nested').mkdir();(target/'empty').mkdir();shutil.copyfile('/usr/bin/true',target/'nested'/'executable');(target/'nested'/'executable').chmod(0o755);(target/'large-data').write_bytes(b'all real corpus bytes\n'*(16*1024**2//22));(target/'nested'/'Unicode café').write_bytes(bytes(range(256)));(target/'alias').symlink_to('nested/executable')
  source=root/'source';source.mkdir();(source/'corpus').symlink_to(target);(source/'Cargo.toml').write_text('actual source\n');job=uuid.uuid4().hex;terminal=root/'terminal.json';terminal.write_text(json.dumps({'completed':True,'source':OPTIONS.source,'job':job,'actual_exit':1}));return root,proof,target,source,terminal,job
 def capture(self,f,budget=2*1024**2):return a.archive('mixed-controls',OPTIONS.source,f[5],{'proof':f[1],'source':f[3]},f[4],budget,64*1024**2,capture_links=True,targets={'corpus':f[2]},pack_data_targets=['corpus'])
 def test_complete_corpus_data_packed_all_elf_plain_and_full_native_restoration(self):
  f=self.fixture();result=self.capture(f);manifest=Path(result['archive_manifest']['path']);m=json.loads(manifest.read_text());self.assertEqual(m['data_packed_targets'],['corpus']);self.assertEqual(m['elf_count'],2);elf=Path(result['destination'])/'payload/corpus/nested/executable';self.assertEqual(a.digest(elf),a.digest(f[2]/'nested/executable'));self.assertEqual(elf.stat().st_mode&0o777,0o755)
  self.assertEqual(a.verify(manifest,result['archive_manifest']['sha256'],0)['files'],m['file_count']);restored=a.restore_target(manifest,result['archive_manifest']['sha256'],0,'corpus',f[0]/'restored',64*1024**2,64*1024**2);self.assertEqual(restored['plain_elf_files'],1)
  for original in f[2].rglob('*'):
   if original.is_file() and not original.is_symlink():copy=f[0]/'restored'/original.relative_to(f[2]);self.assertEqual(a.digest(original),a.digest(copy));self.assertEqual(original.stat().st_mode&0o777,copy.stat().st_mode&0o777)
  self.assertEqual(restored['link_metadata'][0]['target'],'nested/executable');RESULTS.append(result)
 def test_original_strict_rejection_no_mode_conflict_or_undeclared_target_escape(self):
  f=self.fixture()
  for kwargs in ({'pack_targets':['corpus']},{'pack_targets':['corpus'],'pack_data_targets':['corpus']},{'pack_data_targets':['source']}):
   with self.assertRaises(ValueError):a.archive('mixed-controls',OPTIONS.source,f[5],{'proof':f[1],'source':f[3]},f[4],2*1024**2,64*1024**2,capture_links=True,targets={'corpus':f[2]},**kwargs)
  self.assertFalse((a.ROOT/'mixed-controls'/OPTIONS.source/f[5]).exists())
 def test_plain_elf_mismatch_missing_and_mode_corruption_reject_every_verifier(self):
  for kind in ('bytes','missing','mode'):
   f=self.fixture();result=self.capture(f);file=Path(result['destination'])/'payload/corpus/nested/executable'
   if kind=='bytes':file.write_bytes(b'wrong native ELF')
   elif kind=='missing':file.unlink()
   else:file.chmod(0o600)
   with self.subTest(kind=kind),self.assertRaises(ValueError):a.verify(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0)
 def test_nonelf_pack_corruption_keeps_plain_elf_but_never_qualifies(self):
  f=self.fixture();result=self.capture(f);manifest=Path(result['archive_manifest']['path']);m=json.loads(manifest.read_text());path=Path(m['packs']['corpus']['path']);path.write_bytes(b'not complete gzip data')
  with self.assertRaises(ValueError):a.verify(manifest,result['archive_manifest']['sha256'],0)
  self.assertEqual(a.digest(Path(result['destination'])/'payload/corpus/nested/executable'),a.digest(f[2]/'nested/executable'))
 def test_actual_plain_elf_and_data_mutation_both_block_complete_publication(self):
  for which in ('nested/executable','large-data'):
   f=self.fixture();real=a.pack.write_pack
   def mutate(*args,**kwargs):row=real(*args,**kwargs);(f[2]/which).write_bytes(b'author changed original corpus');return row
   with patch.object(a.pack,'write_pack',side_effect=mutate),self.assertRaises(ValueError):self.capture(f)
   self.assertFalse((a.ROOT/'mixed-controls'/OPTIONS.source/f[5]/'archive.manifest.json').exists())
 def test_plain_native_elf_bytes_count_in_actual_capacity_and_stay_original(self):
  f=self.fixture();elf=f[2]/'nested/executable';elf.write_bytes(b'\x7fELF'+os.urandom(3*1024**2));before=a.digest(elf)
  with self.assertRaises(OSError):self.capture(f)
  self.assertEqual(a.digest(elf),before);self.assertFalse((a.ROOT/'mixed-controls'/OPTIONS.source/f[5]).exists())
 def test_empty_data_subset_still_keeps_full_plain_elf_and_directories(self):
  f=self.fixture();(f[2]/'large-data').unlink();(f[2]/'nested'/'Unicode café').unlink();result=self.capture(f);manifest=Path(result['archive_manifest']['path']);m=json.loads(manifest.read_text());self.assertEqual(m['packs']['corpus']['logical_bytes'],0);self.assertTrue(a.verify(manifest,result['archive_manifest']['sha256'],0)['complete']);restored=a.restore_target(manifest,result['archive_manifest']['sha256'],0,'corpus',f[0]/'restored',2*1024**2,64*1024**2);self.assertEqual(restored['files'],1);self.assertTrue((f[0]/'restored/empty').is_dir())
 def test_full_restore_guards_original_total_not_compressed_data_size(self):
  f=self.fixture();result=self.capture(f)
  with self.assertRaises(OSError):a.restore_target(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0,'corpus',f[0]/'restored',2*1024**2,64*1024**2)
  self.assertFalse((f[0]/'restored').exists());self.assertTrue((f[2]/'large-data').exists())
class Results(unittest.TextTestResult):
 def __init__(self,*args,**kwargs):super().__init__(*args,**kwargs);self.classifications={}
 def addSuccess(self,t):super().addSuccess(t);self.classifications[t._testMethodName]='pass'
 def addFailure(self,t,e):super().addFailure(t,e);self.classifications[t._testMethodName]='fail'
 def addError(self,t,e):super().addError(t,e);self.classifications[t._testMethodName]='error'
r=unittest.TextTestRunner(verbosity=2,resultclass=Results).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls));row={'source':OPTIONS.source,'tests_run':r.testsRun,'failed':len(r.failures),'errors':len(r.errors),'actual_exit':int(not r.wasSuccessful()),'classifications':r.classifications,'captures':RESULTS,'diagnostics':{t.id():e[-4096:] for t,e in r.failures+r.errors}};Path(OPTIONS.output).write_text(json.dumps(row,separators=(',',':'))+'\n');print(json.dumps(row,separators=(',',':')));raise SystemExit(row['actual_exit'])
