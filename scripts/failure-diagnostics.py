#!/usr/bin/env python3
"""Read authenticated native failures; emit bounded diagnostics, never pass proof."""
import argparse, base64, collections, hashlib, json, pathlib, re, tempfile, zlib
LIMIT = 64 * 1024 * 1024
ANSI = re.compile(r'\x1b\[[0-?]*[ -/]*[@-~]')
def digest(data): return hashlib.sha256(data).hexdigest()
def verified(root, record, limit=LIMIT):
    path = pathlib.Path(record['path'])
    if not path.is_absolute() or path.resolve(strict=True) != path or not path.is_relative_to(root):
        raise ValueError('input must be canonical within declared native owner')
    if type(record['bytes']) is not int or not 0 <= record['bytes'] <= limit:
        raise ValueError('invalid bounded input size')
    data = path.read_bytes()
    if len(data) != record['bytes'] or digest(data) != record['sha256']:
        raise ValueError('native input size/hash contradiction')
    return data

def unpack(root, descriptor):
    manifest = json.loads(verified(root, descriptor, 524288)); parts=[]
    for row in manifest['parts']:
        name = row['name']
        if pathlib.Path(name).name != name: raise ValueError('unsafe compact part')
        record = dict(row, path=str(pathlib.Path(descriptor['path']).parent/name))
        parts.append(json.loads(verified(root, record, 524288))['data'])
    encoded = base64.b64decode(''.join(parts), validate=True)
    inflater = zlib.decompressobj(); data = inflater.decompress(encoded, LIMIT+1)
    if len(data)>LIMIT or not inflater.eof or inflater.unconsumed_tail or inflater.unused_data:
        raise ValueError('unbounded/trailing compact stream')
    if len(data)!=manifest['uncompressed_bytes'] or digest(data)!=manifest['uncompressed_sha256']:
        raise ValueError('native map hash contradiction')
    return json.loads(data)

def nextest_excerpts(data, expected):
    lines=ANSI.sub('', data.decode('utf-8', errors='replace')).splitlines(); results={}
    for identity in expected:
        name=identity.split(' ',1)[-1]
        starts=[i for i,line in enumerate(lines) if ('STDERR:' in line or 'STDOUT:' in line) and name in line]
        excerpts=[]
        for start in starts:
            end=start+1
            while end<len(lines) and 'STDERR:' not in lines[end] and 'STDOUT:' not in lines[end] and not re.match(r'^\s*(PASS|FAIL|SKIP)\s+\[',lines[end]): end+=1
            text='\n'.join(lines[start:end]); excerpts.append({'text':text[:12000],'truncated':len(text)>12000})
        contexts=[]
        for i,line in enumerate(lines):
            if name in line and ('panicked at' in line or 'thread ' in line):
                text='\n'.join(lines[max(0,i-3):min(len(lines),i+20)])
                contexts.append({'text':text[:12000],'truncated':len(text)>12000})
        results[identity]={'identified_output_sections':len(starts),'excerpts':excerpts[:4], 'section_limit_reached':len(excerpts)>4,
                           'literal_named_panic_contexts':contexts[:4], 'panic_context_limit_reached':len(contexts)>4}
    return results

def produce(request):
    root=pathlib.Path(request['native_owner']).resolve(strict=True)
    if not root.is_relative_to('/tmp') or str(root)!=request['native_owner']: raise ValueError('native /tmp owner required')
    source=request['engine_source']; data=verified(root,request['workspace_raw'])
    model=unpack(root,request['wpt_manifest'])
    if model['source']!=source or model['role']!='after' or model['input_sha256']!=request['input_sha256']: raise ValueError('source/cohort mismatch')
    records=model['records']; by_file={r['file']:r for r in records}
    if len(records)!=request['case_count'] or len(by_file)!=len(records): raise ValueError('case count/collision')
    lost=request['lost_subtests']; grouped=collections.defaultdict(list)
    for file,name,occurrence in lost: grouped[file].append((name,occurrence))
    reports={}
    for file,identities in grouped.items():
        record=by_file[file]; raw=json.loads(verified(root,record['raw']['stdout']))
        if raw['test']!=file: raise ValueError('native report identity mismatch')
        counts=collections.Counter(); current={}
        for sub in raw['subtests']:
            name=sub['name']; occurrence=counts[name];counts[name]+=1;current[(name,occurrence)]=sub
        messages=collections.Counter(); examples=[]; missing=0
        for identity in identities:
            sub=current.get(identity)
            if sub is None: missing+=1; messages['missing native subtest identity']+=1;continue
            message=sub.get('message'); key=message if type(message) is str else json.dumps(message,ensure_ascii=True)
            messages[key]+=1
            if len(examples)<20: examples.append({'name':identity[0],'occurrence':identity[1],'native_status':sub['status'],'message':key[:1500],'message_truncated':len(key)>1500})
        reports[file]={'lost_identities':len(identities),'missing_in_actual_report':missing,'actual_subtests':len(raw['subtests']),
                       'harness_status':raw.get('harness_status'),'detail':str(raw.get('detail'))[:1500],
                       'message_clusters':[{'message':key[:1500],'count':count,'truncated':len(key)>1500} for key,count in messages.most_common(20)],'examples':examples}
    return {'schema':1,'diagnostic_only':True,'qualification_claimed':False,'engine_source':source,'original_job':request['original_job'],
            'workspace_raw':request['workspace_raw'],'wpt_manifest':request['wpt_manifest'],'case_count':len(records),
            'lost_subtest_count':len(lost),'nextest_failures':nextest_excerpts(data,request['nextest_failed']), 'wpt_clusters':reports}

def self_test():
    passed=[]
    with tempfile.TemporaryDirectory(prefix='amazing-diagnostic-controls-') as directory:
        root=pathlib.Path(directory);p=root/'raw';p.write_bytes(b'authentic bytes');r={'path':str(p),'bytes':len(p.read_bytes()),'sha256':digest(p.read_bytes())}
        assert verified(root,r)==b'authentic bytes';passed.append('exact_native_size_and_hash')
        for key,value in [('bytes',13),('sha256','0'*64),('bytes',True)]:
            bad=dict(r);bad[key]=value
            try:verified(root,bad)
            except ValueError:pass
            else:raise AssertionError('contradictory witness accepted')
        passed.append('contradictory_size_hash_and_bool_rejected')
        alias=root/'alias';alias.symlink_to(p)
        try:verified(root,dict(r,path=str(alias)))
        except ValueError:pass
        else:raise AssertionError('symlink accepted')
        passed.append('source_alias_rejected')
    fixture=b' FAIL [ 0.1s] suite test::one\n---- STDERR: suite test::one ----\nassertion left != right\n PASS [ 0.1s] suite test::two\n'
    result=nextest_excerpts(fixture,['suite test::one']);assert 'assertion left != right' in result['suite test::one']['excerpts'][0]['text'];passed.append('named_failure_sections_preserved')
    assert nextest_excerpts(b'\x1b[31m'+fixture,['suite test::one'])==result;passed.append('ansi_does_not_change_diagnostic_identity')
    assert nextest_excerpts(fixture,['suite absent'])['suite absent']['identified_output_sections']==0;passed.append('missing_sections_never_invented')
    long=b'---- STDERR: suite test::one ----\n'+b'x'*16000;assert nextest_excerpts(long,['suite test::one'])['suite test::one']['excerpts'][0]['truncated'];passed.append('bounded_excerpt_declares_truncation')
    panic=b"thread 'test::one' panicked at source.rs:3:\nassertion failed: value\n";assert 'assertion failed' in nextest_excerpts(panic,['suite test::one'])['suite test::one']['literal_named_panic_contexts'][0]['text'];passed.append('literal_named_panic_context_without_invented_stream_header')
    return {'controls':len(passed),'pass':len(passed),'fail':0,'identities':passed,'control_only':True}

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--request',type=pathlib.Path);parser.add_argument('--output',type=pathlib.Path,required=True);parser.add_argument('--self-test',action='store_true');args=parser.parse_args()
    result=self_test() if args.self_test else produce(json.loads(args.request.read_text()))
    encoded=json.dumps(result,ensure_ascii=True,separators=(',',':')).encode()+b'\n'
    if len(encoded)>524288:raise ValueError('compact diagnostics exceed limit')
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_bytes(encoded);print(json.dumps({'bytes':len(encoded),'sha256':digest(encoded),'diagnostic_only':True}));return 0
if __name__=='__main__':raise SystemExit(main())
