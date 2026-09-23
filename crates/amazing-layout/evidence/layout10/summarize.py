"""Audit LAYOUT-10 fixed-base WPT movement and stable repeat hashes."""
import json
from pathlib import Path

here = Path(__file__).parent
layout = here.parent
calc_oracle = json.loads((layout / 'layout8/browser-oracles.json').read_text())
broad_oracle = json.loads((layout / 'layout5/browser-oracles.json').read_text())
consensus = {
    'calc': {test['id'] for test in calc_oracle['tests'] if test['consensus_pass']},
    'broad': {test['id'] for test in broad_oracle['tests'] if test['consensus_pass']},
}
revision = calc_oracle['wpt_revision']


def read(path):
    return json.loads(path.read_text(encoding='utf-8-sig'))


def projection(report):
    return {
        test['id']: (
            test['classification'], test.get('actual_sha256'),
            test.get('reference_sha256'), test.get('diff_sha256'),
        )
        for test in report['tests']
    }


def pair(prefix, suite):
    first, second = (read(here / f'{prefix}-{suite}-{n}.json') for n in (1, 2))
    assert first['wpt_revision'] == second['wpt_revision'] == revision, (prefix, suite)
    assert first['viewport'] == second['viewport'], (prefix, suite)
    assert projection(first) == projection(second), f'unstable {prefix} {suite}'
    return first


def compare(before, after, known):
    old, new = projection(before), projection(after)
    assert old.keys() == new.keys()
    gains = [name for name in old if old[name][0] != 'pass' and new[name][0] == 'pass']
    losses = [name for name in old if old[name][0] == 'pass' and new[name][0] != 'pass']
    return {
        'before_counts': before['counts'],
        'after_counts': after['counts'],
        'raw_gains': gains,
        'raw_losses': losses,
        'consensus_gains': [name for name in gains if name in known],
        'consensus_losses': [name for name in losses if name in known],
        'changed_images_or_classification': [name for name in old if old[name] != new[name]],
    }


previous_calc = read(layout / 'layout8/calc-1.json')
previous_broad = read(layout / 'layout9/reports/final-flex-grid-broad-1.json')
base_calc, base_broad = (pair('baseline', suite) for suite in ('calc', 'broad'))
final_calc, final_broad = (pair('candidate', suite) for suite in ('calc', 'broad'))
result = {
    'css9_calc': compare(previous_calc, base_calc, consensus['calc']),
    'css9_broad': compare(previous_broad, base_broad, consensus['broad']),
    'layout10_calc': compare(base_calc, final_calc, consensus['calc']),
    'layout10_broad': compare(base_broad, final_broad, consensus['broad']),
}
for suite in (
    'flex-grid', 'geometry', 'tables', 'floats', 'block-collapse', 'position',
    'text-flow', 'contents', 'paint-compositing', 'images',
):
    result[suite] = compare(pair('baseline', suite), pair('candidate', suite), consensus['broad'])
named = [
    'css-values__calc-max-height-block-1.html',
    'css-values__calc-min-width-block-intrinsic-1.html',
    'css-values__calc-rounding-003.html',
    'css-values__calc-border-radius-1.html',
    'css-flexbox__gap-006-rtl.html',
    'css-flexbox__gap-006-lr.html',
]
result['named'] = {
    name: {
        'before': projection(base_calc if name.startswith('css-values') else base_broad)[name],
        'after': projection(final_calc if name.startswith('css-values') else final_broad)[name],
    }
    for name in named
}
(here / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({name: {
    'before': item['before_counts']['pass'],
    'after': item['after_counts']['pass'],
    'gains': len(item['raw_gains']),
    'losses': len(item['raw_losses']),
    'consensus_gains': len(item['consensus_gains']),
    'consensus_losses': len(item['consensus_losses']),
} for name, item in result.items() if name != 'named'}, indent=2))
