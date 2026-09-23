"""Summarize paired, pinned LAYOUT-7 render reports and oracle movement."""
import json
from pathlib import Path

here = Path(__file__).parent
oracle = json.loads((here.parent / 'layout5' / 'browser-oracles.json').read_text())
consensus = {item['id'] for item in oracle['tests'] if item['consensus_pass']}


def read(name):
    return json.loads((here / f'{name}.json').read_text(encoding='utf-8-sig'))


def projection(report):
    return {
        item['id']: (
            item['classification'], item.get('actual_sha256'),
            item.get('reference_sha256'), item.get('diff_sha256'),
        )
        for item in report['tests']
    }


def pair(prefix):
    a, b = (read(f'{prefix}-{n}') for n in (1, 2))
    assert a['wpt_revision'] == b['wpt_revision'] == oracle['wpt_revision'], prefix
    assert a['viewport'] == b['viewport'], prefix
    assert projection(a) == projection(b), f'unstable {prefix}'
    return a


def compare(before, after):
    b, a = projection(before), projection(after)
    assert b.keys() == a.keys()
    gains = [name for name in b if b[name][0] != 'pass' and a[name][0] == 'pass']
    losses = [name for name in b if b[name][0] == 'pass' and a[name][0] != 'pass']
    return {
        'before_counts': before['counts'],
        'after_counts': after['counts'],
        'raw_gains': gains,
        'raw_losses': losses,
        'consensus_gains': [name for name in gains if name in consensus],
        'consensus_losses': [name for name in losses if name in consensus],
        'changed_pixels_or_classification': [name for name in b if b[name] != a[name]],
    }


result = {
    name: compare(pair(f'baseline-{name}'), pair(f'candidate-{name}'))
    for name in ('broad', 'focused')
}
for name in (
    'flex-grid', 'geometry', 'tables', 'floats', 'block-collapse',
    'position', 'text-flow', 'contents', 'paint-compositing', 'images',
):
    if (here / f'baseline-{name}-1.json').exists():
        result[name] = compare(pair(f'baseline-{name}'), pair(f'candidate-{name}'))
print(json.dumps(result, indent=2))
