#!/usr/bin/env python3
"""Prepare and run an exact-ref workspace gate through the public Boat wrapper."""
import argparse
import base64
import hashlib
import importlib.util
import json
import pathlib
import re
import shlex
import subprocess
import sys
import uuid

ROOT = pathlib.Path(__file__).resolve().parent.parent
HELPERS = ('linux-workspace-native.py', 'gate-command.py', 'gate-summary.py')
PROFILE = {'CARGO_PROFILE_DEV_DEBUG': 'line-tables-only',
           'CARGO_PROFILE_TEST_DEBUG': 'line-tables-only', 'CARGO_INCREMENTAL': '0'}


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), ROOT / 'scripts' / (name + '.py'))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def sha(data):
    return hashlib.sha256(data).hexdigest()


def prepare(source, lane, destination, job=None):
    if not re.fullmatch('[0-9a-f]{40}', source):
        raise ValueError('an exact commit is required')
    if not re.fullmatch('[A-Za-z0-9_-]+', lane):
        raise ValueError('invalid lane')
    job = job or 'workspace-' + uuid.uuid4().hex
    if not re.fullmatch('[A-Za-z0-9_-]+', job):
        raise ValueError('invalid job')
    root = f'amazing/out/{lane}/{job}/compact'
    load('completed-job').valid_output_root(lane, root)
    raw = f'/tmp/amazing-workspace-raw/{lane}/{job}'
    hashes = {n: sha((ROOT / 'scripts' / n).read_bytes()) for n in HELPERS}
    hashes.update({n: sha((ROOT / 'scripts' / n).read_bytes()) for n in
                   ('linux-run.sh', 'native-storage-plan.py', 'native-storage-retention.py', 'completed-job.py', 'lane-fetch.sh')})
    config = {'schema': 1, 'source': source, 'lane': lane, 'job': job,
              'corpus_role': 'standalone workspace candidate', 'profile': PROFILE,
              'raw_root': raw, 'compact_root': root,
              'compact_absolute': '/home/user/' + root, 'helper_sha256': hashes}
    plan = {'schema': 1, 'ticket': lane, 'source': source, 'phase': 'before', 'job': job,
            'scratch_parent': '/tmp/amazing-native-storage', 'output_root': raw,
            'retained_root': raw + '/retained', 'build_budget_bytes': 24 * 1024**3,
            'retention_budget_bytes': 128 * 1024**2, 'bootstrap_budget_bytes': 2 * 1024**3,
            'reserve_bytes': 2 * 1024**3, 'retention_method': 'hardlink-or-copy',
            'elf_policy': 'all', 'retire_after_retention': False,
            'compact_root': config['compact_absolute'],
            'summary_path': config['compact_absolute'] + '/storage.summary.json'}
    destination = pathlib.Path(destination)
    destination.mkdir(parents=True, exist_ok=False)
    def write(name, value):
        path = destination / name
        path.write_text(json.dumps(value, sort_keys=True, indent=2) + '\n')
        return path
    storage = write('storage-plan.json', plan)
    load('native-storage-plan').validate(storage, native=False)
    write('config.json', config)
    # Embed reviewed helper bytes beside the frozen engine source without
    # requiring the old engine commit to contain this newly qualified launcher.
    payload = {n: base64.b64encode((ROOT / 'scripts' / n).read_bytes()).decode() for n in HELPERS}
    payload['config.json'] = base64.b64encode(json.dumps(config).encode()).decode()
    script = 'set -euo pipefail\nunset AMAZING_JS_VM AMAZING_JS_VM_COMPILE_LOG AMAZING_JS_VM_REJECT_CONTEXT\n'
    script += "python3 - <<'WORKSPACE_PAYLOAD'\nimport base64,pathlib\n"
    script += f"root=pathlib.Path({(raw + '/helpers')!r})\nroot.mkdir(parents=True,exist_ok=False)\n"
    script += f"payload={payload!r}\nfor name,data in payload.items(): (root/name).write_bytes(base64.b64decode(data))\nWORKSPACE_PAYLOAD\n"
    script += 'exec python3 ' + shlex.quote(raw + '/helpers/linux-workspace-native.py') + ' ' + shlex.quote(raw + '/helpers/config.json') + '\n'
    command = destination / 'command.sh'
    command.write_text(script)
    endpoint = destination / 'endpoint.manifest.json'
    argv = ['bash', str(ROOT / 'scripts/linux-run.sh'), lane, '--host', 'boat', '--ref', source,
            '--setup-summary', str(destination / 'setup.manifest.json'),
            '--endpoint-receipt', str(endpoint), '--compact-root', root,
            '--storage-plan', str(storage), '--command-file', str(command)]
    request = {'schema': 1, 'source': source, 'lane': lane, 'job': job,
               'machine': 'default 4 vCPU / 8 GB', 'finite_run_ttl_seconds': 7200,
               'ensure_may_renew_finite_ttl': True, 'physical_storage_phase': 'before',
               'corpus_role': config['corpus_role'], 'admission_bytes': 28.125 * 1024**3,
               'command_sha256': sha(command.read_bytes()), 'storage_sha256': sha(storage.read_bytes()),
               'config_sha256': sha((destination / 'config.json').read_bytes()),
               'helper_sha256': hashes, 'argv': argv, 'compact_root': root,
               'endpoint_receipt': str(endpoint), 'raw_root': raw,
               'fetch_paths': [root + '/' + n for n in
                   ('workspace.manifest.json', 'nextest.phase.manifest.json', 'doctests.phase.manifest.json', 'storage.summary.json')]}
    write('request.summary.json', request)
    return request


def run(request, destination):
    code = subprocess.run(request['argv'], check=False).returncode
    endpoint = pathlib.Path(request['endpoint_receipt'])
    # A setup failure has no endpoint; do not fetch/recover nonexistent output.
    fetched = None
    if endpoint.is_file():
        fetched = subprocess.run(['bash', str(ROOT / 'scripts/lane-fetch.sh'), request['lane'],
            str(pathlib.Path(destination) / 'compact'), '--endpoint-receipt', str(endpoint),
            '--source', request['source'], *request['fetch_paths']], check=False).returncode
    observation = pathlib.Path(str(endpoint) + '.observation.summary.json')
    value = {'schema': 1, 'source': request['source'], 'lane': request['lane'],
             'wrapper_exit': code, 'compact_fetch_exit': fetched,
             'native_observation': json.loads(observation.read_text()) if observation.is_file() else None}
    (pathlib.Path(destination) / 'result.summary.json').write_text(json.dumps(value, sort_keys=True) + '\n')
    return code if code else (0 if fetched == 0 else 3)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('ref', nargs='?', default='HEAD')
    parser.add_argument('--lane', default='integrator-primary')
    parser.add_argument('--prepare-only', action='store_true')
    parser.add_argument('--destination')
    args = parser.parse_args()
    common = pathlib.Path(subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', '--path-format=absolute', '--git-common-dir'], text=True).strip()).parent
    # Preserve linux-gate.sh's original [ref] / omitted-HEAD resolution from
    # the primary checkout even when qualified helper bytes live in a worktree.
    source = subprocess.check_output(['git', '-C', str(common), 'rev-parse', args.ref + '^{commit}'], text=True).strip()
    subprocess.run([sys.executable, str(ROOT / 'scripts/merge-preflight.py'), '--base',
                    __import__('os').environ.get('MERGE_BASE', 'origin/dev'), '--ref', source], check=True)
    destination = pathlib.Path(args.destination) if args.destination else common / '.agents/integrator-primary/gates' / ('workspace-' + uuid.uuid4().hex)
    request = prepare(source, args.lane, destination)
    print(json.dumps({'request': str(destination / 'request.summary.json'), 'source': source, 'lane': args.lane}), flush=True)
    return 0 if args.prepare_only else run(request, destination)


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(json.dumps({'wrapper_exit': 2, 'error': str(error)}), file=sys.stderr)
        raise SystemExit(2)
