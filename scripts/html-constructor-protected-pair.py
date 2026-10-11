"""Exact paired HTML passing name multiplicities and files, no normalization."""
import base64,collections,gzip,hashlib,json,pathlib,sys,zlib
before_path,after_path,output=map(pathlib.Path,sys.argv[1:4]);before=json.loads(before_path.read_text());after=json.loads(after_path.read_text())
trees_before,trees_after=(json.loads(pathlib.Path(x).read_text())for x in sys.argv[4:6]);assert trees_before['revision']==trees_after['revision'] and trees_before['manifest']==trees_after['manifest'];tree_losses=sorted(set(trees_before['passing_ids'])-set(trees_after['passing_ids']));assert not tree_losses,'html5lib passing cases lost';assert trees_before['unsupported_ids']==trees_after['unsupported_ids']
capsule=json.loads(pathlib.Path(sys.argv[6]).read_text())
assert capsule['source']==before['source_commit'] and capsule['original_compact_sha256']==hashlib.sha256(before_path.read_bytes()).hexdigest()
def before_records(value):
 result={}
 for suite,rows in value['records'].items():
  result[suite]=[]
  for row in rows:
   counter=collections.Counter()
   for name,count in row['passing'].items():counter[hashlib.sha256(name.encode('utf-16le','surrogatepass')).hexdigest()]+=count
   result[suite].append(dict(row,passing=dict(counter)))
 return result
def unpack(r):
 p=pathlib.Path(r['raw_counter']['path']);assert hashlib.sha256(p.read_bytes()).hexdigest()==r['raw_counter']['sha256'];return json.loads(gzip.decompress(p.read_bytes()))
a=before_records(capsule);b=unpack(after);expected={'core','shadow','encoding','syntax','the-window-object','the-windowproxy-exotic-object','windows','the-iframe-element','the-base-element','base-url','cssom','the-history-interface','modules','syntax72'}
assert set(a)==set(b)==expected,'incomplete whole corpus phase records'
losses=[];gains=[];files_lost=[];counts={}
for suite in sorted(a):
 old={r['test']:r for r in a[suite]};new={r['test']:r for r in b[suite]};assert len(old)==len(a[suite])and len(new)==len(b[suite]);assert set(old)==set(new),'selection changed'
 passed_before=passed_after=0
 for name,record in old.items():
  prior=collections.Counter(record['passing']);current=collections.Counter(new[name]['passing']);passed_before+=prior.total();passed_after+=current.total()
  losses.extend({'suite':suite,'test':name,'name_utf16le_sha256':h,'multiplicity':n}for h,n in (prior-current).items());gains.extend({'suite':suite,'test':name,'name_utf16le_sha256':h,'multiplicity':n}for h,n in (current-prior).items())
  if record['status']=='pass'and new[name]['status']!='pass':files_lost.append({'suite':suite,'test':name,'after_status':new[name]['status']})
 counts[suite]={'files':len(old),'before':passed_before,'after':passed_after}
report={'source_before':before['source_commit'],'source_after':after['source_commit'],'binary_before':before['binary_sha256'],'binary_after':after['binary_sha256'],'counts':counts,'passing_counter_losses':losses,'passing_counter_gains':gains,'passing_files_lost':files_lost,'zero_loss':not losses and not files_lost,'trees':{'before_passed':trees_before['passed'],'after_passed':trees_after['passed'],'unsupported':trees_before['unsupported'],'lost':tree_losses,'raw_before':trees_before['raw'],'raw_after':trees_after['raw']},'raw_before':before['raw_counter'],'raw_after':after['raw_counter']}

for kind in ('xmlts.compact.json','executed-trees.compact.json'):
 x=json.loads((before_path.parent/kind).read_text());y=json.loads((after_path.parent/kind).read_text())
 if kind.startswith('xmlts'):
  assert x['total']==y['total']==2377 and x['eligible']==y['eligible']==1992
  assert x['archive_sha256']==y['archive_sha256'] and x['manifest_sha256']==y['manifest_sha256']
  aa=json.loads(zlib.decompress(base64.b64decode(x['records_zlib_base64'])));bb=json.loads(zlib.decompress(base64.b64decode(y['records_zlib_base64'])))
  identity=lambda r:(r['fixture'],r['id'],r['metadata_sha256'],r['source_sha256'])
  assert collections.Counter(map(identity,aa))==collections.Counter(map(identity,bb))
  pa=collections.Counter(identity(r) for r in aa if r['classification']=='pass');pb=collections.Counter(identity(r) for r in bb if r['classification']=='pass')
 else:
  assert x['revision']==y['revision'] and x['manifest']==y['manifest']
  pa=collections.Counter(r['id'] for r in x['records'] if r['status']=='pass');pb=collections.Counter(r['id'] for r in y['records'] if r['status']=='pass')
 lost=list((pa-pb).elements());report[kind]={'before':pa.total(),'after':pb.total(),'lost':lost};report['zero_loss']=report['zero_loss'] and not lost

data=(json.dumps(report,separators=(',',':'),sort_keys=True)+'\n').encode();assert len(data)<=524288;output.write_bytes(data)
print(json.dumps({'zero_loss':report['zero_loss'],'lost_multiplicities':sum(x['multiplicity']for x in losses),'gained_multiplicities':sum(x['multiplicity']for x in gains),'lost_files':len(files_lost)}))
raise SystemExit(0 if report['zero_loss']else 1)
