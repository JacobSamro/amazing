"""Retain every classification and exact name-byte hash from a full sweep."""
import hashlib, json, pathlib, sys

source, output = map(pathlib.Path, sys.argv[1:3])
records = [json.loads(line) for line in source.read_text().splitlines()]
assert len({r['test'] for r in records}) == len(records)
compact = []
for record in records:
    names = [[hashlib.sha256(subtest['name'].encode('utf-8', 'surrogatepass')).hexdigest(), subtest['status']]
             for subtest in record['subtests']]
    assert sum(record['counts'].values()) == len(names)
    row = dict(test=record['test'], status=record['status'], counts=record['counts'], names=names,
               harness_status=record.get('harness_status'))
    if 'source_sha256' in record:
        row['source_sha256'] = record['source_sha256']
    compact.append(row)
output.write_text(''.join(json.dumps(row, separators=(',', ':')) + '\n' for row in compact))
