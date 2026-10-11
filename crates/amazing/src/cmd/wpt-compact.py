#!/usr/bin/env python3
"""Compact before/after evidence for measure-wpt-dom.py runs.

usage: wpt-compact.py <suite> <before-dir> <after-dir> > compact.json
Reads round-1 jsonl of each run. Output keeps counts, per-file classification
of the after run, every lost/gained file, and every lost/gained passing subtest
(by test and subtest name), plus the sha256 of each raw file. An empty `lost_*`
list is a claim the reader can re-check against the raw files.
"""
import hashlib
import json
import sys
from pathlib import Path


def load(directory, suite):
    """Every round's rows; a test counts as passing only if it passes in all
    rounds ("stable"), as passing at all if it passes in any ("any")."""
    rounds, shas = [], []
    for number in (1, 2):
        path = Path(directory) / (suite + '-%d.jsonl' % number)
        if not path.exists():
            continue
        raw = path.read_bytes()
        shas.append(hashlib.sha256(raw).hexdigest())
        rounds.append({json.loads(l)['test']: json.loads(l) for l in raw.splitlines()})
    return rounds, shas


def passed(r):
    return {s['name'] for s in r['subtests'] if s['status'] == 0}


def stable_pass(rounds, test):
    return all(rows.get(test, {}).get('status') == 'pass' for rows in rounds)


def any_pass(rounds, test):
    return any(rows.get(test, {}).get('status') == 'pass' for rows in rounds)


def stable_subtests(rounds, test):
    sets = [passed(rows[test]) if test in rows else set() for rows in rounds]
    return set.intersection(*sets) if sets else set()


def any_subtests(rounds, test):
    out = set()
    for rows in rounds:
        if test in rows:
            out |= passed(rows[test])
    return out


def totals(rounds):
    rows = rounds[0]
    files = {}
    for r in rows.values():
        files[r['status']] = files.get(r['status'], 0) + 1
    return {'files': files, 'pass_subtests': sum(len(passed(r)) for r in rows.values()),
            'rounds': len(rounds)}


def main():
    suite, before_dir, after_dir = sys.argv[1:4]
    before, before_sha = load(before_dir, suite)
    after, after_sha = load(after_dir, suite)
    tests = sorted(set(before[0]) | set(after[0]))
    lost_files = [t for t in tests if stable_pass(before, t) and not any_pass(after, t)]
    flaky_files = [t for t in tests if stable_pass(before, t) and any_pass(after, t) and not stable_pass(after, t)]
    gained_files = [t for t in tests if stable_pass(after, t) and not any_pass(before, t)]
    lost_sub, gained_sub = [], []
    for t in tests:
        b, a = stable_subtests(before, t), any_subtests(after, t)
        lost_sub += [[t, n] for n in sorted(b - a)]
        gained_sub += [[t, n] for n in sorted(stable_subtests(after, t) - any_subtests(before, t))]
    out = {'suite': suite, 'before': totals(before), 'after': totals(after),
           'raw_sha256': {'before': before_sha, 'after': after_sha},
           'rule': 'lost = passes in every before round and in no after round; gained = passes in every after round and no before round',
           'lost_files': lost_files, 'flaky_files_passing_in_some_after_round_only': flaky_files,
           'gained_files': gained_files, 'lost_subtests': lost_sub,
           'gained_subtests_count': len(gained_sub),
           'after_status': {t: after[0][t]['status'] for t in sorted(after[0])}}
    json.dump(out, sys.stdout, separators=(',', ':'))
    print()


if __name__ == '__main__':
    main()
