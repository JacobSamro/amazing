#!/usr/bin/env python3
"""Actual native lossless plain content aliases and capacity diagnostics."""
import argparse
import importlib.util
import json
from pathlib import Path
import os
import shutil
import tempfile
import unittest
from unittest.mock import patch
import uuid

p=argparse.ArgumentParser();p.add_argument('--source',required=True);p.add_argument('--output',required=True);OPTIONS=p.parse_args();S=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('a',S/'native-proof-archive.py');a=importlib.util.module_from_spec(spec);spec.loader.exec_module(a)
RESULTS=[]
class Controls(unittest.TestCase):
 def fixture(self):
  base=Path(tempfile.mkdtemp(prefix='native-content.',dir='/tmp'));left=base/'left';right=base/'right';left.mkdir();right.mkdir();data=b'actual original duplicate bytes\n'*(16*1024**2//32);(left/'large').write_bytes(data);(right/'large').write_bytes(data);shutil.copyfile('/usr/bin/true',left/'elf');shutil.copyfile('/usr/bin/true',right/'elf');job=uuid.uuid4().hex;terminal=base/'terminal.json';terminal.write_text(json.dumps({'completed':True,'source':OPTIONS.source,'job':job,'actual_exit':1}));return base,left,right,terminal,job
 def capture(self,f,**kw):return a.archive('content-controls',OPTIONS.source,f[4],{'left':f[1],'right':f[2]},f[3],20*1024**2,64*1024**2,**kw)
 def test_actual_duplicate_bytes_plain_elf_paths_new_inode_and_all_original_metadata(self):
  f=self.fixture();before={str(p):a.fingerprint(p.stat()) for d in f[1:3] for p in d.iterdir()}
  with self.assertRaises(OSError):self.capture(f)
  result=self.capture(f,deduplicate_content=True);m=json.loads(Path(result['archive_manifest']['path']).read_text());self.assertEqual(len(m['content_aliases']),2);self.assertGreater(m['logical_bytes'],m['required_bytes'])
  for name in ('large','elf'):
   left=Path(result['destination'])/'payload/left'/name;right=Path(result['destination'])/'payload/right'/name;self.assertEqual(left.stat().st_ino,right.stat().st_ino);self.assertNotEqual(left.stat().st_ino,(f[1]/name).stat().st_ino);self.assertEqual(a.digest(left),a.digest(f[1]/name));self.assertEqual(left.stat().st_mode&0o777,(f[1]/name).stat().st_mode&0o777)
  self.assertEqual(before,{str(p):a.fingerprint(p.stat()) for d in f[1:3] for p in d.iterdir()});self.assertTrue(a.verify(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0)['complete']);RESULTS.append(result)
 def test_readonly_exact_perlabel_plain_candidate_unique_and_no_created_archive(self):
  f=self.fixture();r=a.capacity_plan('content-controls',OPTIONS.source,f[4],{'left':f[1],'right':f[2]},f[3],20*1024**2,64*1024**2,deduplicate=True);facts=r['capacity'];self.assertEqual(facts['logical_bytes'],sum(p.stat().st_size for d in f[1:3] for p in d.iterdir()));self.assertEqual(facts['plain_bytes'],facts['logical_bytes']);self.assertEqual(facts['candidate_unique_plain_bytes']*2,facts['plain_bytes']);self.assertEqual(facts['labels']['left']['elf_files'],1);self.assertFalse(r['archive_complete']);self.assertFalse((a.ROOT/'content-controls'/OPTIONS.source/f[4]).exists())
 def test_different_modes_never_share_content_or_understate_metadata_identity(self):
  f=self.fixture();(f[2]/'large').chmod(0o600)
  with self.assertRaises(OSError):self.capture(f,deduplicate_content=True)
  self.assertFalse((a.ROOT/'content-controls'/OPTIONS.source/f[4]).exists())
 def test_source_mutation_during_byte_comparison_preserves_partial_no_completion(self):
  f=self.fixture();real=a.content.equal_bytes
  def mutate(original,captured):
   result=real(original,captured);original.write_bytes(b'changed original fixture');return result
  with patch.object(a.content,'equal_bytes',side_effect=mutate),self.assertRaises(ValueError):self.capture(f,deduplicate_content=True)
  dest=a.ROOT/'content-controls'/OPTIONS.source/f[4];self.assertTrue((dest/'failure.manifest.json').exists());self.assertFalse((dest/'archive.manifest.json').exists())
 def test_same_hash_candidate_requires_real_byte_comparison_no_collision_success(self):
  f=self.fixture()
  with patch.object(a.content,'equal_bytes',return_value=False),self.assertRaisesRegex(ValueError,'collision'):self.capture(f,deduplicate_content=True)
  self.assertFalse((a.ROOT/'content-controls'/OPTIONS.source/f[4]/'archive.manifest.json').exists())
 def test_link_unsupported_fails_closed_without_budgeted_copy_fallback(self):
  f=self.fixture();real=os.link
  def links(source,dest,*args,**kwargs):
   if '/payload/' in str(dest):raise OSError(95,'native hardlink unsupported')
   return real(source,dest,*args,**kwargs)
  with patch.object(a.os,'link',side_effect=links),self.assertRaises(OSError):self.capture(f,deduplicate_content=True)
  self.assertTrue((f[1]/'elf').exists());self.assertFalse((a.ROOT/'content-controls'/OPTIONS.source/f[4]/'archive.manifest.json').exists())
 def test_actual_alias_corruption_rejects_full_verification_every_plain_member(self):
  f=self.fixture();result=self.capture(f,deduplicate_content=True);path=Path(result['destination'])/'payload/right/elf';path.write_bytes(b'corrupt captured alias')
  with self.assertRaises(ValueError):a.verify(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0)
  self.assertEqual(a.digest(f[1]/'elf'),a.digest(f[2]/'elf'))
 def test_mixed_data_dedup_elf_plain_and_full_restoration_count_every_member(self):
  f=self.fixture();target=f[0]/'target';target.mkdir();shutil.copyfile('/usr/bin/true',target/'first');shutil.copyfile('/usr/bin/true',target/'second');(target/'data').write_bytes(b'lossless corpus bytes'*1024)
  result=a.archive('content-controls',OPTIONS.source,f[4],{'left':f[1]},f[3],20*1024**2,64*1024**2,targets={'target':target},pack_data_targets=['target'],deduplicate_content=True);m=json.loads(Path(result['archive_manifest']['path']).read_text());self.assertEqual(m['elf_count'],3);restored=a.restore_target(result['archive_manifest']['path'],result['archive_manifest']['sha256'],0,'target',f[0]/'restored',4*1024**2,64*1024**2);self.assertEqual(restored['files'],3);self.assertEqual(restored['plain_elf_files'],2)
  for original in target.iterdir():self.assertEqual(a.digest(original),a.digest(f[0]/'restored'/original.name))
class Results(unittest.TextTestResult):
 def __init__(self,*args,**kwargs):super().__init__(*args,**kwargs);self.classifications={}
 def addSuccess(self,t):super().addSuccess(t);self.classifications[t._testMethodName]='pass'
 def addFailure(self,t,e):super().addFailure(t,e);self.classifications[t._testMethodName]='fail'
 def addError(self,t,e):super().addError(t,e);self.classifications[t._testMethodName]='error'
r=unittest.TextTestRunner(verbosity=2,resultclass=Results).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls));row={'source':OPTIONS.source,'tests_run':r.testsRun,'failed':len(r.failures),'errors':len(r.errors),'actual_exit':int(not r.wasSuccessful()),'classifications':r.classifications,'captures':RESULTS,'diagnostics':{t.id():e[-4096:] for t,e in r.failures+r.errors}};Path(OPTIONS.output).write_text(json.dumps(row,separators=(',',':'))+'\n');print(json.dumps(row,separators=(',',':')));raise SystemExit(row['actual_exit'])
