"""Compare complete WPT records by test and exact subtest-name bytes."""
import collections, hashlib, json, pathlib, sys


def load(path):
    records = [json.loads(line) for line in pathlib.Path(path).read_text().splitlines()]
    names = [record['test'] for record in records]
    assert len(names) == len(set(names)), 'duplicate test records'
    assertions = collections.Counter()
    for record in records:
        for subtest in record['subtests']:
            name = subtest['name'].encode('utf-8', 'surrogatepass')
            key = (record['test'], hashlib.sha256(name).hexdigest(), subtest['status'])
            assertions[key] += 1
    return records, assertions


before, old = load(sys.argv[1])
after, new = load(sys.argv[2])
assert {r['test'] for r in before} == {r['test'] for r in after}, 'selection changed'
passes = lambda entries: collections.Counter({key[:2]: count for key, count in entries.items() if key[2] == 0})
gained, lost = passes(new) - passes(old), passes(old) - passes(new)
result = dict(selected=len(before), gained=sum(gained.values()), lost=sum(lost.values()),
              unchanged=old == new, gained_records=[[*key, count] for key, count in sorted(gained.items())],
              lost_records=[[*key, count] for key, count in sorted(lost.items())])
print(json.dumps(result, indent=2))
if lost:
    raise SystemExit(1)
