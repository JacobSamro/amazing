#!/usr/bin/env python3
"""Real native supervisor/phase fixtures for both diagnostic completion schemas.

Compiler text is an explicit contract fixture, not an engine qualification.
No previous engine job or retained raw archive is read by these controls.
"""
import copy, hashlib, importlib.util, json, os, pathlib, subprocess, sys, tempfile, unittest, uuid
from unittest import mock
ROOT=pathlib.Path(__file__).resolve().parent

def module(name):
    spec=importlib.util.spec_from_file_location(name.replace('-','_'),ROOT/(name+'.py'))
    result=importlib.util.module_from_spec(spec);spec.loader.exec_module(result);return result
J=module('completed-job');D=module('wrapped-failure-diagnostic');F=module('gate-failure-diagnostic')
SOURCE='a'*40

class Admission(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='amazing-completion-admission-')
        self.addCleanup(self.temp.cleanup);self.root=pathlib.Path(self.temp.name)
        self.home=self.root/'home';self.home.mkdir();self.phase=self.root/'build.phase.manifest.json'
        self.child=self.root/'child.py'
        self.child.write_text('import importlib.util,sys\ns=importlib.util.spec_from_file_location("phase",'+repr(str(ROOT/'gate-command.py'))+');g=importlib.util.module_from_spec(s);s.loader.exec_module(g)\ncode=g.run([sys.executable,"-c",'+repr('import sys;sys.stderr.write("error[E0308]: controlled type mismatch\\n");sys.exit(101)')+'],"build",'+repr(SOURCE)+','+repr(str(self.root/'build.log'))+','+repr(str(self.phase))+')\nsys.exit(100 if code["wrapper_exit"]==101 else 3)\n')
        command='exec '+__import__('shlex').quote(sys.executable)+' '+__import__('shlex').quote(str(self.child))
        self.endpoint={'schema':2,'lane':'diagnostic_fixture','source_commit':SOURCE,'box_id':'bx_fixture',
            'host':'user@127.0.0.1','port':1234,'job_id':uuid.uuid4().hex,
            'output_root':'amazing/out/diagnostic_fixture/control-'+uuid.uuid4().hex,
            'command_sha256':J.digest(command.encode()),'execution_sha256':J.digest(command.encode()),'source_archive_sha256':'c'*64}
        program='import importlib.util,sys;s=importlib.util.spec_from_file_location("job",'+repr(str(ROOT/'completed-job.py'))+');j=importlib.util.module_from_spec(s);s.loader.exec_module(j);sys.exit(j.run('+repr(self.endpoint)+','+repr(command)+'))'
        child=subprocess.run([sys.executable,'-I','-c',program],env=dict(os.environ,HOME=str(self.home)),capture_output=True)
        self.assertEqual(child.returncode,100,child.stderr.decode())
        self.marker_path=self.home/('amazing-completed-'+self.endpoint['job_id']+'.json')
        self.marker=json.loads(self.marker_path.read_text())
        with mock.patch.object(pathlib.Path,'home',return_value=self.home): self.observation=J.observe(self.endpoint)
        self.assertEqual(self.marker['actual_exit'],100);self.assertEqual(self.observation['actual_exit'],100)
        self.receipt=json.loads(self.phase.read_text());self.assertEqual(self.receipt['command_exit'],101)
        self.inputs=self.root/'completion.json'
    def admission(self, record):
        return J.diagnostic_completion(record,SOURCE,self.endpoint['job_id'])
    def wrapped(self,record,sha=None):
        self.inputs.write_text(json.dumps(record))
        return json.loads(D.produce(self.inputs,sha or J.digest(self.inputs.read_bytes()),self.phase,J.digest(self.phase.read_bytes()),SOURCE,self.endpoint['job_id'],self.root))
    def test_actual_native_marker_and_public_observation_both_preserve_distinct_exit_facts(self):
        original=self.marker_path.read_bytes()
        for record,form in [(self.marker,'native-completion-marker'),(self.observation,'public-observation-summary')]:
            with self.subTest(form=form):
                endpoint,actual,kind=self.admission(record);self.assertEqual(endpoint,self.endpoint);self.assertEqual((actual,kind),(100,form))
                result=self.wrapped(record);self.assertEqual(result['original_completion_format'],form);self.assertEqual(result['original_actual_exit'],100);self.assertEqual(result['phase_actual_exit'],101);self.assertFalse(result['tests_rerun']);self.assertFalse(result['counts_reconstructed'])
        self.assertEqual(self.marker_path.read_bytes(),original)
    def test_native_cli_consumes_original_marker_without_observation_rewrite(self):
        output=self.root/'cli.compact.json';original=self.marker_path.read_bytes()
        command=[sys.executable,str(ROOT/'wrapped-failure-diagnostic.py'),'--terminal',str(self.marker_path),'--terminal-sha256',J.digest(original),'--phase-receipt',str(self.phase),'--phase-receipt-sha256',J.digest(self.phase.read_bytes()),'--source',SOURCE,'--job',self.endpoint['job_id'],'--native-owner',str(self.root),'--output',str(output)]
        result=subprocess.run(command,capture_output=True);self.assertEqual(result.returncode,0,result.stderr.decode());self.assertEqual(self.marker_path.read_bytes(),original)
        self.assertEqual(json.loads(output.read_text())['original_completion_format'],'native-completion-marker')
    def test_gate_diagnostic_uses_same_original_completion_contract(self):
        raw=json.loads(self.phase.read_text())['raw_native'];summary={'source_commit':SOURCE,'job_exit':100,'phases':[{'phase':'build','source_commit':SOURCE,'actual_returncode':101,'raw':raw}]};p=self.root/'gate.summary.json';p.write_text(json.dumps(summary))
        for record in [self.marker,self.observation]:
            self.inputs.write_text(json.dumps(record));result=json.loads(F.extract(p,J.digest(p.read_bytes()),self.inputs,J.digest(self.inputs.read_bytes()),SOURCE,self.endpoint['job_id'],'build'))
            self.assertEqual(result['original_actual_exit'],100);self.assertEqual(result['phase_actual_exit'],101)
            self.assertEqual(result['original_state'],record.get('state'));self.assertEqual(result['original_completed'],record.get('completed'))
    def test_compiler_marker_grammar_is_anchored_without_exit_inference(self):
        cases=[('error[E0308]: controlled type mismatch',True),
               ('error: could not compile controlled fixture',True),
               ('a note containing error[E0308]: text',False),
               ('"error[E0308]: quoted text"',False),
               ('error[E0308] missing colon',False),('ordinary error text',False)]
        for text,expected in cases:
            with self.subTest(text=text): self.assertEqual(bool(F.FAIL.search(text)),expected)
        self.assertEqual(self.marker['actual_exit'],100)
        self.assertEqual(self.receipt['command_exit'],101)
    def test_all_marker_identity_fields_require_the_official_schema(self):
        changes={'schema':True,'lane':'../escape','source_commit':'b'*40,'box_id':'foreign','host':'invalid','port':True,'job_id':'b'*32,'output_root':'/tmp/escape','command_sha256':'x','execution_sha256':'x','source_archive_sha256':'x','boot_id':'old','artifacts':[]}
        for key,value in changes.items():
            record=copy.deepcopy(self.marker);record[key]=value
            with self.subTest(key=key),self.assertRaises(ValueError):self.admission(record)
    def test_real_schema_one_marker_and_observation_keep_legacy_identity(self):
        owner=self.root/'schema-one';owner.mkdir();phase=owner/'build.phase.manifest.json';child=owner/'child.py'
        child.write_text(self.child.read_text().replace(str(self.root/'build.log'),str(owner/'build.log')).replace(str(self.phase),str(phase)))
        quote=__import__('shlex').quote;command='exec '+quote(sys.executable)+' '+quote(str(child))
        endpoint={k:v for k,v in self.endpoint.items() if k not in ('execution_sha256','source_archive_sha256')}
        endpoint.update(schema=1,job_id=uuid.uuid4().hex,output_root='amazing/out/diagnostic_fixture/schema-one-'+uuid.uuid4().hex,command_sha256=J.digest(command.encode()))
        program='import importlib.util,sys;s=importlib.util.spec_from_file_location("job",'+repr(str(ROOT/'completed-job.py'))+');j=importlib.util.module_from_spec(s);s.loader.exec_module(j);sys.exit(j.run('+repr(endpoint)+','+repr(command)+'))'
        result=subprocess.run([sys.executable,'-I','-c',program],env=dict(os.environ,HOME=str(self.home)),capture_output=True);self.assertEqual(result.returncode,100,result.stderr.decode())
        path=self.home/('amazing-completed-'+endpoint['job_id']+'.json');original=path.read_bytes();marker=json.loads(original)
        with mock.patch.object(pathlib.Path,'home',return_value=self.home):observed=J.observe(endpoint)
        for record in [marker,observed]:self.assertEqual(J.diagnostic_completion(record,SOURCE,endpoint['job_id'])[:2],(endpoint,100))
        diagnostic=json.loads(D.produce(path,J.digest(original),phase,J.digest(phase.read_bytes()),SOURCE,endpoint['job_id'],owner));self.assertEqual(diagnostic['phase_actual_exit'],101);self.assertEqual(path.read_bytes(),original)
    def test_missing_or_partial_native_identity_is_not_an_observation(self):
        for key in [*J.FIELDS,'execution_sha256','source_archive_sha256','boot_id','artifacts']:
            record=copy.deepcopy(self.marker);record.pop(key)
            with self.subTest(key=key),self.assertRaises((ValueError,KeyError)):self.admission(record)
    def test_incomplete_native_markers_and_hybrids_fail_closed(self):
        for updates in [{'completed':False},{'completed':1},{'job':self.endpoint},{'state':'TERMINAL'}]:
            with self.subTest(updates=updates),self.assertRaises(ValueError):self.admission(dict(self.marker,**updates))
    def test_unknown_and_running_observations_do_not_become_completed(self):
        for state in ['UNKNOWN','RUNNING',None]:
            with self.subTest(state=state),self.assertRaises(ValueError):self.admission(dict(self.observation,state=state))
    def test_positive_and_signal_native_returncodes_must_match_real_normalization(self):
        command='kill -TERM $$'
        endpoint=dict(self.endpoint,job_id=uuid.uuid4().hex,
            output_root='amazing/out/diagnostic_fixture/signal-'+uuid.uuid4().hex,
            command_sha256=J.digest(command.encode()),execution_sha256=J.digest(command.encode()))
        program='import importlib.util,sys;s=importlib.util.spec_from_file_location("job",'+repr(str(ROOT/'completed-job.py'))+');j=importlib.util.module_from_spec(s);s.loader.exec_module(j);sys.exit(j.run('+repr(endpoint)+','+repr(command)+'))'
        result=subprocess.run([sys.executable,'-I','-c',program],env=dict(os.environ,HOME=str(self.home)),capture_output=True)
        self.assertEqual(result.returncode,143,result.stderr.decode())
        marker=json.loads((self.home/('amazing-completed-'+endpoint['job_id']+'.json')).read_text())
        self.assertEqual(marker['command_returncode'],-15)
        with mock.patch.object(pathlib.Path,'home',return_value=self.home): observed=J.observe(endpoint)
        for record in [marker,observed]:
            self.assertEqual(J.diagnostic_completion(record,SOURCE,endpoint['job_id'])[1],143)
            conflicting=copy.deepcopy(record);conflicting['actual_exit']=conflicting['runner_exit']=15
            with self.assertRaises(ValueError):J.diagnostic_completion(conflicting,SOURCE,endpoint['job_id'])
    def test_success_bool_unknown_runner_and_conflicting_exit_facts_rejected(self):
        for record in [self.marker,self.observation]:
            for updates in [{'actual_exit':0,'runner_exit':0,'command_returncode':0},{'actual_exit':True},{'runner_exit':True},{'command_returncode':True},{'runner_exit':3},{'command_returncode':101},{'actual_exit':None}]:
                with self.subTest(updates=updates),self.assertRaises(ValueError):self.admission(dict(record,**updates))
    def test_original_marker_hash_and_phase_hash_still_authoritative(self):
        with self.assertRaises(ValueError):self.wrapped(self.marker,'0'*64)
        self.inputs.write_text(json.dumps(self.marker))
        with self.assertRaises(ValueError):D.produce(self.inputs,J.digest(self.inputs.read_bytes()),self.phase,'0'*64,SOURCE,self.endpoint['job_id'],self.root)
    def test_large_original_marker_catalog_is_read_bounded_without_dumping_or_relabeling(self):
        # A genuine catalog with many tiny compact files exceeds64KiB while
        # remaining within the native producer's two-MiB compact byte budget.
        inputs=self.root/'compact-inputs';inputs.mkdir()
        for index in range(700): (inputs/('entry-'+str(index)+'.compact.json')).write_text('{}')
        large=self.root/'large';large.mkdir();phase=large/'build.phase.manifest.json';child=large/'child.py'
        child.write_text(self.child.read_text().replace(str(self.root/'build.log'),str(large/'build.log')).replace(str(self.phase),str(phase)))
        endpoint=dict(self.endpoint,job_id=uuid.uuid4().hex,
            output_root='amazing/out/diagnostic_fixture/large-'+uuid.uuid4().hex)
        quote=__import__('shlex').quote
        command='mkdir -p '+quote(str((self.home/endpoint['output_root']).parent))+' && cp -r '+quote(str(inputs))+' '+quote(str(self.home/endpoint['output_root']))+' && exec '+quote(sys.executable)+' '+quote(str(child))
        endpoint.update(command_sha256=J.digest(command.encode()),execution_sha256=J.digest(command.encode()))
        program='import importlib.util,sys;s=importlib.util.spec_from_file_location("job",'+repr(str(ROOT/'completed-job.py'))+');j=importlib.util.module_from_spec(s);s.loader.exec_module(j);sys.exit(j.run('+repr(endpoint)+','+repr(command)+'))'
        result=subprocess.run([sys.executable,'-I','-c',program],env=dict(os.environ,HOME=str(self.home)),capture_output=True)
        self.assertEqual(result.returncode,100,result.stderr.decode())
        path=self.home/('amazing-completed-'+endpoint['job_id']+'.json');self.assertGreater(path.stat().st_size,65536);original=path.read_bytes()
        self.assertEqual(json.loads(phase.read_text())['command_exit'],101)
        result=json.loads(D.produce(path,J.digest(original),phase,J.digest(phase.read_bytes()),SOURCE,endpoint['job_id'],large))
        self.assertEqual(result['original_actual_exit'],100);self.assertEqual(result['phase_actual_exit'],101)
        self.assertEqual(result['original_completion_format'],'native-completion-marker');self.assertEqual(path.read_bytes(),original)
    def test_terminal_input_remains_bounded(self):
        self.inputs.write_bytes(b'x'*(F.MAX_COMPLETION+1))
        with self.assertRaises(ValueError):D.produce(self.inputs,J.digest(self.inputs.read_bytes()),self.phase,J.digest(self.phase.read_bytes()),SOURCE,self.endpoint['job_id'],self.root)

if __name__=='__main__':unittest.main()
