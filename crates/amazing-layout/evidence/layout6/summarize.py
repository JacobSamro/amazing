"""Verify the pinned LAYOUT-6 WPT comparisons and print measured movement."""
import json
from pathlib import Path

base = Path(__file__).parent
oracle = json.loads((base / 'browser-oracles.json').read_text())
manifest = json.loads((base / 'inline-block-manifest.json').read_text())
assert oracle['wpt_revision'] == manifest['revision']
assert {test['id'] for test in oracle['tests']} == {test['id'] for test in manifest['tests']}
consensus = {test['id'] for test in oracle['tests'] if test['consensus_pass']}


def read(name):
    return json.loads((base / (name + '.json')).read_text(encoding='utf-8-sig'))


def projection(report):
    return {
        test['id']: (
            test['classification'], test.get('actual_sha256'),
            test.get('reference_sha256'), test.get('diff_sha256'),
        )
        for test in report['tests']
    }


def pair(prefix):
    first, second = (read(f'{prefix}-{number}') for number in (1, 2))
    assert first['wpt_revision'] == second['wpt_revision'] == manifest['revision'], prefix
    assert first['viewport'] == second['viewport'], prefix
    assert projection(first) == projection(second), f'unstable {prefix}'
    return first


def movement(before, after):
    first, second = projection(before), projection(after)
    assert first.keys() == second.keys()
    gains = [name for name in first if first[name][0] != 'pass' and second[name][0] == 'pass']
    losses = [name for name in first if first[name][0] == 'pass' and second[name][0] != 'pass']
    return {
        'before_counts': before['counts'],
        'after_counts': after['counts'],
        'raw_gains': gains,
        'raw_losses': losses,
        'consensus_gains': [name for name in gains if name in consensus],
        'consensus_losses': [name for name in losses if name in consensus],
        'changed_pixels_or_classification': [name for name in first if first[name] != second[name]],
    }


result = {'inline_block': movement(pair('baseline-inline'), pair('final-inline')), 'retained': {}}
for suite in (
    'flex-grid-broad', 'flex-grid', 'geometry', 'tables', 'floats', 'block-collapse',
    'position', 'text-flow', 'contents', 'paint-compositing', 'images',
):
    before, after = pair(f'baseline-{suite}'), pair(f'final-{suite}')
    first, second = projection(before), projection(after)
    assert first.keys() == second.keys(), suite
    result['retained'][suite] = {
        'before_counts': before['counts'],
        'after_counts': after['counts'],
        'pass_losses': [name for name in first if first[name][0] == 'pass' and second[name][0] != 'pass'],
        'pass_gains': [name for name in first if first[name][0] != 'pass' and second[name][0] == 'pass'],
        'changed_pixels_or_classification': [name for name in first if first[name] != second[name]],
    }
print(json.dumps(result, indent=2))
