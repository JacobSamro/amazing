#!/usr/bin/env python3
"""Wrapped command/case controls supplement the original24 phase controls."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import sys

spec = importlib.util.spec_from_file_location('diagnostic', Path(__file__).with_name('gate-failure-diagnostic.py'))
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)


def run(output):
    records = []
    with tempfile.TemporaryDirectory(prefix='amazing-render-diagnostic-controls-', dir='/tmp') as scratch:
        root = Path(scratch).resolve()
        log, summary_path, terminal_path, compact, report = [root / n for n in ['raw.log', 'summary.json', 'terminal.json', 'map.compact.json', 'report.json']]
        source, job, phase, case_id = 'a' * 40, 'b' * 32, 'css-css-grid', 'lost-case.html'
        raw = b'FAIL engine::group\nthread panicked at grid.rs:8\nassertion failed\nleft: none\nright: tracks\n'
        log.write_bytes(raw)
        receipt = {'source_commit': source, 'phase': phase, 'command_exit': 2, 'engine_exit': 2,
                   'wrapper_exit': 2, 'raw_native': {'path': str(log), 'bytes': len(raw), 'sha256': d.digest(raw)},
                   'command': ['python3', 'scripts/wpt_render_compare.py', '--out', str(root), '--subdir', 'css/css-grid']}
        base = {'source_commit': source, 'job_exit': 1, 'phases': [{'phase': phase, 'source_commit': source,
                'actual_returncode': 2, 'wrapper_exit': 2, 'original_receipt': receipt}], 'compact_files': {}}
        terminal = {'state': 'TERMINAL', 'actual_exit': 1, 'runner_exit': 1, 'command_returncode': 1,
                    'job': {'source_commit': source, 'job_id': job}}
        tests = [
            ('wrapped-phase', 'phase', None, True), ('wrapped-source', 'phase', 'source', False),
            ('wrapped-name', 'phase', 'name', False), ('wrapped-native-exit', 'phase', 'exit', False),
            ('wrapped-engine-exit', 'phase', 'engine', False), ('wrapped-exit-bool', 'phase', 'bool', False),
            ('wrapped-wrapper-exit', 'phase', 'wrapper', False), ('wrapped-missing-raw', 'phase', 'missing', False),
            ('authentic-lost-case', 'case', None, True), ('case-not-lost', 'case', 'not-lost', False),
            ('case-pass', 'case', 'pass', False), ('case-wrong-id', 'case', 'id', False),
            ('case-duplicate-id', 'case', 'duplicate', False), ('case-compact-hash', 'case', 'hash', False),
            ('case-changed-classification', 'case', 'classification', False), ('case-wrong-pin', 'case', 'pin', False),
            ('case-wrong-viewport', 'case', 'viewport', False), ('case-report-symlink', 'case', 'symlink', False),
            ('case-outside-native', 'case', 'outside', False), ('case-wrong-render-command', 'case', 'command', False),
            ('case-phase-subdirectory', 'case', 'subdir', False), ('case-ambiguous-output', 'case', 'out', False),
            ('case-long-detail-bounded', 'case', 'long', True), ('case-unicode-detail-bounded', 'case', 'unicode', True)]
        for name, mode, mutation, expected in tests:
            s = copy.deepcopy(base);r = s['phases'][0]['original_receipt']
            c = {'subdir': 'css/css-grid', 'wpt_revision': 'c' * 40, 'viewport': {'width': 800, 'height': 600, 'device_scale': 1},
                 'tests': {case_id: 'crash'}, 'baseline_compare': {'lost': [case_id]}}
            detail = 'renderer exit code 1: immutable fixture I/O failure'
            rp = {'subdir': c['subdir'], 'wpt_revision': c['wpt_revision'], 'viewport': c['viewport'],
                  'tests': [{'id': case_id, 'classification': 'crash', 'detail': detail}]}
            if mutation == 'source': r['source_commit'] = 'f' * 40
            elif mutation == 'name': r['phase'] = 'other'
            elif mutation == 'exit': r['command_exit'] = 0
            elif mutation == 'engine': r['engine_exit'] = 0
            elif mutation == 'bool': r['engine_exit'] = True
            elif mutation == 'wrapper': r['wrapper_exit'] = 3
            elif mutation == 'missing': del r['raw_native']
            elif mutation == 'not-lost': c['baseline_compare']['lost'] = []
            elif mutation == 'pass': c['tests'][case_id] = 'pass'
            elif mutation == 'id': rp['tests'][0]['id'] = 'other.html'
            elif mutation == 'duplicate': rp['tests'].append(copy.deepcopy(rp['tests'][0]))
            elif mutation == 'classification': rp['tests'][0]['classification'] = 'timeout'
            elif mutation == 'pin': rp['wpt_revision'] = 'f' * 40
            elif mutation == 'viewport': rp['viewport'] = {'width': 200, 'height': 300}
            elif mutation == 'outside': r['command'][r['command'].index('--out') + 1] = '/etc'
            elif mutation == 'command': r['command'][1] = 'private.py'
            elif mutation == 'subdir': r['command'][r['command'].index('--subdir') + 1] = 'css/other'
            elif mutation == 'out': r['command'].extend(['--out', str(root)])
            elif mutation == 'long': rp['tests'][0]['detail'] = 'renderer failure ' + 'x' * 100000
            elif mutation == 'unicode': rp['tests'][0]['detail'] = 'renderer failure ' + '日' * 30000
            compact.write_text(json.dumps(c))
            s['compact_files'][phase + '.compact.json'] = {'path': str(compact), 'bytes': compact.stat().st_size, 'sha256': d.digest(compact.read_bytes())}
            if mutation == 'hash': s['compact_files'][phase + '.compact.json']['sha256'] = '0' * 64
            if report.exists() or report.is_symlink(): report.unlink()
            report.write_text(json.dumps(rp))
            if mutation == 'symlink': report.unlink();report.symlink_to(compact)
            summary_path.write_text(json.dumps(s));terminal_path.write_text(json.dumps(terminal))
            args = [str(summary_path), d.digest(summary_path.read_bytes()), str(terminal_path), d.digest(terminal_path.read_bytes()), source, job, phase]
            try:
                body = d.extract(*args) if mode == 'phase' else d.extract_case(*args, case_id)
                value = json.loads(body)
                assert value['original_actual_exit'] == 1 and not value['counts_reconstructed'] and not value['tests_rerun']
                assert len(body) <= d.MAX_OUTPUT
                if mode == 'case':
                    assert value['case']['classification'] == 'crash' and value['classification_unchanged']
                    if mutation in ('long', 'unicode'): assert value['case']['detail_truncated']
                accepted = True
            except (ValueError, FileNotFoundError): accepted = False
            assert accepted == expected, name
            records.append({'id': name, 'classification': 'PASS'})
    with Path(output).open('x') as stream:json.dump({'schema': 1, 'pass': len(records), 'fail': 0, 'controls': records}, stream);stream.write('\n')


if __name__ == '__main__': run(sys.argv[1])
