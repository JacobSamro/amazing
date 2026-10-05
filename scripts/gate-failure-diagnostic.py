#!/usr/bin/env python3
"""Publish bounded assertion context from a hash-bound, actually failed gate phase.

This reads retained native logs in place. It never reruns tests, recovers an
archive, alters receipts, or derives pass/fail counts from log text.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat

MAX_RAW = 16 * 1024 * 1024
MAX_OUTPUT = 64 * 1024
ANSI = re.compile(r'\x1b\[[0-?]*[ -/]*[@-~]')
FAIL = re.compile(r'(?:\bFAIL\b|\bFAILED\b|panicked at|assertion .*failed|assertion failed|^\s*(?:left|right):)')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def checked_file(path, expected, limit, size=None):
    path = Path(path)
    if not path.is_absolute() or path.resolve() != path:
        raise ValueError('input must be canonical, absolute and nonsymlinked')
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, 'rb') as stream:
        meta = os.fstat(stream.fileno())
        if not stat.S_ISREG(meta.st_mode) or meta.st_size > limit:
            raise ValueError('input is not a bounded regular file')
        if size is not None and meta.st_size != size:
            raise ValueError('input byte count mismatch')
        data = stream.read(limit + 1)
    if len(data) > limit or digest(data) != expected:
        raise ValueError('input size/hash mismatch')
    return data


def extract(summary_path, summary_sha, terminal_path, terminal_sha, source, job, phase):
    if not re.fullmatch(r'[a-f0-9]{40}', source) or not re.fullmatch(r'[a-f0-9]{32}', job):
        raise ValueError('full source and captured job identifiers required')
    summary = json.loads(checked_file(summary_path, summary_sha, 512 * 1024))
    terminal = json.loads(checked_file(terminal_path, terminal_sha, 64 * 1024))
    endpoint = terminal.get('job', {})
    actual = terminal.get('actual_exit')
    if (terminal.get('state') != 'TERMINAL' or type(actual) is not int or actual == 0
            or type(terminal.get('runner_exit')) is not int
            or type(terminal.get('command_returncode')) is not int
            or terminal.get('runner_exit') != actual
            or terminal.get('command_returncode') != actual
            or endpoint.get('source_commit') != source or endpoint.get('job_id') != job):
        raise ValueError('original authenticated terminal failure required')
    if summary.get('source_commit') != source or summary.get('job_exit') != actual:
        raise ValueError('summary does not bind original source/native exit')
    matches = [row for row in summary.get('phases', []) if row.get('phase') == phase]
    if len(matches) != 1:
        raise ValueError('phase must be present exactly once')
    row = matches[0]
    if (row.get('source_commit') != source or type(row.get('actual_returncode')) is not int
            or row['actual_returncode'] == 0):
        raise ValueError('phase must have an actual failing exit')
    raw = row['raw']
    if not Path(raw['path']).is_relative_to('/tmp') or type(raw['bytes']) is not int:
        raise ValueError('retained native /tmp raw binding required')
    data = checked_file(raw['path'], raw['sha256'], MAX_RAW, raw['bytes'])
    lines = ANSI.sub('', data.decode('utf-8', errors='replace')).splitlines()
    hits = [i for i, line in enumerate(lines) if FAIL.search(line)]
    if not hits:
        raise ValueError('no failure/assertion marker; no inferred diagnostic')
    # Preserve line numbers and matched lines. Snippets may be truncated, never
    # reclassified. This is diagnostic context, not a substitute for the gate.
    selected = sorted({j for i in hits for j in range(max(0, i - 2), min(len(lines), i + 8))})
    snippets, used, omitted = [], 0, 0
    for index in selected:
        text = lines[index]
        encoded = text.encode('utf-8')
        clipped = len(encoded) > 2048
        if clipped:
            text = encoded[:2048].decode('utf-8', errors='ignore')
        entry = {'line': index + 1, 'text': text, 'marker': index in hits,
                 'line_truncated': clipped}
        cost = len(json.dumps(entry, ensure_ascii=True).encode()) + 2
        if used + cost > 48 * 1024:
            omitted += 1
            continue
        snippets.append(entry)
        used += cost
    result = {'schema': 1, 'kind': 'failed-phase-diagnostic', 'source_commit': source,
              'original_job': endpoint, 'original_state': terminal['state'],
              'original_actual_exit': actual, 'original_runner_exit': terminal['runner_exit'],
              'summary': {'path': str(summary_path), 'sha256': summary_sha},
              'terminal': {'path': str(terminal_path), 'sha256': terminal_sha},
              'phase': phase, 'phase_actual_exit': row['actual_returncode'],
              'phase_argv': row.get('argv'), 'raw': raw, 'snippets': snippets,
              'matched_marker_lines': len(hits), 'omitted_context_lines': omitted,
              'counts_reconstructed': False, 'tests_rerun': False}
    output = (json.dumps(result, sort_keys=True, ensure_ascii=True) + '\n').encode()
    if len(output) > MAX_OUTPUT:
        raise ValueError('diagnostic exceeds 64 KiB')
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('summary', 'summary-sha256', 'terminal', 'terminal-sha256',
                'source', 'job', 'phase', 'output'):
        parser.add_argument('--' + key, required=True)
    args = parser.parse_args()
    output = extract(args.summary, args.summary_sha256, args.terminal,
                     args.terminal_sha256, args.source, args.job, args.phase)
    destination = Path(args.output)
    if not destination.is_absolute() or destination.parent.resolve() != destination.parent:
        raise ValueError('fresh canonical output path required')
    with destination.open('xb') as stream:
        stream.write(output)
    print(json.dumps({'bytes': len(output), 'sha256': digest(output),
                      'path': str(destination)}, sort_keys=True))


if __name__ == '__main__':
    main()
