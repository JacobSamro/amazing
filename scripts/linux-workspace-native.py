#!/usr/bin/env python3
"""Whole workspace phases; raw logs native, bounded receipts captured separately."""
import hashlib
import importlib.util
import json
import os
import pathlib
import sys


def load(name, root):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), root / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def execute(config, commands=None):
    source = config['source']
    if os.environ.get('COMMIT') != source or os.environ.get('LANE') != config['lane']:
        raise ValueError('native source/lane identity differs from frozen request')
    for name in ('linux-workspace-native.py', 'gate-command.py', 'gate-summary.py'):
        actual = hashlib.sha256(pathlib.Path(__file__).with_name(name).read_bytes()).hexdigest()
        if actual != config['helper_sha256'][name]:
            raise ValueError('frozen helper hash mismatch: ' + name)
    if any(os.environ.get(k) != v for k, v in config['profile'].items()):
        raise ValueError('actual Cargo profile differs from frozen request')
    raw = pathlib.Path(config['raw_root'])
    compact = pathlib.Path(config['compact_absolute'])
    raw.mkdir(parents=True, exist_ok=True)
    compact.mkdir(parents=True, exist_ok=True)
    helpers = pathlib.Path(__file__).parent
    phase = load('gate-command', helpers)
    summary = load('gate-summary', helpers)
    commands = commands or [
        ['cargo', 'nextest', 'run', '--workspace', '--locked', '--no-fail-fast'],
        ['cargo', 'test', '--workspace', '--locked', '--doc']]
    details = []
    for name, argv in zip(('nextest', 'doctests'), commands):
        # gate-command's private stream and requested copy both stay native.
        detail = phase.run(argv, name, source, raw / (name + '.log'),
                           compact / (name + '.phase.manifest.json'))
        details.append(detail)
    code = 0 if all(d['wrapper_exit'] == 0 for d in details) else (
        1 if any(d['engine_exit'] not in (None, 0) for d in details) else 3)
    value = {'schema': 1, 'source_commit': source, 'lane': config['lane'],
             'job': config['job'], 'corpus_role': 'standalone workspace candidate',
             'storage_phase': 'before', 'job_exit': code,
             'phase_exits': {d['phase']: d['engine_exit'] for d in details},
             'phase_details': details,
             'counts': summary.log_counts([raw / 'nextest.log', raw / 'doctests.log']),
             'raw': [summary.raw(raw / (n + '.log')) for n in ('nextest', 'doctests')],
             'helper_sha256': config['helper_sha256'], 'profile': config['profile']}
    # A failed publisher is tooling, never a fabricated successful gate.
    summary.write(compact / 'workspace.manifest.json', value)
    print(json.dumps({'source_commit': source, 'job_exit': code,
                      'phase_exits': value['phase_exits'], 'counts': value['counts']}), flush=True)
    return code


if __name__ == '__main__':
    try:
        raise SystemExit(execute(json.loads(pathlib.Path(sys.argv[1]).read_text())))
    except (OSError, ValueError, KeyError) as error:
        print(json.dumps({'wrapper_exit': 3, 'error': str(error)}), flush=True)
        raise SystemExit(3)
