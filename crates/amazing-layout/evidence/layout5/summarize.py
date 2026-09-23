"""Compare pinned LAYOUT-5 WPT reports and exact-revision browser consensus."""
import json
import sys
from pathlib import Path

base = Path(__file__).parent
reports = Path(sys.argv[1])
features = json.loads((base / 'browser-oracles.json').read_text())
oracles = {item['id']: item for item in features['tests']}

def read(name):
    directory = base if name.startswith('broad-baseline-') else reports
    return json.loads((directory / f'{name}.json').read_text(encoding='utf-8-sig'))

def projection(report):
    return {item['id']: (item['classification'], item.get('actual_sha256'),
                         item.get('reference_sha256'), item.get('diff_sha256'))
            for item in report['tests']}

def pair(phase, suite):
    names = ([f'broad-baseline-{repeat}' for repeat in (1, 2)]
             if phase == 'baseline' and suite == 'flex-grid-broad'
             else [f'{phase}-{suite}-{repeat}' for repeat in (1, 2)])
    first, second = (read(name) for name in names)
    assert first['wpt_revision'] == second['wpt_revision'] == features['wpt_revision']
    assert first['viewport'] == second['viewport'] == {'width': 800, 'height': 600, 'device_scale': 1}
    assert projection(first) == projection(second), f'unstable {phase} {suite}'
    return first

baseline = pair('baseline', 'flex-grid-broad')
slice_ = pair('slice', 'flex-grid-broad')
final = pair('final', 'flex-grid-broad')
start, mid, end = (projection(report) for report in (baseline, slice_, final))
assert start.keys() == mid.keys() == end.keys() == oracles.keys()

def movements(before, after):
    gains = [name for name in before if before[name][0] != 'pass' and after[name][0] == 'pass']
    losses = [name for name in before if before[name][0] == 'pass' and after[name][0] != 'pass']
    changes = [name for name in before if before[name] != after[name]]
    return {'raw_gains': gains, 'raw_losses': losses,
            'consensus_gains': [name for name in gains if oracles[name]['consensus_pass']],
            'consensus_losses': [name for name in losses if oracles[name]['consensus_pass']],
            'changed_pixels_or_classification': changes}

summary = {'counts': {'baseline': baseline['counts'], 'baseline_alignment': slice_['counts'],
                      'combined': final['counts']},
           'baseline_alignment': movements(start, mid),
           'outer_inline_increment': movements(mid, end),
           'overall': movements(start, end),
           'regressions': {}}
layout4 = base.parent / 'layout4'
for suite in ('flex-grid','geometry','tables','floats','block-collapse','position',
              'text-flow','contents','paint-compositing','images'):
    report = pair('final', suite)
    previous = json.loads((layout4 / f'final-{suite}-1.json').read_text(encoding='utf-8-sig'))
    before, after = projection(previous), projection(report)
    assert before.keys() == after.keys(), f'changed manifest for {suite}'
    summary['regressions'][suite] = {
        'previous_counts': previous['counts'],
        'final_counts': report['counts'],
        'pass_losses': [name for name in before
                        if before[name][0] == 'pass' and after[name][0] != 'pass'],
        'pass_gains': [name for name in before
                       if before[name][0] != 'pass' and after[name][0] == 'pass'],
    }
print(json.dumps(summary, indent=2))
