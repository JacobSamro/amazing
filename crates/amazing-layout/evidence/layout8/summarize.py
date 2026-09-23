"""Compare pinned LAYOUT-8 repeats with its fixed-base and LAYOUT-7 retained gates."""
import json
from pathlib import Path

here = Path(__file__).parent
prior = here.parent / 'layout7'
calc_oracle = json.loads((here / 'browser-oracles.json').read_text())
retained_oracle = json.loads((here.parent / 'layout5' / 'browser-oracles.json').read_text())
consensus = {
    'calc': {item['id'] for item in calc_oracle['tests'] if item['consensus_pass']},
    'retained': {item['id'] for item in retained_oracle['tests'] if item['consensus_pass']},
}


def read(path):
    return json.loads(path.read_text(encoding='utf-8-sig'))


def projection(report):
    return {
        item['id']: (
            item['classification'], item.get('actual_sha256'),
            item.get('reference_sha256'), item.get('diff_sha256'),
        )
        for item in report['tests']
    }


def pair(folder, name):
    a, b = (read(folder / f'{name}-{n}.json') for n in (1, 2))
    assert a['wpt_revision'] == b['wpt_revision'] == calc_oracle['wpt_revision'], name
    assert a['viewport'] == b['viewport'], name
    assert projection(a) == projection(b), f'unstable {name}'
    return a


def compare(before, after, known):
    b, a = projection(before), projection(after)
    assert b.keys() == a.keys()
    gains = [name for name in b if b[name][0] != 'pass' and a[name][0] == 'pass']
    losses = [name for name in b if b[name][0] == 'pass' and a[name][0] != 'pass']
    return {
        'before_counts': before['counts'],
        'after_counts': after['counts'],
        'raw_gains': gains,
        'raw_losses': losses,
        'consensus_gains': [name for name in gains if name in known],
        'consensus_losses': [name for name in losses if name in known],
        'changed_images_or_classification': [name for name in b if b[name] != a[name]],
    }


result = {
    'calc': compare(pair(here, 'baseline'), pair(here, 'calc'), consensus['calc']),
    'broad': compare(pair(prior, 'candidate-broad'), pair(here, 'broad'), consensus['retained']),
}
for suite in (
    'flex-grid', 'geometry', 'tables', 'floats', 'block-collapse',
    'position', 'text-flow', 'contents', 'paint-compositing', 'images',
):
    result[suite] = compare(
        pair(prior, f'candidate-{suite}'), pair(here, suite), consensus['retained']
    )
(here / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({name: {
    'before': item['before_counts']['pass'],
    'after': item['after_counts']['pass'],
    'gains': len(item['raw_gains']),
    'losses': len(item['raw_losses']),
    'consensus_gains': len(item['consensus_gains']),
    'consensus_losses': len(item['consensus_losses']),
} for name, item in result.items()}, indent=2))
