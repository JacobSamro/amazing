#!/usr/bin/env python3
"""Optional Test262 placement: one upload/build per box, original shards retained."""
import argparse
import base64
import concurrent.futures
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys
import time
import importlib.util

SCRIPTS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('pool_native', SCRIPTS / 'test262-pool-native.py')
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


def run(argv, **kwargs):
    return subprocess.run([str(x) for x in argv], **kwargs).returncode


def command(source, lane, run_id, shards, total, runner):
    staging = f'/tmp/amazing-t262-pool-{run_id}-{lane}'
    quote = shlex.quote
    lines = ['set -euo pipefail', f'export CARGO_TARGET_DIR={quote(staging + "/target")} CARGO_INCREMENTAL=0',
             f'mkdir -p {quote(staging + "/source")} {quote(staging + "/scripts")}',
             f'rsync -a --delete --exclude /target --exclude "/target-*" --exclude /corpora --exclude /out ./ {quote(staging + "/source/")}',
             f'ln -sfn "$NATIVE_CORPORA_ROOT" {quote(staging + "/source/corpora")}',
             f'cd {quote(staging + "/source")}']
    for name in ('test262-pool-native.py', 'test262.py', 'gate-command.py', 'gate-summary.py'):
        contents = runner if name == 'test262.py' else (SCRIPTS / name).read_bytes()
        encoded = base64.b64encode(contents).decode()
        lines += [f'printf %s {quote(encoded)} | base64 -d > {quote(staging + "/scripts/" + name)}']
    argv = ['python3', staging + '/scripts/test262-pool-native.py', '--source', source,
            '--total', str(total), '--shards', *map(str, shards), '--scripts', staging + '/scripts',
            '--raw', staging + '/raw']
    lines += ['exec ' + shlex.join(argv) + ' --out "$OUT/test262-' + run_id + '/' + source + '"']
    return '\n'.join(lines) + '\n'


def worker(args, index, shards, source, runner, root, run_id):
    lane = f'{args.lane_prefix}-pool-{index}'
    outroot = f'amazing/out/{lane}/test262-{run_id}/{source}'
    endpoint = root / f'pool-{index}.endpoint.manifest.json'
    setup = root / f'pool-{index}.setup.manifest.json'
    state_path = root / f'pool-{index}.native.summary.json'
    cmd = root / f'pool-{index}.command.sh'
    cmd.write_text(command(source, lane, run_id, shards, args.shards, runner))
    with (root / f'pool-{index}.log').open('wb') as output:
        observed = run([SCRIPTS / 'linux-run.sh', lane, '--ref', source,
            '--endpoint-receipt', endpoint, '--compact-root', outroot,
            '--setup-summary', setup, '--command-file', cmd], stdout=output, stderr=subprocess.STDOUT)
    # A failed upload has no job. Do not invent a native UNKNOWN job or poll it.
    if not endpoint.exists():
        return {'pool': index, 'shards': shards, 'observer_exit': observed,
                'state': 'SETUP_FAILED_NO_JOB', 'setup_receipt': str(setup), 'ready': False}
    run([sys.executable, SCRIPTS / 'test262-job-state.py', '--receipt', endpoint,
         '--lane', lane, '--source', source, '--observer-exit', observed, '--seconds', '3600',
         '--output', state_path, '--questions', root / f'pool-{index}.questions.md'])
    state = json.loads(state_path.read_text())
    result = {'pool': index, 'shards': shards, 'observer_exit': observed,
              'state': state.get('state'), 'native_receipt': str(state_path),
              'actual_exit': state.get('actual_exit'), 'runner_exit': state.get('runner_exit'),
              'endpoint': str(endpoint), 'ready': False}
    if not state.get('compact_fetch_allowed') or state.get('state') != 'TERMINAL':
        return result
    pool_dir = root / f'pool-{index}'
    pool_dir.mkdir()
    expected = [outroot + '/pool.summary.json', outroot + '/build.phase.manifest.json']
    if state.get('actual_exit') == 0 and state.get('runner_exit') == 0:
        for k in shards:
            expected += [outroot + f'/shard-{k}.compact.json',
                         outroot + f'/shard-{k}-runner.phase.manifest.json',
                         outroot + f'/shard-{k}-packaging.phase.manifest.json']
    fetched = run(['bash', SCRIPTS / 'lane-fetch.sh', lane, pool_dir,
                  '--endpoint-receipt', endpoint, '--source', source, *expected])
    result['compact_fetch_exit'] = fetched
    result['ready'] = fetched == 0 and state.get('actual_exit') == 0 and state.get('runner_exit') == 0
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('ref')
    parser.add_argument('tag')
    parser.add_argument('--shards', type=int, default=16)
    parser.add_argument('--boxes', type=int, required=True)
    parser.add_argument('--baseline')
    parser.add_argument('--lane-prefix', default='t262-shard')
    parser.add_argument('--runner-ref')
    args = parser.parse_args()
    try:
        groups = native.partition(args.shards, args.boxes)
    except ValueError as error:
        parser.error(str(error))
    if not re.fullmatch('[a-zA-Z0-9_-]+', args.tag) or not re.fullmatch('[a-z0-9-]+', args.lane_prefix):
        parser.error('invalid tag or lane prefix')
    repo = SCRIPTS.parent
    common = Path(subprocess.check_output(['git', '-C', str(repo), 'rev-parse', '--path-format=absolute', '--git-common-dir'], text=True).strip()).parent
    source = subprocess.check_output(['git', '-C', str(common), 'rev-parse', args.ref + '^{commit}'], text=True).strip()
    runner = subprocess.check_output(['git', '-C', str(common), 'show', args.runner_ref + ':scripts/test262.py']) if args.runner_ref else (SCRIPTS / 'test262.py').read_bytes()
    run_id = args.tag + '-' + time.strftime('%Y%m%dT%H%M%S', time.gmtime()) + '-' + str(time.time_ns())
    root = common / '.agents/test262/runs' / run_id
    root.mkdir(parents=True)
    plan = {'schema': 1, 'backend': 'pooled', 'source_commit': source,
            'groups': groups, 'shards': args.shards, 'boxes': args.boxes,
            'jobs_per_shard': 4, 'timeout_seconds': 60,
            'runner_sha256': hashlib.sha256(runner).hexdigest(), 'started_ns': time.time_ns()}
    native.write(root / 'coordinator.plan.summary.json', plan)
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.boxes) as pool:
        futures = [pool.submit(worker, args, i, group, source, runner, root, run_id) for i, group in enumerate(groups)]
        results = []
        for i, future in enumerate(futures):
            try:
                results.append(future.result())
            except Exception as error:
                results.append({'pool': i, 'shards': groups[i], 'ready': False,
                                'error': type(error).__name__ + ': ' + str(error)})
    native.write(root / 'coordinator.result.summary.json', dict(plan, pools=results, finished_ns=time.time_ns()))
    if not all(row['ready'] for row in results):
        print('test262 pooled: incomplete actual jobs; no merge; receipts ' + str(root), file=sys.stderr)
        return 3
    shards = [root / f'pool-{i}' / f'shard-{k}.compact.json' for i, group in enumerate(groups) for k in group]
    if sum(path.stat().st_size for path in shards) > 2000000:
        return 3
    output = common / '.agents/test262' / f't262-{source[:8]}-{args.tag}.json'
    argv = [sys.executable, SCRIPTS / 'test262-merge.py', output, *shards]
    if args.baseline:
        argv += ['--baseline', args.baseline]
    return run(argv)


if __name__ == '__main__':
    raise SystemExit(main())
