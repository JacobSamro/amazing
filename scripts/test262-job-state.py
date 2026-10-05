#!/usr/bin/env python3
"""Resolve a captured shard through the public native lifecycle contract."""
import argparse
import importlib.util
import json
from pathlib import Path
import tempfile
import time

SCRIPT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('completed_job', SCRIPT / 'completed-job.py')
job = importlib.util.module_from_spec(spec)
spec.loader.exec_module(job)


def resolve(args, api=job):
    """Observer exit is diagnostic only; native terminal evidence decides."""
    summary = {'schema': 1, 'source_commit': args.source, 'lane': args.lane,
               'endpoint_receipt': args.receipt, 'observer_exit': args.observer_exit,
               'state': 'UNKNOWN', 'actual_exit': None, 'runner_exit': None,
               'compact_fetch_allowed': False, 'observations': 0}
    deadline = time.monotonic() + args.seconds
    try:
        # This is the qualified public helper beside this controller, never MAIN.
        for name in ('status', 'receipt', 'identity'):
            if not callable(getattr(api, name, None)):
                raise ValueError('public helper lacks the qualified lifecycle API')
        row = api.receipt(args)
        summary.update(job=api.identity(row), box_id=row['box_id'],
                       remote_output_root=row['output_root'])
        with tempfile.TemporaryDirectory(prefix='test262-native-observation-') as scratch:
            while True:
                left = deadline - time.monotonic()
                if left <= 0:
                    summary['reason'] = 'bounded wait expired with authenticated RUNNING job'
                    break
                # Unique output for every public call: an interrupted observer
                # cannot leave a previous receipt to masquerade as this result.
                output = Path(scratch) / f"observation-{summary['observations']}.summary.json"
                call = argparse.Namespace(receipt=args.receipt, lane=args.lane,
                                          source=args.source, output=str(output),
                                          seconds=max(1, min(60, int(left))))
                api.status(call, True)
                observed = json.loads(output.read_text())
                if observed.get('job') != api.identity(row):
                    raise ValueError('public observation identity mismatch')
                if observed.get('state') not in ('UNKNOWN', 'RUNNING', 'TERMINAL'):
                    raise ValueError('invalid public native state')
                summary.update(observed)
                summary['observations'] += 1
                if summary['state'] != 'RUNNING':
                    break
        terminal = (summary['state'] == 'TERMINAL'
                    and type(summary.get('actual_exit')) is int
                    and type(summary.get('runner_exit')) is int)
        summary['compact_fetch_allowed'] = terminal
        code = summary['runner_exit'] if terminal else 3
    except (OSError, ValueError, TypeError, KeyError, AttributeError) as error:
        summary.update(state='UNKNOWN', actual_exit=None, runner_exit=None,
                       compact_fetch_allowed=False, reason=str(error)[:1200])
        code = 3
    destination = Path(args.output)
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(summary, indent=2) + '\n')
    if not summary['compact_fetch_allowed']:
        question = Path(args.questions)
        question.parent.mkdir(parents=True, exist_ok=True)
        question.write_text('# Test262 native observation unresolved\n\n'
                            f"Source {args.source}; lane {args.lane}; observer exit {args.observer_exit}.\n"
                            f"Native state {summary['state']}; actual exit {summary.get('actual_exit')}; "
                            f"runner exit {summary.get('runner_exit')}.\n"
                            f"Endpoint {args.receipt}; raw host root {summary.get('remote_output_root', 'not captured')}.\n"
                            f"Reason: {summary.get('reason', 'no terminal witness')}.\n"
                            'No compact fetched, no raw recovery and no native exit inferred.\n')
    return code


def main():
    parser = argparse.ArgumentParser()
    for name in ('receipt', 'lane', 'source', 'output', 'questions'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--observer-exit', type=int, required=True)
    parser.add_argument('--seconds', type=int, default=3600)
    args = parser.parse_args()
    if not 1 <= args.seconds <= 7200:
        parser.error('wait bound must be1..7200 seconds')
    return resolve(args)


if __name__ == '__main__':
    raise SystemExit(main())
