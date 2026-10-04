#!/usr/bin/env python3
"""Metadata-only checks of bootstrap/render composition; no engine or network."""
import contextlib,importlib.util,io,json,os,pathlib,subprocess,sys,tempfile,unittest
from unittest import mock
S=pathlib.Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('render_gate',S/'render-gate.py');gate=importlib.util.module_from_spec(spec);spec.loader.exec_module(gate)

class Bootstrap(unittest.TestCase):
 def test_actual_failure_has_receipt_and_bound(self):
  with tempfile.TemporaryDirectory() as d:
   root=pathlib.Path(d);tools=root/'tools';tools.mkdir();out=root/'out';out.mkdir()
   timeout=tools/'timeout';timeout.write_text('#!/bin/bash\nwhile [[ "$1" = --* ]];do shift;done\nshift\nexec "$@"\n');timeout.chmod(0o755)
   env=dict(os.environ,PATH=str(tools)+':'+os.environ['PATH'],OUT=str(out),COMMIT='fixture')
   r=subprocess.run(['bash','-c',f'. "{S}/linux-bootstrap.sh"; bootstrap_stage fixture 7 bash -c "echo failure-detail; exit 37"'],env=env,capture_output=True,text=True)
   self.assertEqual(r.returncode,3);row=json.loads((out/'bootstrap-fixture.manifest.json').read_text())
   self.assertEqual(row['actual_exit'],37);self.assertIn('failure-detail',row['diagnostic_tail']);self.assertTrue(pathlib.Path(row['raw']['path']).is_file())
 def test_actual_remote_assembly_is_valid_shell(self):
  text=(S/'linux-run.sh').read_text();a=text.index('  BOOTSTRAP_B64=');b=text.index('  T0=',a)
  env=dict(os.environ,REPO=str(S.parent),LANE='fixture',COMMIT='a'*40,LEASE='.amazing-busy/fixture',CMD='printf done',NATIVE_MISSION_DIR='')
  r=subprocess.run(['bash','-c',text[a:b]+'\nprintf "%s" "$REMOTE"'],env=env,capture_output=True,text=True)
  self.assertEqual(r.returncode,0,r.stderr)
  valid=subprocess.run(['bash','-n'],input=r.stdout,text=True,capture_output=True);self.assertEqual(valid.returncode,0,valid.stderr)
  self.assertIn('bash -c \'. "$1" && export -p > "$2"\'',r.stdout)
  self.assertIn('. "$env_native.exports"',r.stdout)
 def test_no_home_link_and_bounded_stages(self):
  text=(S/'linux-bootstrap.sh').read_text();run=(S/'linux-run.sh').read_text()
  self.assertNotIn('ln -sfn ~/corpora',run.split('if [ "$HOST" = boat ]; then')[1].split('# Bulk transfers')[0])
  for label in ('environment-copy','environment-load'):self.assertIn('bootstrap_stage '+label+' 30',run)
  self.assertIn('bootstrap_stage native-source-sync 600',text)
  self.assertIn('timeout --signal=TERM --kill-after=5',text)

class Render(unittest.TestCase):
 def fixture(self,workspace_exit=0,baseline=True):
  temp=tempfile.TemporaryDirectory();self.addCleanup(temp.cleanup);root=pathlib.Path(temp.name);out=root/'out';out.mkdir()
  if baseline:
   p=out/'render-gate/tag/before/css-css-floats.compact.json';p.parent.mkdir(parents=True);p.write_text('{}')
  calls=[]
  def run(argv,phase,source,log,receipt,copies=()):
   calls.append((phase,argv));code=workspace_exit if phase=='nextest' else 0
   text='Summary [ 0s] 3 tests run: 3 passed, 17 skipped\n' if phase=='nextest' else 'test result: ok. 4 passed; 0 failed; 0 ignored;\n' if phase=='doctests' else ''
   pathlib.Path(log).write_text(text);r={'phase':phase,'wrapper_exit':code,'command_exit':code};pathlib.Path(receipt).write_text(json.dumps(r))
   for a,b in copies:
    pathlib.Path(a).parent.mkdir(parents=True,exist_ok=True);pathlib.Path(a).write_text(json.dumps({'counts':{'pass':2},'tests':{'a':'pass'}}));pathlib.Path(b).write_bytes(pathlib.Path(a).read_bytes())
   return r
  with mock.patch.dict(os.environ,OUT=str(out),CARGO_TARGET_DIR=str(root/'target')),mock.patch.object(gate.command,'run',side_effect=run),mock.patch.object(sys,'argv',['render-gate','--source','after','--tag','tag','--workspace','--baseline-ref','before','css/css-floats']),contextlib.redirect_stdout(io.StringIO()):rc=gate.main()
  return rc,calls,json.loads((out/'render-gate/tag/after/gate.summary.json').read_text())
 def test_workspace_docs_render_and_exact_baseline(self):
  rc,calls,row=self.fixture();self.assertEqual(rc,0)
  self.assertEqual([x[0] for x in calls],['nextest','doctests','ensure-wpt','build','css-css-floats'])
  self.assertIn('--status-level',calls[0][1]);self.assertIn('--compact',calls[-1][1]);self.assertIn('--baseline',calls[-1][1]);self.assertEqual(row['counts']['doctests']['passed'],4)
 def test_native_failure_still_runs_docs_and_corpus(self):
  rc,calls,row=self.fixture(101);self.assertEqual(rc,1);self.assertEqual(row['phase_exits']['nextest'],101);self.assertIn('css-css-floats',[c[0] for c in calls])
 def test_missing_required_baseline_is_failure(self):
  rc,calls,row=self.fixture(baseline=False);self.assertEqual(rc,1);self.assertEqual(row['renders']['css-css-floats']['error'],'required baseline missing')
 def test_frontend_invokes_exact_refs_and_fetches_only_compact(self):
  with tempfile.TemporaryDirectory() as d:
   root=pathlib.Path(d);tools=root/'scripts';tools.mkdir()
   (tools/'linux-run.sh').write_text('#!/bin/bash\nprintf "%s" "${@: -1}" > "$CAPTURE"\n')
   (tools/'lane-fetch.sh').write_text('#!/bin/bash\nmkdir -p "$2"\nprintf \'{"renders":{}}\' > "$2/gate.summary.json"\n')
   text=(S/'linux-render-wpt.sh').read_text();a=text.index('run_side() {');b=text.index('if [ -n "$BASE_REF" ]',a)
   script='REPO="$FIXTURE"; MAIN="$FIXTURE"; LANE=layout; TAG=fixture; HELPERS=e30=; ARGS=(--workspace); SUBDIRS=(css/css-floats)\n'+text[a:b]+'\nrun_side '+('a'*40)+' '+('b'*40)
   env=dict(os.environ,FIXTURE=str(root),CAPTURE=str(root/'command'))
   r=subprocess.run(['bash','-c',script],env=env,capture_output=True,text=True);self.assertEqual(r.returncode,0,r.stderr)
   cmd=(root/'command').read_text();valid=subprocess.run(['bash','-n'],input=cmd,text=True,capture_output=True);self.assertEqual(valid.returncode,0,valid.stderr)
   self.assertIn("--baseline-ref '"+'b'*40+"'",cmd);self.assertIn('--workspace css/css-floats',cmd)
 def test_frontend_compact_only_exact_ref(self):
  text=(S/'linux-render-wpt.sh').read_text();self.assertIn('--ref "$source"',text);self.assertNotIn('clusters.json',text);self.assertNotIn('rm -rf',text);self.assertIn('gate.summary.json',text)

if __name__=='__main__':unittest.main()
