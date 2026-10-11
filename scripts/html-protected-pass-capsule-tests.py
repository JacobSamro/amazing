#!/usr/bin/env python3
import base64,collections,hashlib,importlib.util,json,pathlib,sys,unittest,zlib
s=importlib.util.spec_from_file_location('capsule',pathlib.Path(__file__).with_name('html-protected-pass-capsule.py'));P=importlib.util.module_from_spec(s);s.loader.exec_module(P)
def setup():
 names=['exact\ud800','same','same'];counter=collections.Counter(hashlib.sha256(x.encode('utf-16le','surrogatepass')).hexdigest() for x in names)
 rows={'core':[{'test':'real.html','status':'fail','subtests':[{'name':x,'status':0} for x in names]+[{'name':'old failure','status':1}]}]}
 old={'core':[{'test':'real.html','status':'fail','passing_count':3,'passing_counter_sha256':P.C.digest(json.dumps(counter,sort_keys=True,separators=(',',':')).encode())}]}
 return {'records_zlib_base64':base64.b64encode(zlib.compress(json.dumps(old).encode())).decode()},rows
class Controls(unittest.TestCase):
 def test_exact_utf16_duplicate_pass_multiplicities_and_original_failures(self):
  m,r=setup();v=P.project(m,r);self.assertEqual(v['core'][0]['passing'],{'exact\ud800':1,'same':2});self.assertEqual(v['core'][0]['status'],'fail')
 def test_changed_passing_name_or_multiplicity_rejected(self):
  for field,value in [('name','foreign'),('status',1)]:
   m,r=setup();r['core'][0]['subtests'][0][field]=value
   with self.assertRaises(ValueError):P.project(m,r)
 def test_original_case_or_classification_or_inventory_drift_rejected(self):
  for kind in ['case','status','inventory','suite']:
   m,r=setup()
   if kind=='case':r['core'][0]['test']='foreign'
   if kind=='status':r['core'][0]['status']='pass'
   if kind=='inventory':r['core'].append(r['core'][0])
   if kind=='suite':r['foreign']=r.pop('core')
   with self.assertRaises(ValueError):P.project(m,r)
if __name__=='__main__':
 r=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls))
 pathlib.Path(sys.argv[1]).write_text(json.dumps({'fixture_only':True,'controls':r.testsRun,'failed':len(r.failures)+len(r.errors)})+'\n')
 raise SystemExit(not r.wasSuccessful())
