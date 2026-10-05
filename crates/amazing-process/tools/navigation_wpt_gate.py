#!/usr/bin/env python3
"""Run every pinned Content Security Policy testharness source/variant; retain raw on host."""
import collections, concurrent.futures, hashlib, importlib.util, json, pathlib, re, sys


def write(path, value):
    data = (json.dumps(value, ensure_ascii=True, separators=(',', ':'), sort_keys=True) + '\n').encode()
    assert len(data) <= 512 * 1024, 'compact evidence size limit'
    path.write_bytes(data)


def identities(records):
    return collections.Counter((r['test'], s['name']) for r in records
                               for s in r['subtests'] if s['status'] == 0)


def summarize(path, records):
    return {'raw_path': str(path.resolve()), 'raw_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
            'files': dict(collections.Counter(r['status'] for r in records)),
            'subtests': dict(collections.Counter(str(s['status']) for r in records for s in r['subtests']))}


if sys.argv[1] == 'compare':
    before_path, after_path, output = map(pathlib.Path, sys.argv[2:5])
    before, after = [list(map(json.loads, p.read_text().splitlines())) for p in (before_path, after_path)]
    assert {r['test'] for r in before} == {r['test'] for r in after}, 'corpus changed'
    assert {r['test']: r['source_sha256'] for r in before} == {r['test']: r['source_sha256'] for r in after}, 'fixture changed'
    old, new = identities(before), identities(after)
    lost = [{'test': t, 'name': n, 'count': c} for (t, n), c in (old - new).items()]
    gained = [{'test': t, 'name': n, 'count': c} for (t, n), c in (new - old).items()]
    old_files = {r['test'] for r in before if r['status'] == 'pass'}
    new_files = {r['test'] for r in after if r['status'] == 'pass'}
    report = {'base': sys.argv[5], 'source': sys.argv[6], 'before': summarize(before_path, before),
              'after': summarize(after_path, after), 'lost_subtests': lost, 'gained_subtests': gained,
              'lost_files': sorted(old_files-new_files), 'gained_files': sorted(new_files-old_files)}
    write(output, report)
    print(json.dumps(report), flush=True)
    if lost or old_files-new_files:
        raise SystemExit('lost passing CSP tests')
else:
    root, exe, output = map(pathlib.Path, sys.argv[2:5])
    spec = importlib.util.spec_from_file_location('wpt', 'crates/amazing/src/cmd/measure-wpt-dom.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    selection = module.inventory(root, 'content-security-policy')
    entries = []
    for entry in selection['selected']:
        data = (root / entry['test']).read_bytes()
        variants = entry['variants']
        if entry['test'].endswith('.js'):
            variants = [v.decode('utf-8') for v in re.findall(
                rb'^//\s*META:\s*variant=(.*)\r?$', data, re.M)]
            entry['long_timeout'] = bool(re.search(
                rb'^//\s*META:\s*timeout=long\s*$', data, re.M))
        for variant in variants or ['']:
            # Classification is delegated to the installed harness, including
            # generated window wrappers; do not inherit old runner exclusions.
            entries.append(dict(entry, test=entry['test'] + variant, unsupported=None))
    assert entries, 'empty CSP corpus'
    def one(entry):
        result = module.run(exe.resolve(), root.resolve(), entry, 600000 if entry['long_timeout'] else 25000)
        result['source_sha256'] = entry['sha256']
        return result
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        records = list(pool.map(one, entries))
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(b''.join(module.encoded(r) for r in records))
    write(output.with_suffix('.compact.json'), {'revision': module.REVISION,
          'binary_sha256': hashlib.sha256(exe.read_bytes()).hexdigest(),
          'selection_sha256': hashlib.sha256(module.encoded(selection)).hexdigest(),
          **summarize(output, records), 'classifications': {r['test']: r['status'] for r in records}})
    print(json.dumps(summarize(output, records)), flush=True)
