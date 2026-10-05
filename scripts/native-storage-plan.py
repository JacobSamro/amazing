#!/usr/bin/env python3
"""Exact opt-in linux-run storage plan admission, before Cargo bootstrap."""
import argparse
import importlib.util
import json
import os
import pathlib
import subprocess
import sys

spec = importlib.util.spec_from_file_location('retention', pathlib.Path(__file__).with_name('native-storage-retention.py'))
retention = importlib.util.module_from_spec(spec)
spec.loader.exec_module(retention)
FIELDS = {'schema', 'ticket', 'source', 'phase', 'job', 'scratch_parent', 'output_root',
          'retained_root', 'build_budget_bytes', 'retention_budget_bytes', 'bootstrap_budget_bytes',
          'reserve_bytes', 'compact_root', 'summary_path', 'baseline_manifest', 'baseline_source', 'retention_method', 'elf_policy', 'retire_after_retention'}
REQUIRED = FIELDS - {'baseline_manifest', 'baseline_source', 'summary_path', 'compact_root', 'retire_after_retention'}


def validate(path, native=True):
    data = json.loads(pathlib.Path(path).read_text())
    if not isinstance(data, dict) or set(data) - FIELDS or REQUIRED - set(data) or data['schema'] != 1:
        raise retention.StorageError('storage plan fields/schema must be explicit')
    for key in ('ticket', 'job'):
        retention.identifier(data[key])
    retention.source_id(data['source'])
    if os.environ.get('COMMIT') and os.environ['COMMIT'] != data['source']:
        raise retention.StorageError('storage plan differs from exact native COMMIT')
    if data['phase'] not in ('before', 'after') or data['retention_method'] not in ('copy', 'hardlink', 'hardlink-or-copy') or data['elf_policy'] not in ('all', 'runnable'):
        raise retention.StorageError('invalid storage plan policy/phase')
    for key in ('build_budget_bytes', 'retention_budget_bytes', 'bootstrap_budget_bytes', 'reserve_bytes'):
        if type(data[key]) is not int or data[key] < (0 if key == 'reserve_bytes' else 1):
            raise retention.StorageError('storage plan requires explicit positive byte budgets')
    if 'retire_after_retention' in data and type(data['retire_after_retention']) is not bool:
        raise retention.StorageError('retire_after_retention must be an explicit boolean')
    if 'retire_after_retention' in data and data['retention_method'] != 'hardlink-or-copy':
        raise retention.StorageError('retire_after_retention is only valid for the explicit hardlink-or-copy policy')
    def operand(value, scratch=False):
        if native:
            return retention.canonical(value, native=scratch)
        # Mac /tmp and /home aliases cannot authenticate Linux paths. Before
        # allocation validate only normalized absolute POSIX shape; native check
        # then verifies actual symlink/device identity on the receiving host.
        path = pathlib.PurePosixPath(value)
        if not path.is_absolute() or str(path) != value or '..' in path.parts:
            raise retention.StorageError('storage plan requires normalized absolute Linux paths')
        if scratch and (path == pathlib.PurePosixPath('/tmp') or pathlib.PurePosixPath('/tmp') not in path.parents):
            raise retention.StorageError('scratch parent must be beneath native /tmp')
        return path
    scratch = operand(data['scratch_parent'], scratch=True)
    output = operand(data['output_root'])
    retained = operand(data['retained_root'])
    if output not in retained.parents or scratch == output or scratch in output.parents:
        raise retention.StorageError('storage plan raw/retained paths overlap scratch or escape owner')
    if data.get('summary_path'):
        summary = operand(data['summary_path'])
        compact = operand(data.get('compact_root', data['output_root']))
        if compact not in summary.parents or scratch == compact or scratch in compact.parents:
            raise retention.StorageError('storage summary must remain beneath explicit compact owner outside scratch')
    elif data.get('compact_root'):
        raise retention.StorageError('compact owner requires an explicit summary path')
    if data['phase'] == 'before' and ('baseline_manifest' in data or 'baseline_source' in data):
        raise retention.StorageError('before cannot consume a baseline')
    if data['phase'] == 'after':
        if not data.get('baseline_manifest') or not data.get('baseline_source'):
            raise retention.StorageError('after requires exact baseline manifest/source')
        retention.source_id(data['baseline_source'])
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('validate', 'check', 'run'))
    parser.add_argument('--plan', required=True)
    parser.add_argument('--command-file')
    args = parser.parse_args()
    try:
        data = validate(args.plan, native=args.mode != 'validate')
        if args.mode == 'validate':
            print(json.dumps({'schema': 1, 'source': data['source'], 'phase': data['phase'], 'job': data['job']}))
            return 0
        if data['phase'] == 'after':
            retention.validated_manifest(data['baseline_manifest'], data['baseline_source'])
            profile = {k: os.environ.get(k, v) for k, v in retention.PROFILE.items()}
            if json.loads(pathlib.Path(data['baseline_manifest']).read_text())['profile'] != profile:
                raise retention.StorageError('before/after Cargo profile must match')
        if data['retention_method'] == 'hardlink' and retention.filesystem(pathlib.Path(data['scratch_parent']))['device'] != retention.filesystem(pathlib.Path(data['retained_root']))['device']:
            raise retention.StorageError('hardlink retention requires the same actual filesystem')
        if args.mode == 'check':
            session = pathlib.Path(data['scratch_parent']) / data['ticket'] / data['source'] / data['phase'] / data['job']
            if session.exists():
                raise retention.StorageError('fresh source/phase/job target identity already exists')
            # Bootstrap download/extraction and Cargo target overlap. Retained
            # baseline blobs already occupy statvfs used bytes; never double count.
            capacity = retention.capacity(pathlib.Path(data['scratch_parent']), pathlib.Path(data['output_root']),
                data['build_budget_bytes'] + data['bootstrap_budget_bytes'], data['retention_budget_bytes'], data['reserve_bytes'])
            print(json.dumps({'schema': 1, 'utc': retention.utc(), 'source': data['source'], 'phase': data['phase'],
                              'job': data['job'], 'capacity': capacity, 'bootstrap_budget_bytes': data['bootstrap_budget_bytes']}))
            return 0 if capacity['admitted'] else 3
        if not args.command_file:
            raise retention.StorageError('run requires exact command file')
        command = [sys.executable, str(pathlib.Path(__file__).with_name('native-storage-retention.py')), 'run']
        for key, value in data.items():
            if key == 'retire_after_retention':
                if value: command += ['--retire-after-retention']
                continue
            if key not in ('schema', 'bootstrap_budget_bytes', 'summary_path', 'compact_root'):
                command += ['--' + key.replace('_', '-'), str(value)]
        command += ['--', 'bash', args.command_file]
        code = subprocess.run(command, check=False).returncode
        if data.get('summary_path'):
            session = pathlib.Path(data['scratch_parent']) / data['ticket'] / data['source'] / data['phase'] / data['job'] / 'session.json'
            value = {'schema':1, 'source':data['source'], 'phase':data['phase'], 'job':data['job'], 'actual_storage_exit':code}
            if session.is_file():
                raw = json.loads(session.read_text())
                value.update({key:raw[key] for key in ('state','command_exit','storage_exit','storage_error','hardlink_capability','selected_retention_method','profile','capacity','baseline','artifact_manifest','retained_files','retained_elf_files','retained_logical_bytes','copied_unique_bytes','retention_capacity_required_bytes','recoverable_files','recoverable_bytes','target_retired','retire_after_retention','free_after') if key in raw})
                value['session_raw'] = {'path':str(session),'sha256':retention.digest(session),'bytes':session.stat().st_size}
            retention.atomic_json(pathlib.Path(data['summary_path']),value)
        return code
    except (retention.StorageError, ValueError, KeyError, TypeError, OSError) as error:
        print(json.dumps({'schema': 1, 'storage_exit': 3, 'error': str(error)}))
        return 3


if __name__ == '__main__':
    sys.exit(main())
