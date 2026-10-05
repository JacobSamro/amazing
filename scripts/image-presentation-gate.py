#!/usr/bin/env python3
"""Whole installed-image gate: native raw originals, compact receipts only."""
import argparse
import collections
import hashlib
import importlib.util
import json
import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent
LEGACY_EMBEDDED_EXECUTOR_SHA256 = '4310869c9a008b546b0bf5e3ab24520f90c00beec5c047937c57279b7ca0fb2d'
SELECTORS = 'css-logical css-color css-values css-cascade css-conditional css-fonts css-text css-backgrounds css-transforms css-flexbox css-grid css-images css-sizing css-contain css-display selectors css-shadow'.split()
DIRS = 'css/css-fonts css/css-transforms css/css-images css/CSS2/floats css/CSS2/normal-flow css/CSS2/positioning css/css-backgrounds css/css-contain css/css-display css/css-overflow css/css-position css/css-ruby css/css-sizing css/css-tables css/css-viewport css/css-color css/css-cascade css/css-conditional css/selectors css/css-shadow'.split()


def describe(path):
    path = pathlib.Path(path)
    with path.open('rb') as stream:
        sha = hashlib.file_digest(stream, 'sha256').hexdigest()
    return {'path': str(path.resolve()), 'sha256': sha, 'bytes': path.stat().st_size}


def publish(path, value):
    data = (json.dumps(value, sort_keys=True, separators=(',', ':')) + '\n').encode()
    if len(data) > 512 * 1024:
        raise ValueError('compact evidence exceeds512KiB')
    with pathlib.Path(path).open('xb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--source', required=True)
    p.add_argument('--corpus-role', choices=('before', 'after'), required=True)
    p.add_argument('--raw-root', required=True)
    p.add_argument('--compact-root', required=True)
    p.add_argument('--baseline-dir')
    p.add_argument('--baseline-source')
    a = p.parse_args()
    if os.environ.get('AMAZING_SOURCE_COMMIT') != a.source:
        p.error('native source identity mismatch')
    if bool(a.baseline_dir) != bool(a.baseline_source):
        p.error('baseline directory and exact source must be supplied together')
    if (a.corpus_role == 'after') != bool(a.baseline_dir):
        p.error('corpus after requires baseline; corpus before cannot consume one')
    raw, compact = pathlib.Path(a.raw_root), pathlib.Path(a.compact_root)
    compact_parent = compact
    while not compact_parent.exists():
        compact_parent = compact_parent.parent
    capacity = os.statvfs(compact_parent)
    if capacity.f_bavail * capacity.f_frsize < 64 * 1024 * 1024:
        raise OSError('compact filesystem lacks declared64MiB publication headroom')
    raw.mkdir(parents=True, exist_ok=False)
    compact.mkdir(parents=True, exist_ok=True)
    (raw / 'tmp').mkdir()
    os.environ['TMPDIR'] = str((raw / 'tmp').resolve())
    baseline = pathlib.Path(a.baseline_dir) if a.baseline_dir else None
    if baseline:
        old = json.loads((baseline / 'gate.summary.json').read_text())
        if old['source_commit'] != a.baseline_source or old['job_exit'] != 0 or old['corpus_role'] != 'before':
            raise ValueError('baseline is not a completed successful exact-source phase')
        legacy = any(n.endswith('.phase.json') for n in old['compact_files'])
        for name, row in old['compact_files'].items():
            if name.endswith('.phase.json'):
                # These duplicate compact receipts are already present in the
                # authenticated public gate summary. Verify their original
                # hashes in place; never recover raw or reconstruct files.
                phase = next(r for r in old['phases'] if r['phase'] + '.phase.json' == name)
                data = (json.dumps(phase, sort_keys=True, separators=(',', ':')) + '\n').encode()
                if hashlib.sha256(data).hexdigest() != row['sha256'] or len(data) != row['bytes']:
                    raise ValueError('embedded phase receipt differs from original digest')
            elif describe(baseline / name)['sha256'] != row['sha256']:
                raise ValueError('baseline compact bytes changed')
        profile = {k: os.environ.get(k) for k in old['profile']}
        if profile != old['profile']:
            raise ValueError('corpus before/after profile differs')
        for name, row in old['tooling'].items():
            if describe(ROOT / name)['sha256'] != row['sha256']:
                if not (legacy and name == 'image-presentation-gate.py' and row['sha256'] == LEGACY_EMBEDDED_EXECUTOR_SHA256):
                    raise ValueError('corpus before/after classification adapter differs')
    phases, failed = [], False

    def run(label, argv):
        nonlocal failed
        log = raw / (label + '.log')
        with log.open('xb', buffering=0) as stream:
            result = subprocess.run(argv, stdout=stream, stderr=subprocess.STDOUT)
        code = result.returncode
        receipt = {'phase': label, 'source_commit': a.source, 'argv': argv,
                   'actual_returncode': code, 'raw': describe(log)}
        phases.append(receipt)
        failed = failed or code != 0
        publish(compact / (label + '.phase.manifest.json'), receipt)
        return code, log

    run('nextest', ['cargo', 'nextest', 'run', '--workspace', '--locked', '--no-fail-fast'])
    run('doctests', ['cargo', 'test', '--workspace', '--locked', '--doc'])
    ensured, _ = run('ensure-wpt', ['bash', 'scripts/ensure-wpt.sh'])
    built, _ = run('release-build', ['cargo', 'build', '-p', 'amazing', '--locked', '--release'])
    wpt = pathlib.Path.home() / 'corpora/wpt'
    os.environ['WPT_ROOT'] = str(wpt)
    parsed, log = run('parsing', ['cargo', 'run', '-p', 'amazing-css', '--example', 'wpt_conformance', '--locked', '--'] + SELECTORS)
    text = log.read_text(errors='replace')
    counts = re.search(r'TOTAL: (\d+)/(\d+) passed across (\d+) files \((\d+) non-literal args skipped\)', text)
    failures = collections.Counter(line.strip() for line in text.splitlines() if line.strip().startswith('FAIL '))
    parsing = {'source_commit': a.source, 'selections': SELECTORS, 'actual_returncode': parsed,
               'raw': describe(log), 'failure_multiset': dict(sorted(failures.items())),
               'qualification': 'Original stdout failure multiset and counts; no per-call pass IDs invented'}
    if not counts:
        failed = True
    else:
        parsing.update(zip(('passed', 'total', 'files', 'skipped_non_literal'), map(int, counts.groups())))
    if baseline:
        before = json.loads((baseline / 'parsing.summary.json').read_text())
        new = failures - collections.Counter(before['failure_multiset'])
        removed = collections.Counter(before['failure_multiset']) - failures
        cohort_same = all(before.get(k) == parsing.get(k) for k in ('selections', 'total', 'files', 'skipped_non_literal'))
        parsing['comparison'] = {'new_failures': dict(new), 'removed_failures': dict(removed), 'same_cohort': cohort_same}
        failed = failed or bool(new) or not cohort_same
    publish(compact / 'parsing.summary.json', parsing)
    if not ensured and not built:
        exe = pathlib.Path(os.environ['CARGO_TARGET_DIR']) / 'release/amazing'
        for subdir in DIRS:
            slug = subdir.replace('/', '-')
            out = raw / slug
            argv = [sys.executable, str(ROOT / 'wpt_render_compare.py'), '--wpt-root', str(wpt), '--subdir', subdir, '--exe', str(exe), '--out', str(out), '--width', '800', '--height', '600', '--timeout', '20', '--jobs', '4', '--limit', '0', '--compact']
            if baseline:
                argv += ['--baseline', str(baseline / (slug + '.compact.json'))]
            run(slug, argv)
            if (out / 'compact.json').is_file():
                value = json.loads((out / 'compact.json').read_text())
                publish(compact / (slug + '.compact.json'), value)
            else:
                failed = True
        argv = [sys.executable, str(ROOT / 'image-resource-corpus.py'), '--source', a.source, '--corpus-role', a.corpus_role, '--wpt-root', str(wpt), '--exe', str(exe), '--raw-out', str(raw / 'image-resources'), '--compact-out', str(compact / 'image-resources.compact.json')]
        if baseline:
            argv += ['--baseline', str(baseline / 'image-resources.compact.json')]
        run('image-resources', argv)
    spec = importlib.util.spec_from_file_location('gate_summary', ROOT / 'gate-summary.py')
    summary = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(summary)
    result = {'schema': 1, 'source_commit': a.source, 'baseline_source': a.baseline_source,
              'corpus_role': a.corpus_role, 'storage_lifecycle_phase': 'before',
              'profile': {k: os.environ.get(k) for k in ('CARGO_PROFILE_DEV_DEBUG', 'CARGO_PROFILE_TEST_DEBUG', 'CARGO_INCREMENTAL', 'CARGO_PROFILE_DEV_INCREMENTAL', 'CARGO_PROFILE_TEST_INCREMENTAL')},
              'placement': {'raw_device': raw.stat().st_dev, 'target_device': pathlib.Path(os.environ['CARGO_TARGET_DIR']).stat().st_dev, 'compact_device': compact.stat().st_dev},
              'job_exit': int(failed), 'phases': phases, 'render_dirs': DIRS, 'parsing_selectors': SELECTORS,
              'counts': summary.log_counts([r['raw']['path'] for r in phases]),
              'tooling': {n: describe(ROOT / n) for n in ('image-presentation-gate.py', 'image-resource-corpus.py', 'wpt_render_compare.py', 'gate-summary.py')},
              'compact_files': {f.name: describe(f) for f in compact.glob('*.json')},
              'raw_root': str(raw.resolve())}
    publish(compact / 'gate.summary.json', result)
    print(json.dumps({'source': a.source, 'actual_gate_exit': int(failed), 'summary': str(compact / 'gate.summary.json')}), flush=True)
    return int(failed)


if __name__ == '__main__':
    sys.exit(main())
