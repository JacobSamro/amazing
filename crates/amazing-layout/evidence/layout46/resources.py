#!/usr/bin/env python3
"""Pinned border-image HTML reftests with bounded local decoded image resources.
Separate from the existing self-contained static corpus; never mutates WPT.
"""
import argparse,importlib.util,json,re,sys,concurrent.futures
from pathlib import Path
ROOT=Path(__file__).resolve().parents[4]
spec=importlib.util.spec_from_file_location('compare',ROOT/'scripts/wpt_render_compare.py')
c=importlib.util.module_from_spec(spec);spec.loader.exec_module(c)
a=argparse.ArgumentParser();a.add_argument('--wpt-root',type=Path,required=True);a.add_argument('--exe',type=Path,required=True);a.add_argument('--out',type=Path,required=True);a.add_argument('--baseline',type=Path);args=a.parse_args()
url_re=re.compile(r'url\(\s*[\"\']?([^\s\"\')]+)',re.I)
selected=[];excluded={};excluded_tests={}
for path in sorted((args.wpt_root/'css/css-backgrounds').glob('border-image*')):
    if path.suffix.lower() not in ('.html','.htm','.xht','.xhtml'):continue
    source=path.read_text(errors='replace');links=c.reftest_links(path,source,args.wpt_root)
    if not links['supported']:continue
    if path.suffix.lower() in ('.xht','.xhtml'):
        excluded['xml-parser-mode']=excluded.get('xml-parser-mode',0)+1;excluded_tests[path.relative_to(args.wpt_root).as_posix()]='xml-parser-mode';continue
    files=[path]+[l['reference_path'] for l in links['links']]
    reason=None;resources=[]
    for f in files:
        if not f.is_file() or f.suffix.lower() not in ('.html','.htm'):reason='reference-mode';break
        s=f.read_text(errors='replace')
        if c.SCRIPT_RE.search(s) or c.INLINE_EVENT_HANDLER_RE.search(s):reason='scripted';break
        if c.FLAGS_META_RE.search(s):
            flag_values=re.findall(r'<meta\b(?=[^>]*name=[\"\']flags[\"\'])[^>]*content=[\"\']([^\"\']*)',s,re.I)
            if not flag_values or any(set(v.split())-{'image'} for v in flag_values):reason='flagged';break
        if c.STYLESHEET_LINK_RE.search(s) or re.search(r'@(?:font-face|import)|<(?:svg|video|object|embed|iframe|picture|source)\b|\bsrcset\s*=|(?:image-set|cross-fade|paint|element)\s*\(',s,re.I):reason='unsupported-resource-kind';break
        urls=url_re.findall(s)+re.findall(r'<img\b[^>]*\bsrc=[\"\']([^\"\']+)',s,re.I)
        for url in urls:
            if ':' in url or url.startswith('/') or '?' in url or '#' in url:reason='non-relative-resource';break
            resource=(f.parent/url).resolve()
            if not resource.is_relative_to(args.wpt_root.resolve()) or not resource.is_file() or resource.suffix.lower() not in ('.png','.jpg','.jpeg','.gif','.bmp','.ppm'):reason='unsupported-decoded-resource';break
            resources.append(resource.relative_to(args.wpt_root.resolve()).as_posix())
        if reason:break
    if not resources and not reason:reason='no-url-resource'
    if reason:excluded[reason]=excluded.get(reason,0)+1;excluded_tests[path.relative_to(args.wpt_root).as_posix()]=reason;continue
    rel=path.relative_to(args.wpt_root).as_posix();selected.append(dict(path=rel,abs_path=path,links=links['links'],hash=c.sha256_hex(rel),resources=sorted(set(resources))))
if not selected:raise SystemExit('No local-resource border-image reftests selected')
args.out.mkdir(parents=True,exist_ok=True)
inventory={'selected':len(selected),'excluded':excluded,'excluded_tests':excluded_tests,'tests':{x['path']:x['resources'] for x in selected}}
(args.out/'inventory.json').write_text(json.dumps(inventory,indent=2))
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    results=list(pool.map(lambda x:c.run_one_test(x,args.exe,args.wpt_root,args.out,800,600,20),selected))
classes={r['id']:r['classification'] for r in results};(args.out/'compact.json').write_text(json.dumps({'tests':classes},indent=2));(args.out/'report.json').write_text(json.dumps(results,indent=2))
summary={'eligible':len(results),'counts':{v:list(classes.values()).count(v) for v in sorted(set(classes.values()))},'excluded':excluded}
if args.baseline:
    old=c.extract_classifications(json.loads(args.baseline.read_text()));summary['lost']=[k for k,v in old.items() if v=='pass' and classes.get(k)!='pass'];summary['gained']=[k for k,v in classes.items() if v=='pass' and old.get(k)!='pass'];summary['new']=sorted(set(classes)-set(old));summary['removed']=sorted(set(old)-set(classes))
(args.out/'summary.json').write_text(json.dumps(summary,indent=2));print(json.dumps(summary))
sys.exit(bool(summary.get('lost') or summary.get('new') or summary.get('removed') or any(v not in ('pass','mismatch') for v in classes.values())))
