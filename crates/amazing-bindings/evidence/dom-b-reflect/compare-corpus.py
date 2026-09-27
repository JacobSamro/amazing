"""Compare two dom-b-audit run-corpus.py results.json files by (file, subtest name),
Counter-style (duplicate subtest names counted, not deduped into a set), and report
file-level status changes. Usage: compare-corpus.py before.json after.json"""
import json
import sys
import collections

before = json.load(open(sys.argv[1]))
after = json.load(open(sys.argv[2]))


def counter(report):
    passed = collections.Counter()
    failed = collections.Counter()
    for r in report['results']:
        test = r['test']
        for st in r.get('subtests', []):
            key = (test, st['name'])
            if st['status'] == 0:
                passed[key] += 1
            else:
                failed[key] += 1
    return passed, failed


bpass, bfail = counter(before)
apass, afail = counter(after)
bset, aset = set(bpass.elements()), set(apass.elements())
gained = aset - bset
lost = bset - aset
print('before pass subtests:', sum(bpass.values()), 'fail subtests:', sum(bfail.values()))
print('after  pass subtests:', sum(apass.values()), 'fail subtests:', sum(afail.values()))
print('gained (new passes):', len(gained))
print('lost (regressed):', len(lost))
for k in sorted(lost):
    print('LOST', k)
for k in sorted(gained):
    print('GAINED', k)

bstat = {r['test']: r['status'] for r in before['results']}
astat = {r['test']: r['status'] for r in after['results']}
changed = {t: (bstat[t], astat.get(t)) for t in bstat if astat.get(t) != bstat[t]}
print('file status changes:', len(changed))
for t, (b, a) in sorted(changed.items()):
    print('FILECHANGE', t, b, '->', a)
