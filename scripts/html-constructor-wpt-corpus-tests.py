#!/usr/bin/env python3
"""Native caller protocol fixtures; never alleged engine/WPT passes."""
import importlib.util,json,pathlib,tempfile,unittest,sys
spec=importlib.util.spec_from_file_location('corpus',pathlib.Path(__file__).with_name('html-constructor-wpt-corpus.py'))
C=importlib.util.module_from_spec(spec);spec.loader.exec_module(C)
CASE='explicit-caller-fixture.html'
def report(subs,status='fail'):
 counts={key:0 for key in ['pass','fail','timeout','notrun','error']}
 for name,code in subs:counts[['pass','fail','timeout','notrun'][code] if type(code) is int and code in [0,1,2,3] else 'error']+=1
 return {'test':CASE,'wpt_revision':C.PIN,'status':status,'harness_status':0,
  'counts':counts,'subtests':[{'name':name,'status':code} for name,code in subs]}
class Controls(unittest.TestCase):
 def test_original_named_pass_file_and_duplicate_subtest_identity(self):
  r=C.classify(CASE,report([('same',0),('same',0)],'pass'),0);files,subs=C.passing([r])
  self.assertEqual(files,{CASE});self.assertEqual(subs,{(CASE,'same',0),(CASE,'same',1)})
 def test_pinned_optional_precondition_failure_is_retained_without_promotion(self):
  r=C.classify(CASE,report([('optional',4),('good',0)]),1)
  self.assertEqual(r['subtests'][0]['status'],'precondition_failed');self.assertFalse(r['protocol_errors'])
  self.assertEqual(C.passing([r])[1],{(CASE,'good',0)})
 def test_native_nullable_name_is_not_invented_or_passed(self):
  r=C.classify(CASE,report([(None,0),('good',0)]),1)
  self.assertIsNone(r['subtests'][0]['name']);self.assertEqual(r['subtests'][0]['status'],'error')
  self.assertEqual(C.passing([r]),(set(),{(CASE,'good',0)}))
 def test_invalid_native_status_and_bool_keep_exact_original_value(self):
  for code in [-1,99,None,True,'0']:
   r=C.classify(CASE,report([('bad',code),('good',0)]),1)
   self.assertEqual(r['subtests'][0]['native_status'],code);self.assertEqual(r['subtests'][0]['status'],'error')
   self.assertEqual(C.passing([r])[1],{(CASE,'good',0)})
 def test_invalid_name_shapes_never_replace_native_values(self):
  for name in [False,22,[],{'actual':'native'}]:
   r=C.classify(CASE,report([(name,1)]),1)
   self.assertEqual(r['subtests'][0]['name'],name);self.assertFalse(C.passing([r])[1])
 def test_inconsistent_native_pass_claim_is_error(self):
  r=C.classify(CASE,report([('bad',1)],'pass'),0);self.assertEqual(r['status'],'error');self.assertFalse(C.passing([r])[0])
 def test_counts_mismatch_does_not_abort_remaining_measurements(self):
  value=report([('good',0)]);value['counts']['pass']=9
  r=C.classify(CASE,value,1);self.assertEqual(r['status'],'error');self.assertIn('native_count_mismatch:pass',r['protocol_errors'])
 def test_noninteger_counts_are_not_accepted(self):
  v=report([('good',0)]);v['counts']['pass']=True
  self.assertIn('native_count_mismatch:pass',C.classify(CASE,v,1)['protocol_errors'])
 def test_foreign_pin_or_case_has_no_qualifying_subtest_identity(self):
  for field,value in [('wpt_revision','0'*40),('wpt_revision',None),('test','foreign.html')]:
   v=report([('good',0)]);v[field]=value;self.assertFalse(C.passing([C.classify(CASE,v,1)])[1])
 def test_actual_native_exit_disagreement_is_preserved(self):
  r=C.classify(CASE,report([('good',0)],'pass'),101);self.assertEqual(r['actual_exit'],101);self.assertEqual(r['status'],'error')
 def test_empty_native_pass_has_no_nonempty_qualifying_file_pass(self):
  r=C.classify(CASE,report([],'pass'),0);self.assertEqual(r['status'],'pass');self.assertEqual(C.passing([r]),(set(),set()))
 def test_missing_and_nonobject_reports_remain_errors(self):
  for v in [None,[],7]:self.assertEqual(C.classify(CASE,v,101)['status'],'error')
 def test_deadline_remains_recorded_at_same_profile_without_pass(self):
  r=C.classify(CASE,None,-9,True);self.assertEqual(r['status'],'timeout');self.assertEqual(r['actual_exit'],-9);self.assertFalse(C.passing([r])[1])
 def test_lone_surrogate_names_keep_exact_identity(self):
  name='exact\ud800';r=C.classify(CASE,report([(name,0)],'pass'),0)
  self.assertEqual(C.passing([r])[1],{(CASE,name,0)})
 def test_real_child_report_is_persisted_before_schema_interpretation(self):
  with tempfile.TemporaryDirectory(prefix='amazing-caller-controls-',dir='/tmp') as t:
   root=pathlib.Path(t);binary=root/'explicit-protocol-fixture';v=report([(None,-1),('good',0)])
   binary.write_text('#!/usr/bin/env python3\nimport json,sys\nprint('+repr(json.dumps(v))+')\nsys.exit(1)\n');binary.chmod(0o700)
   r=C.capture(0,CASE,binary,root,root,'f'*40,C.digest(binary.read_bytes()))
   raw=json.loads((root/'00000.record.json').read_text());self.assertEqual(raw['report'],v);self.assertEqual(raw['actual_exit'],1)
   self.assertIsNone(raw['report']['subtests'][0]['name']);self.assertEqual(r['raw']['record']['sha256'],C.digest((root/'00000.record.json').read_bytes()))
   self.assertEqual(C.passing([r])[1],{(CASE,'good',0)})
if __name__=='__main__':
 result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls))
 pathlib.Path(sys.argv[1]).write_text(json.dumps({'fixture_only':True,'controls':result.testsRun,'failed':len(result.errors)+len(result.failures),'failed_ids':[t.id() for t,_ in result.errors+result.failures]})+'\n')
 raise SystemExit(not result.wasSuccessful())
