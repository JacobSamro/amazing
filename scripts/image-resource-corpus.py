#!/usr/bin/env python3
"""Faithful full image resource cohort; no static-resource filtering."""
import argparse, collections, concurrent.futures, hashlib, importlib.util, json, os, pathlib, subprocess, sys
parser=argparse.ArgumentParser()
parser.add_argument('--source',required=True)
parser.add_argument('--corpus-role',choices=('before','after'),required=True)
parser.add_argument('--wpt-root',required=True)
parser.add_argument('--exe',required=True)
parser.add_argument('--raw-out',required=True)
parser.add_argument('--compact-out',required=True)
parser.add_argument('--baseline',default='')
parser.add_argument('--baseline-source')
parser.add_argument('--baseline-corpus-role',choices=('before','after'),default='before')
args=parser.parse_args()
source,baseline=args.source,args.baseline
out=pathlib.Path(args.raw_out); out.mkdir(parents=True,exist_ok=False)
wpt=pathlib.Path(args.wpt_root).resolve()
if os.environ.get('AMAZING_SOURCE_COMMIT') != source:
 raise ValueError('public native source does not match requested source')
spec=importlib.util.spec_from_file_location('public_compare',pathlib.Path(__file__).parent/'wpt_render_compare.py');api=importlib.util.module_from_spec(spec);spec.loader.exec_module(api)
manifest_path=pathlib.Path('crates/amazing-paint/tests/fixtures/rc21-wpt-render.json');manifest=json.loads(manifest_path.read_text());assert len(manifest['tests'])==226
revision=subprocess.check_output(['git','-C',str(wpt),'rev-parse','HEAD'],text=True).strip();assert revision==manifest['revision']
paths={r['test'] for r in manifest['tests']};assert len(paths)==226
rc_ids={r['test']:r['id'] for r in manifest['tests']}
# Discover every resource-backed reftest; no classify_candidate resource filter.
for p in sorted((wpt/'css/css-images').rglob('*')):
 if p.is_file() and p.suffix.lower() in ('.html','.htm','.xhtml','.xht','.svg'):
  source_text=api.read_wpt_text(p);links=api.reftest_links(p,source_text,wpt)
  if links['supported'] or links['reason']!='not-reftest':paths.add(str(p.relative_to(wpt)))
if len(paths)!=491: raise ValueError(f'full frozen resource cohort changed: {len(paths)} !=491')
exe=pathlib.Path(args.exe).resolve();raw=out/'raw';raw.mkdir()
metadata={'revision':revision,'rc21_entries':rc_ids,'corpus_role':args.corpus_role,'manifest_sha256':api.sha256_file(manifest_path),'settings':{'width':800,'height':600,'timeout':20,'jobs':4},'adapter_sha256':api.sha256_file(pathlib.Path(__file__)),'comparator_sha256':api.sha256_file(pathlib.Path(__file__).parent/'wpt_render_compare.py'),'selection_sha256':hashlib.sha256(json.dumps(sorted(paths),separators=(',',':')).encode()).hexdigest(),'profile':{k:os.environ.get(k) for k in ('CARGO_PROFILE_DEV_DEBUG','CARGO_PROFILE_TEST_DEBUG','CARGO_INCREMENTAL')}}
b=None
if baseline:
 b=json.loads(pathlib.Path(baseline).read_text())
 contract_spec=importlib.util.spec_from_file_location('image_contract',pathlib.Path(__file__).parent/'image-corpus-contract.py');contract=importlib.util.module_from_spec(contract_spec);contract_spec.loader.exec_module(contract)
 contract.validate_resource_metadata(b,metadata,args.baseline_corpus_role,args.baseline_source,paths)

def run(path):
 p=wpt/path
 if not p.is_file():return {'id':path,'classification':'unsupported','detail':'missing frozen corpus entry'}
 links=api.reftest_links(p,api.read_wpt_text(p),wpt)
 if not links['supported']:return {'id':path,'classification':'unsupported','detail':links['reason']}
 try:
  result=api.run_one_test({'path':path,'abs_path':p,'links':links['links']},exe,wpt,raw,800,600,20)
  result['id']=path;return result
 except (OSError,ValueError) as e:
  return {'id':path,'classification':'error','detail':str(e)}
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:results=list(pool.map(run,sorted(paths)))
counts=dict(collections.Counter(r['classification'] for r in results));states={r['id']:r['classification'] for r in results}
report={'source':source,'revision':revision,'viewport':[800,600],'manifest_sha256':api.sha256_file(manifest_path),'rc21_entries':rc_ids,'counts':counts,'tests':results};report_path=out/'report.json';report_path.write_text(json.dumps(report,separators=(',',':'))+'\n')
compact={k:v for k,v in report.items() if k!='tests'};compact.update(schema=1,corpus_role=args.corpus_role,settings={'width':800,'height':600,'timeout':20,'jobs':4},binary_sha256=api.sha256_file(exe),adapter_sha256=api.sha256_file(pathlib.Path(__file__)),comparator_sha256=api.sha256_file(pathlib.Path(__file__).parent/'wpt_render_compare.py'),selection_sha256=hashlib.sha256(json.dumps(sorted(paths),separators=(',',':')).encode()).hexdigest());compact['tests']=states;compact['raw_report']={'path':str(report_path),'sha256':api.sha256_file(report_path),'bytes':report_path.stat().st_size}
compact['source_commit']=source
compact['profile']={k:os.environ.get(k) for k in ('CARGO_PROFILE_DEV_DEBUG','CARGO_PROFILE_TEST_DEBUG','CARGO_INCREMENTAL')}
failed=False
if baseline:
 compact['comparison']=contract.compare_resource(b,compact,args.baseline_corpus_role,args.baseline_source)
 failed=bool(compact['comparison']['lost'])

data=(json.dumps(compact,separators=(',',':'))+'\n').encode();
if len(data)>512*1024: raise ValueError('compact resource evidence exceeds512KiB')
with pathlib.Path(args.compact_out).open('xb') as stream:
 stream.write(data); stream.flush(); os.fsync(stream.fileno())
print(json.dumps({'source':source,'counts':counts,'rc21':len(rc_ids),'total':len(paths),'lost':compact.get('comparison',{}).get('lost',[])}),flush=True)
sys.exit(1 if failed else 0)
