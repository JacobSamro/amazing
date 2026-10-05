#!/usr/bin/env python3
"""Whole standalone native tooling controls. No Cargo or engine execution."""
import argparse
import base64
import hashlib
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import zlib

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('collector', HERE / 'nextest-compact.py')
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)


def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--output', required=True)
    args = parser.parse_args(); results = []
    def check(name, function):
        try:
            function(); results.append({'id': name, 'status': 'pass'})
        except Exception as error:
            results.append({'id': name, 'status': 'fail', 'error': str(error)})
    def expect(value, expected):
        assert value == expected, (value, expected)
    def parsed(text): return m.parse(text.splitlines())
    def good(text, expected):
        records, _, errors, _ = parsed(text); expect(errors, []); expect(records, expected)
    summary = 'Summary [0.01s] 1 test run: 1 passed, 0 skipped'
    for label, row in [('legacy', 'PASS [0.001s] crate suite::name'),
                       ('progress', 'PASS [0.001s] ( 1/1) crate suite::name'),
                       ('wide_progress', 'PASS [0.001s] (  0001/0001) crate suite::name'),
                       ('ansi_csi', '\x1b[32mPASS\x1b[0m [0.001s] (1/1) \x1b[35mcrate\x1b[0m suite::name'),
                       ('ansi_osc_bel', '\x1b]0;title\x07PASS [0.001s] crate suite::name'),
                       ('ansi_osc_st', '\x1b]0;title\x1b\\PASS [0.001s] crate suite::name')]:
        check(label, lambda row=row: good(row+'\n'+summary, {'crate suite::name':'pass'}))
    check('name_spaces_punctuation',lambda: good('PASS [0.01s] (1/1) crate suite::test ( 2/3) [data] $value / path\n'+summary, {'crate suite::test ( 2/3) [data] $value / path':'pass'}))
    check('skip_not_in_run_total',lambda:good('PASS [0.1s] crate a\nSKIP [ignored] crate b\nSummary [0.1s] 1 test run: 1 passed, 1 skipped',{'crate a':'pass','crate b':'skip'}))
    check('fail_rows',lambda:good('FAIL [0.1s] (1/1) crate a\nSummary [0.1s] 1 test run: 0 passed, 1 failed, 0 skipped',{'crate a':'fail'}))
    check('final_repeat_after_summary',lambda:good('FAIL [0.1s] crate a\nSummary [0.1s] 1 test run: 0 passed, 1 failed, 0 skipped\nFAIL [0.1s] crate a',{'crate a':'fail'}))
    for name,text in [
        ('duplicate_row','PASS [0.1s] crate a\nPASS [0.1s] crate a\n'+summary),
        ('progress_collision','PASS [0.1s] (1/2) crate a\nPASS [0.1s] (2/2) crate a\n'+summary),
        ('contradictory_duplicate','PASS [0.1s] crate a\nFAIL [0.1s] crate a\n'+summary),
        ('missing_summary','PASS [0.1s] crate a'),
        ('duplicate_summary','PASS [0.1s] crate a\n'+summary+'\n'+summary),
        ('missing_status_rows','Summary [0.1s] 2 tests run: 2 passed, 0 skipped'),
        ('malformed_row','PASS no duration crate a\n'+summary),
        ('missing_binary','PASS [0.1s] onlyname\n'+summary),
        ('count_mismatch','PASS [0.1s] crate a\nSummary [0.1s] 2 tests run: 2 passed, 0 skipped'),
        ('fail_count_mismatch','FAIL [0.1s] crate a\n'+summary),
        ('duplicate_count','PASS [0.1s] crate a\nSummary [0.1s] 1 test run: 1 passed, 1 passed, 0 skipped'),
        ('incomplete_run','PASS [0.1s] crate a\nSummary [0.1s] 1/2 tests run: 1 passed, 0 skipped')]:
        check(name, lambda text=text: expect(bool(parsed(text)[2]), True))
    check('real_lost_pass', lambda:expect(m.comparison({'crate a':'pass'}, {'crate a':'fail'})['lost'],['crate a']))
    check('skip_loses_pass',lambda:expect(m.comparison({'crate a':'pass'}, {'crate a':'skip'})['lost'],['crate a']))
    check('exact_name_no_global_progress_removal',lambda:expect(m.normalize({'crate a ( 1/2)':'pass'})[0],{'crate a ( 1/2)':'pass'}))
    def collision():
        try: m.normalize({'(1/2) crate a':'pass','crate a':'pass'})
        except ValueError: return
        raise AssertionError('normalization collision accepted')
    check('baseline_collision',collision)
    fixture = HERE/'fixtures/nextest-compact/authenticated-maps.json'
    authenticated=json.loads(fixture.read_text())
    def original_maps():
        before,_=m.unpack(authenticated['baseline']);after,changed=m.unpack(authenticated['after'])
        c=m.comparison(before,after);expect(len(after),598);expect(changed,598);expect(c['baseline_pass'],540);expect(c['lost'],[]);expect(len(c['gained']),58)
    check('authentic_540_to_598_compact',original_maps)
    def mutation():
        before,_=m.unpack(authenticated['baseline']);after,_=m.unpack(authenticated['after']);target=sorted(before)[0];after[target]='fail';expect(m.comparison(before,after)['lost'],[target])
    check('authentic_negative_one_lost',mutation)
    def invalid_compressed():
        try:m.unpack({'classifications_zlib_base64':base64.b64encode(zlib.compress(b'{}')+b'trailing').decode()})
        except ValueError:return
        raise AssertionError('trailing compressed data accepted')
    check('compressed_trailing_data',invalid_compressed)
    def duplicate_json():
        payload=b'{"crate a":"pass","crate a":"fail"}'
        try:m.unpack({'classifications_zlib_base64':base64.b64encode(zlib.compress(payload)).decode()})
        except ValueError:return
        raise AssertionError('duplicate JSON identity accepted')
    check('duplicate_json_identity',duplicate_json)
    def fixture_rows():
        identities,_=m.unpack(authenticated['after'])
        rows=['PASS [0.001s] ('+str(i)+'/598) '+key for i,key in enumerate(identities,1)]
        rows.append('Summary [0.1s] 598 tests run: 598 passed, 0 skipped')
        good('\n'.join(rows),identities)
    check('whole_authentic_identity_formatting_fixture',fixture_rows)
    with tempfile.TemporaryDirectory(prefix='nextest-collector-controls-') as td:
        root=pathlib.Path(td);raw=root/'output.log';phase=root/'phase.json';baseline=root/'baseline.json'
        raw.write_text('PASS [0.1s] (1/1) crate a\n'+summary+'\n');baseline.write_text(json.dumps({'classifications':{'crate a':'pass'}}))
        def receipt(exit=0,**extra):
            d={'source_commit':'control-source','command_exit':exit,'engine_exit':exit,'wrapper_exit':exit,'output_copy_exit':0,'raw_native':m.describe(raw)};d.update(extra);phase.write_text(json.dumps(d))
        def collect():return m.collect(raw,phase,'control-source',baseline)
        receipt()
        check('phase_raw_provenance',lambda:expect(collect()['phase']['raw_native'],m.describe(raw)))
        check('actual_exit_zero',lambda:expect(collect()['collector_exit'],0))
        def mismatch():
            receipt(raw_native={'bytes':1,'sha256':'wrong'});expect(collect()['collector_exit'],3);receipt()
        check('raw_hash_mismatch',mismatch)
        def wrongsource():
            receipt(source_commit='another');expect(collect()['collector_exit'],3);receipt()
        check('source_mismatch',wrongsource)
        def copyfailed():
            receipt(wrapper_exit=3,output_copy_exit=1);expect(collect()['collector_exit'],3);receipt()
        check('phase_copy_failure',copyfailed)
        def realexit():
            receipt(exit=100);d=collect();expect(d['actual_engine_exit'],100);expect(d['collector_exit'],1);receipt()
        check('actual_nonzero_preserved',realexit)
        def exit_contradiction():
            raw.write_text('FAIL [0.1s] crate a\nSummary [0.1s] 1 test run: 0 passed, 1 failed, 0 skipped\n');receipt()
            expect(collect()['collector_exit'],3)
            raw.write_text('PASS [0.1s] (1/1) crate a\n'+summary+'\n');receipt()
        check('zero_exit_failed_count_contradiction',exit_contradiction)
        def missingexit():
            receipt(engine_exit=None);expect(collect()['collector_exit'],3);receipt()
        check('missing_native_exit',missingexit)
        for label, overrides in [
            ('command_nonzero_engine_zero', {'command_exit':101}),
            ('wrapper_nonzero_engine_zero', {'wrapper_exit':101}),
            ('command_zero_engine_nonzero', {'engine_exit':101,'wrapper_exit':101}),
            ('command_bool', {'command_exit':False}),
            ('engine_bool', {'engine_exit':False}),
            ('wrapper_bool', {'wrapper_exit':False}),
            ('copy_bool', {'output_copy_exit':False}),
            ('command_string', {'command_exit':'0'}),
            ('engine_string', {'engine_exit':'0'}),
            ('wrapper_string', {'wrapper_exit':'0'}),
            ('copy_string', {'output_copy_exit':'0'}),
            ('command_missing', {'command_exit':None}),
            ('wrapper_missing', {'wrapper_exit':None}),
            ('copy_missing', {'output_copy_exit':None}),
            ('engine_negative', {'engine_exit':-1}),
            ('wrapper_negative', {'wrapper_exit':-1}),
            ('copy_nonzero', {'output_copy_exit':1}),
            ('signal_projection_contradiction', {'command_exit':-9,'engine_exit':0,'wrapper_exit':0})]:
            def witnesses(overrides=overrides):
                receipt(**overrides);expect(collect()['collector_exit'],3);receipt()
            check('exit_witness_'+label,witnesses)
        def signal_exit():
            receipt(command_exit=-9,engine_exit=137,wrapper_exit=137)
            d=collect();expect(d['actual_engine_exit'],137);expect(d['collector_exit'],1);receipt()
        check('truthful_gate_command_signal_projection',signal_exit)
        def failcli():
            raw.write_text('FAIL [0.1s] crate a\nSummary [0.1s] 1 test run: 0 passed, 1 failed, 0 skipped\n');receipt(exit=100)
            dest=root/'compact.json';process=subprocess.run([sys.executable,str(HERE/'nextest-compact.py'),'--source-commit','control-source','--phase-receipt',str(phase),'--baseline',str(baseline),str(raw),str(dest)],capture_output=True,text=True)
            expect(process.returncode,1);d=json.loads(dest.read_text());expect(d['comparison']['lost'],['crate a']);expect(d['actual_engine_exit'],100)
        check('cli_real_failure_and_lost',failcli)
    result={'schema':1,'qualification':'Standalone native Python tooling controls only; no engine/Cargo/profile equivalence claim. Authentic compact fixtures retain original hashes; text controls are explicit formatting fixtures.','helper':m.describe(HERE/'nextest-compact.py'),'controls':m.describe(__file__),'fixture':m.describe(fixture),'counts':{'pass':sum(r['status']=='pass' for r in results),'fail':sum(r['status']=='fail' for r in results)},'results':results,'actual_exit':int(any(r['status']=='fail' for r in results))}
    pathlib.Path(args.output).write_text(json.dumps(result,sort_keys=True,indent=2)+'\n');print(json.dumps(result['counts']));return result['actual_exit']


if __name__=='__main__':raise SystemExit(main())
