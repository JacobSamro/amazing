#!/usr/bin/env python3
"""Run the whole public job-control suite; report genuine native tooling results."""
import argparse,hashlib,importlib.util,json,pathlib,sys,unittest

def main():
 p=argparse.ArgumentParser();p.add_argument('--source',required=True);p.add_argument('--output',required=True);a=p.parse_args()
 scripts=pathlib.Path(__file__).resolve().parent
 class Results(unittest.TextTestResult):
  def __init__(self,*args,**kwargs):super().__init__(*args,**kwargs);self.controls=[]
  def addSuccess(self,test):super().addSuccess(test);self.controls.append({'id':test.id(),'classification':'pass'})
  def addFailure(self,test,error):super().addFailure(test,error);self.controls.append({'id':test.id(),'classification':'fail'})
  def addError(self,test,error):super().addError(test,error);self.controls.append({'id':test.id(),'classification':'error'})
  def addSubTest(self,test,subtest,error):
   super().addSubTest(test,subtest,error)
   if error is not None:self.controls.append({'id':subtest.id(),'classification':'fail' if issubclass(error[0],test.failureException) else 'error'})
  def addSkip(self,test,reason):super().addSkip(test,reason);self.controls.append({'id':test.id(),'classification':'skip','reason':reason})
 suite=unittest.TestSuite()
 for path in sorted(scripts.glob('completed-job*tests.py')):
  spec=importlib.util.spec_from_file_location(path.stem,path);module=importlib.util.module_from_spec(spec);sys.modules[path.stem]=module;spec.loader.exec_module(module)
  suite.addTests(unittest.TestLoader().loadTestsFromModule(module))
 if not suite.countTestCases():raise RuntimeError('Whole public completed-job control suite is empty')
 result=unittest.TextTestRunner(verbosity=2,resultclass=Results).run(suite)
 receipts=[]
 for name,module in list(sys.modules.items()):
  if name=='completed-job-diagnostic-tests':receipts.extend(module.CONTROL_RECEIPTS)
 summary={'schema':1,'source':a.source,'scope':'Native tooling controls ONLY; real owned children, controlled diagnostic faults, no engine gate or provider lifecycle changes','counts':{'tests':result.testsRun,'passed':sum(r['classification']=='pass' for r in result.controls),'failed':len(result.failures),'errors':len(result.errors),'skipped':len(result.skipped)},'controls':result.controls,'native_control_receipts':receipts,'public_sources':{f.name:{'bytes':f.stat().st_size,'sha256':hashlib.sha256(f.read_bytes()).hexdigest()} for f in sorted(scripts.glob('completed-job*.py'))},'actual_suite_exit':int(not result.wasSuccessful())}
 out=pathlib.Path(a.output);out.parent.mkdir(parents=True,exist_ok=True);out.write_text(json.dumps(summary,separators=(',',':'))+'\n')
 print(json.dumps({'summary':str(out),'counts':summary['counts'],'actual_exit':summary['actual_suite_exit']}),flush=True)
 return summary['actual_suite_exit']

if __name__=='__main__':sys.exit(main())
