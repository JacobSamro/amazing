#!/usr/bin/env python3
"""Pinned Linux counterpart of run-render-wpt.ps1 (same first-link/fuzzy rules)."""
import argparse
import concurrent.futures
import hashlib
import json
import os
import re
import subprocess
from collections import Counter
from pathlib import Path
from urllib.parse import unquote


def attr(tag, name):
    match = re.search(r'\b' + re.escape(name) + r'''\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))''', tag, re.I | re.S)
    return next((v for v in match.groups() if v is not None), None) if match else None


def reference(source, path, root):
    for tag in re.findall(r'<link\b[^>]*>', source, re.I | re.S):
        rel, href = attr(tag, 'rel'), attr(tag, 'href')
        if not rel or not href:
            continue
        rel = rel.lower().split()
        relation = 'match' if 'match' in rel else 'mismatch' if 'mismatch' in rel else None
        if relation is None:
            continue
        if re.match(r'^[A-Za-z][A-Za-z0-9+.-]*:', href) or href.startswith('//'):
            raise ValueError('remote reftest link')
        # URL attributes trim surrounding ASCII whitespace; Windows path
        # normalization had hidden this upstream fixture's trailing space.
        clean = unquote(re.split(r'[?#]', href.strip(' \t\r\n\f'), 1)[0])
        target = ((root if clean.startswith('/') else path.parent) / clean.lstrip('/')).resolve()
        target.relative_to(root)
        return relation, href, target
    raise ValueError('no upstream reftest link')


def fuzzy(source):
    for tag in re.findall(r'<meta\b[^>]*>', source, re.I | re.S):
        if (attr(tag, 'name') or '').lower() != 'fuzzy':
            continue
        content = attr(tag, 'content') or ''
        maximum = re.search(r'maxDifference\s*=\s*\d+-(\d+)', content)
        pixels = re.search(r'totalPixels\s*=\s*\d+-(\d+)', content)
        return int(maximum[1]) if maximum else 0, int(pixels[1]) if pixels else 0
    return 0, 0


def ppm(path):
    data = path.read_bytes()
    tokens, index = [], 0
    while len(tokens) < 4:
        while index < len(data) and data[index] in b' \r\n\t':
            index += 1
        if data[index] == 35:
            index = data.index(b'\n', index)
            continue
        start = index
        while index < len(data) and data[index] not in b' \r\n\t':
            index += 1
        tokens.append(data[start:index])
    if tokens[0] != b'P6' or tokens[3] != b'255':
        raise ValueError('unsupported PPM')
    width, height = int(tokens[1]), int(tokens[2])
    pixels = data[index + 1:]
    if len(pixels) != width * height * 3:
        raise ValueError('invalid PPM length')
    return width, height, pixels


def compare(actual, expected, diff):
    aw, ah, a = ppm(actual)
    rw, rh, b = ppm(expected)
    if (aw, ah) != (rw, rh):
        raise ValueError('image dimensions differ')
    changed = bytearray(aw * ah)
    maximum = 0
    if a != b:
        for i, (x, y) in enumerate(zip(a, b)):
            delta = abs(x - y)
            if delta:
                changed[i // 3] = 1
                maximum = max(maximum, delta)
    pixels = bytearray(b'\xff\x00\xff' * (aw * ah))
    pixels[1::3] = bytes(0 if value else 255 for value in changed)
    diff.write_bytes(f'P6\n{aw} {ah}\n255\n'.encode() + pixels)
    return sum(changed), maximum


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def render(binary, source, output, root):
    try:
        process = subprocess.run([str(binary), 'render-wpt', str(source), str(output), '800', '600'],
                                 capture_output=True, timeout=20, env=dict(os.environ, AMAZING_WPT_ROOT=str(root)))
    except subprocess.TimeoutExpired:
        return 'timeout', 'render exceeded 20 seconds'
    output.with_suffix('.log').write_bytes(process.stdout + process.stderr)
    if process.returncode:
        return {2: 'error', 3: 'unsupported'}.get(process.returncode, 'crash'), f'exit {process.returncode}'
    if not output.exists():
        return 'crash', 'no image'
    return 'rendered', ''


def run_case(case, args):
    result = {'id': case['id'], 'classification': 'error'}
    directory = args.output / case['id']
    directory.mkdir()
    try:
        path = (args.wpt / case['test']).resolve()
        path.relative_to(args.wpt)
        source = path.read_text(encoding='utf-8-sig', errors='replace')
        try:
            relation, href, ref = reference(source, path, args.wpt)
        except ValueError as error:
            return dict(result, classification='unsupported', detail=str(error))
        if not ref.is_file():
            return dict(result, classification='unsupported', detail=f'missing reference: {ref}')
        actual, expected, diff = [directory / name for name in ['actual.ppm', 'reference.ppm', 'diff.ppm']]
        for input_path, output_path in [(path, actual), (ref, expected)]:
            classification, detail = render(args.binary, input_path, output_path, args.wpt)
            if classification != 'rendered':
                return dict(result, classification=classification, detail=detail)
        changed, maximum = compare(actual, expected, diff)
        allowed_max, allowed_pixels = fuzzy(source)
        matched = maximum <= allowed_max and changed <= allowed_pixels
        passed = matched if relation == 'match' else not matched
        result.update(classification='pass' if passed else 'mismatch', relation=relation,
                      reference_href=href, differing_pixels=changed, max_difference=maximum,
                      allowed_max_difference=allowed_max, allowed_differing_pixels=allowed_pixels,
                      actual_sha256=digest(actual), reference_sha256=digest(expected), diff_sha256=digest(diff))
    except (OSError, ValueError) as error:
        result['detail'] = str(error)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--wpt', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--source', required=True)
    parser.add_argument('--jobs', type=int, default=3)
    args = parser.parse_args()
    args.wpt, args.binary, args.output = args.wpt.resolve(), args.binary.resolve(), args.output.resolve()
    manifest = json.loads(args.manifest.read_text())
    revision = subprocess.check_output(['git', '-C', str(args.wpt), 'rev-parse', 'HEAD'], text=True).strip()
    if revision != manifest['revision']:
        raise SystemExit('WPT revision differs from manifest')
    ids = [case['id'] for case in manifest['tests']]
    if len(ids) != len(set(ids)):
        raise SystemExit('duplicate case IDs')
    args.output.mkdir(parents=True, exist_ok=False)
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        results = list(pool.map(lambda case: run_case(case, args), manifest['tests']))
    counts = dict.fromkeys(['pass', 'mismatch', 'error', 'timeout', 'unsupported', 'crash'], 0)
    counts.update(Counter(case['classification'] for case in results))
    report = dict(schema=1, wpt_revision=revision, viewport=dict(width=800, height=600, device_scale=1),
                  source_commit=args.source, executable_sha256=digest(args.binary),
                  manifest_sha256=digest(args.manifest), platform='Linux', counts=counts, tests=results)
    (args.output / 'report.json').write_text(json.dumps(report, separators=(',', ':')) + '\n')
    print(json.dumps(dict(output=str(args.output), source=args.source, counts=counts)), flush=True)
    return int(any(counts[key] for key in ['error', 'timeout', 'unsupported', 'crash']))


if __name__ == '__main__':
    raise SystemExit(main())
