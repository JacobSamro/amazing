#!/usr/bin/env python3
import copy,importlib.util,json,pathlib,sys,unittest
s=importlib.util.spec_from_file_location('pair',pathlib.Path(__file__).with_name('html-constructor-wpt-pair.py'));P=importlib.util.module_from_spec(s);s.loader.exec_module(P)
def measurement(role):
 return {'role':role,'source':'a'*40 if role=='before' else 'b'*40,'pin':P.C.PIN,'input_sha256':'i',
  'cases':[['native.html','sha'],['error.html','other']],'runner_sha256':'r','selectors':['dom','html/dom','custom-elements'],
  'jobs':4,'timeout_ms':20000,'viewport':[800,600],'records':[
   {'file':'native.html','status':'pass','subtests':[{'name':'same','occurrence':0,'status':'pass'},{'name':'same','occurrence':1,'status':'pass'}]},
   {'file':'error.html','status':'error','subtests':[{'name':None,'occurrence':0,'status':'error'}]}]}
class Controls(unittest.TestCase):
 def test_original_errors_are_measured_not_all_pass_precondition(self):self.assertTrue(P.compare(measurement('before'),measurement('after'))['zero_loss'])
 def test_actual_duplicate_variant_loss_blocks(self):
  b,a=measurement('before'),measurement('after');a['records'][0]['subtests'].pop();v=P.compare(b,a)
  self.assertFalse(v['zero_loss']);self.assertEqual(v['lost_subtests'],[['native.html','same',1]])
 def test_original_passing_file_loss_blocks_even_when_subtests_remain(self):
  a=measurement('after');a['records'][0]['status']='fail';self.assertEqual(P.compare(measurement('before'),a)['lost_files'],['native.html'])
 def test_genuine_new_named_pass_is_a_gain(self):
  a=measurement('after');a['records'][1]['subtests'].append({'name':'new','occurrence':0,'status':'pass'})
  v=P.compare(measurement('before'),a);self.assertTrue(v['zero_loss']);self.assertEqual(v['gained_subtests'],[['error.html','new',0]])
 def test_profile_runner_pin_or_case_drift_rejected(self):
  for key,value in [('pin','0'*40),('jobs',8),('timeout_ms',25000),('viewport',[1,1]),('runner_sha256','foreign'),('cases',[])]:
   a=measurement('after');a[key]=value
   with self.assertRaises(AssertionError):P.compare(measurement('before'),a)
 def test_missing_or_duplicate_case_measurement_rejected(self):
  for rows in [[],[measurement('after')['records'][0]]*2]:
   a=measurement('after');a['records']=rows
   with self.assertRaises(AssertionError):P.compare(measurement('before'),a)
if __name__=='__main__':
 r=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls))
 pathlib.Path(sys.argv[1]).write_text(json.dumps({'fixture_only':True,'controls':r.testsRun,'failed':len(r.failures)+len(r.errors)})+'\n')
 raise SystemExit(not r.wasSuccessful())
