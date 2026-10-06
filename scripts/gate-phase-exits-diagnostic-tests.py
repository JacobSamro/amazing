#!/usr/bin/env python3
"""Native original-phase controls. These fixtures never qualify engine passes."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('diagnostic',Path(__file__).with_name('gate-failure-diagnostic.py'))
D=importlib.util.module_from_spec(spec);spec.loader.exec_module(D)
SOURCE='a'*40;JOB='b'*32
class Controls(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(prefix='amazing-phase-exits-controls-',dir='/tmp')
        self.root=Path(self.tmp.name).resolve();self.rawroot=self.root/'raw';self.rawroot.mkdir()
        self.summary=self.rawroot/'gate.manifest.json';self.hashes=self.root/'raw-hashes.json';self.terminal=self.root/'terminal.json'
        self.log=self.rawroot/'own-wpt.log';self.script=self.root/'command.sh';self.execution=self.root/'command.log'
        self.script.write_text('export OUT="'+str(self.rawroot)+'"\n'+
            'python3 -c "raise RuntimeError(\'native collector sentinel\')" >"$OUT/own-wpt.log" 2>&1\n'+
            'code=$?\nprintf "%s" "$code" >"$OUT/phase-exit"\n'+
            'printf "HTML_PHASE before own-wpt exit=%s\\n" "$code"\nexit 100\n')
        run=subprocess.run(['bash',str(self.script)],capture_output=True)
        self.execution.write_bytes(run.stdout+run.stderr)
        self.phase=int((self.rawroot/'phase-exit').read_text())
        self.s={'source_commit':SOURCE,'job_exit':run.returncode,'phase_exits':{'own-wpt':self.phase}}
        self.t={'state':'TERMINAL','actual_exit':run.returncode,'runner_exit':run.returncode,'command_returncode':run.returncode,
            'job':{'source_commit':SOURCE,'job_id':JOB,'execution_sha256':D.digest(self.script.read_bytes())},
            'execution_raw':self.binding(self.execution)}
        self.h={};self.save()
    def tearDown(self):self.tmp.cleanup()
    def binding(self,p):return {'path':str(p),'bytes':p.stat().st_size,'sha256':D.digest(p.read_bytes())}
    def save(self):
        self.summary.write_text(json.dumps(self.s));self.terminal.write_text(json.dumps(self.t))
        self.h={'gate.manifest.json':self.binding(self.summary),'own-wpt.log':self.binding(self.log)}
        self.hashes.write_text(json.dumps(self.h))
    def write_hashes(self):self.hashes.write_text(json.dumps(self.h))
    def call(self,source=SOURCE,job=JOB,summary_sha=None,terminal_sha=None,hashes_sha=None):
        return json.loads(D.extract_phase_exits(str(self.summary),summary_sha or D.digest(self.summary.read_bytes()),
            str(self.terminal),terminal_sha or D.digest(self.terminal.read_bytes()),source,job,'own-wpt',
            str(self.hashes),hashes_sha or D.digest(self.hashes.read_bytes())))
    def rejected(self,**kw):
        with self.assertRaises((ValueError,FileNotFoundError)):self.call(**kw)
    def test_actual_native_failure_context_without_counts_or_fabricated_receipt(self):
        v=self.call();self.assertEqual(v['phase_actual_exit'],self.phase);self.assertEqual(v['original_actual_exit'],100)
        self.assertFalse(v['phase_receipt_synthesized']);self.assertFalse(v['tests_rerun']);self.assertFalse(v['counts_reconstructed'])
        self.assertTrue(any('native collector sentinel' in r['text'] for r in v['snippets']))
    def test_original_metadata_summary_without_redundant_native_path(self):
        self.h['gate.manifest.json'].pop('path');self.write_hashes()
        self.assertFalse(self.call()['phase_receipt_synthesized'])
    def test_foreign_source(self):self.rejected(source='c'*40)
    def test_foreign_job(self):self.rejected(job='c'*32)
    def test_nonterminal(self):self.t['state']='RUNNING';self.save();self.rejected()
    def test_native_success(self):self.t['actual_exit']=0;self.save();self.rejected()
    def test_wrapper_native_exit_mismatch(self):self.t['runner_exit']=3;self.save();self.rejected()
    def test_boolean_phase_exit(self):self.s['phase_exits']['own-wpt']=True;self.save();self.rejected()
    def test_successful_phase(self):self.s['phase_exits']['own-wpt']=0;self.save();self.rejected()
    def test_missing_phase(self):self.s['phase_exits']={};self.save();self.rejected()
    def test_contradictory_phase_receipts(self):self.s['phases']=[{'phase':'own-wpt'}];self.save();self.rejected()
    def test_stale_summary_hash(self):self.rejected(summary_sha='0'*64)
    def test_stale_terminal_hash(self):self.rejected(terminal_sha='0'*64)
    def test_stale_hash_metadata(self):self.rejected(hashes_sha='0'*64)
    def test_changed_summary_source(self):self.s['source_commit']='c'*40;self.save();self.rejected()
    def test_changed_summary_exit(self):self.s['job_exit']=1;self.save();self.rejected()
    def test_foreign_native_root(self):
        foreign=self.root/'foreign';foreign.mkdir();p=foreign/'own-wpt.log';p.write_bytes(self.log.read_bytes())
        self.h['own-wpt.log']=self.binding(p);self.write_hashes();self.rejected()
    def test_altered_raw_bytes(self):self.log.write_text('RuntimeError: altered\n');self.rejected()
    def test_altered_raw_size(self):self.h['own-wpt.log']['bytes']+=1;self.write_hashes();self.rejected()
    def test_symlink_log(self):
        p=self.root/'alias.log';p.write_bytes(self.log.read_bytes());self.log.unlink();self.log.symlink_to(p);self.rejected()
    def test_changed_original_execution(self):self.execution.write_text('HTML_PHASE before own-wpt exit=0\n');self.rejected()
    def test_changed_original_script(self):self.script.write_text('export OUT="/tmp/foreign"\n');self.rejected()
    def test_authentic_storage_wrapper_embedded_original_command(self):
        import base64
        original=self.script.read_bytes();self.t['job']['command_sha256']=D.digest(original)
        body=base64.b64encode(original).decode()
        self.script.write_text("printf '%s' '"+body+"' | base64 -d > storage-command.sh\n")
        self.t['job']['execution_sha256']=D.digest(self.script.read_bytes());self.save()
        self.assertFalse(self.call()['phase_receipt_synthesized'])
    def test_foreign_embedded_original_command(self):
        self.t['job']['command_sha256']='0'*64;self.save();self.rejected()
    def test_actual_phase_marker_disagrees(self):
        self.execution.write_text('HTML_PHASE before own-wpt exit=2\n');self.t['execution_raw']=self.binding(self.execution);self.save();self.rejected()
    def test_duplicate_phase_markers(self):
        self.execution.write_bytes(self.execution.read_bytes()*2);self.t['execution_raw']=self.binding(self.execution);self.save();self.rejected()
    def test_missing_error_marker_does_not_infer_cause(self):self.log.write_text('ordinary output\n');self.save();self.rejected()
    def test_bare_terminal_python_error_retains_the_actual_deepest_cause(self):
        self.log.write_text('Traceback (most recent call last):\n'+('  unrelated preceding frame\n'*30)+
            '  File "own-wpt.py", line 32, in run\n    assert actual == expected\nAssertionError\n')
        self.save();v=self.call()
        self.assertTrue(any(r['marker'] and r['text']=='AssertionError' for r in v['snippets']))
        self.assertTrue(any('assert actual == expected' in r['text'] for r in v['snippets']))
        self.assertFalse(v['counts_reconstructed'])
    def test_bounded_output_retains_truncation(self):
        self.log.write_text('RuntimeError: '+('x'*200000)+'\n');self.save();v=self.call()
        self.assertTrue(any(r['line_truncated'] for r in v['snippets']));self.assertLess(len(json.dumps(v)),D.MAX_OUTPUT)

if __name__=='__main__':
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(Controls)
    result=unittest.TextTestRunner(verbosity=2).run(suite)
    if len(sys.argv)>1:
        Path(sys.argv[1]).write_text(json.dumps({'controls':result.testsRun,'failures':len(result.failures),
            'errors':len(result.errors),'actual_success':result.wasSuccessful()},sort_keys=True)+'\n')
    raise SystemExit(0 if result.wasSuccessful() else 1)
