#!/usr/bin/env python3
"""Opt-in native Cargo target capacity and terminal artifact retention.

Run inside linux-run's native command, after bootstrap. This owns no Boat,
SSH, launch observer or job-status policy. Budgets are explicit caller inputs.
All ELF and generated-source bytes survive in verified executable blobs;
only this helper's terminal fresh Cargo target is retired. OUT stays intact.
"""
import argparse
import datetime
import errno
import fcntl
import hashlib
import json
import os
import pathlib
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile

SOURCE_SUFFIXES = {'.rs', '.c', '.cc', '.cpp', '.h', '.hpp', '.js', '.py',
                   '.sh', '.toml', '.lock', '.log', '.wasm', '.wat'}
PROFILE = {'CARGO_PROFILE_DEV_DEBUG': 'line-tables-only',
           'CARGO_PROFILE_TEST_DEBUG': 'line-tables-only', 'CARGO_INCREMENTAL': '0'}


class StorageError(Exception):
    pass


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False) as stream:
        temporary = pathlib.Path(stream.name)
        json.dump(value, stream, separators=(',', ':'), sort_keys=True)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


def identifier(value):
    if not re.fullmatch(r'[a-z0-9][a-z0-9._-]{0,127}', value):
        raise StorageError('invalid ticket/job identity')
    return value


def source_id(value):
    if not re.fullmatch(r'[0-9a-f]{40}', value):
        raise StorageError('source must be an exact full commit SHA')
    return value


def canonical(path, native=False):
    path = pathlib.Path(path)
    if not path.is_absolute() or path.resolve() != path:
        raise StorageError('storage paths must be absolute without symlink components')
    if native and (path == pathlib.Path('/tmp') or pathlib.Path('/tmp') not in path.parents):
        raise StorageError('scratch parent must be a directory beneath native /tmp')
    return path


def existing_parent(path):
    while not path.exists():
        path = path.parent
    return path


def filesystem(path):
    parent = existing_parent(path)
    usage = os.statvfs(parent)
    return {'path': str(path), 'device': parent.stat().st_dev,
            'available_bytes': usage.f_bavail * usage.f_frsize,
            'total_bytes': usage.f_blocks * usage.f_frsize}


def capacity(scratch, output, build_bytes, retention_bytes, reserve_bytes):
    rows = [dict(filesystem(scratch), required_bytes=build_bytes),
            dict(filesystem(output), required_bytes=retention_bytes)]
    groups = {}
    for row in rows:
        group = groups.setdefault(row['device'], {'available_bytes': row['available_bytes'],
                                                  'required_bytes': reserve_bytes})
        group['available_bytes'] = min(group['available_bytes'], row['available_bytes'])
        group['required_bytes'] += row['required_bytes']
    okay = all(row['available_bytes'] >= row['required_bytes'] for row in groups.values())
    return {'filesystems': rows, 'shared_device_requirements': list(groups.values()), 'admitted': okay}


def validated_manifest(path, expected_source=None):
    path = canonical(path)
    data = json.loads(path.read_text())
    if data.get('schema') != 1 or data.get('command_exit') != 0 or data.get('phase') != 'before':
        raise StorageError('baseline requires a completed successful before artifact manifest')
    if data.get('source') != expected_source:
        raise StorageError('baseline source does not match its recorded commit')
    if not data.get('artifacts') or not any(a['kind'] == 'elf' for a in data['artifacts']):
        raise StorageError('baseline has no frozen measured executable')
    for artifact in data['artifacts']:
        blob = canonical(artifact['blob'])
        if not blob.is_file() or blob.stat().st_size != artifact['bytes'] or digest(blob) != artifact['sha256']:
            raise StorageError('baseline retained executable/source bytes changed')
    return {'path': str(path), 'sha256': digest(path), 'source': data['source'],
            'artifact_count': len(data['artifacts'])}


def elf_kind(path):
    with path.open('rb') as stream:
        header = stream.read(64)
    if not header.startswith(b'\x7fELF'):
        return None
    # Preserve malformed/unknown ELF, never classify by filename or executable bit.
    if len(header) < 20 or header[4] not in (1, 2) or header[5] not in (1, 2):
        return 'unknown'
    required = 52 if header[4] == 1 else 64
    if len(header) < required or header[6] != 1:
        return 'unknown'
    kind = struct.unpack_from('<H' if header[5] == 1 else '>H', header, 16)[0]
    return {1: 'relocatable', 2: 'executable', 3: 'shared-object'}.get(kind, 'unknown')


def artifacts(target, elf_policy='all'):
    retained, recoverable = [], []
    for directory, dirs, files in os.walk(target, followlinks=False):
        for name in dirs + files:
            if (pathlib.Path(directory) / name).is_symlink():
                raise StorageError('target contains a symlink; retain it without automatic retirement')
        for name in files:
            path = pathlib.Path(directory) / name
            info = path.stat()
            if not stat.S_ISREG(info.st_mode):
                raise StorageError('target contains a non-regular artifact')
            elf = elf_kind(path)
            row = {'relative_path': str(path.relative_to(target)), 'bytes': info.st_size,
                   'mode': stat.S_IMODE(info.st_mode)}
            if (elf and not (elf == 'relocatable' and elf_policy == 'runnable')) or path.suffix in SOURCE_SUFFIXES:
                retained.append(dict(row, sha256=digest(path), kind='elf' if elf else 'generated-source', elf_type=elf))
            else:
                recoverable.append(row)
    return retained, recoverable


def probe_hardlink(source, destination):
    """Exercise the syscall on the actual artifact and destination namespace.

    Device IDs are diagnostic only: overlay/snapshot filesystems may report
    equal IDs while refusing a link across their backing layers. Only this
    fresh probe name is removed; the source and previous proofs are untouched.
    """
    destination.mkdir(parents=True, exist_ok=True)
    result = {'source': str(source), 'destination_directory': str(destination),
              'source_device': source.stat().st_dev,
              'destination_device': destination.stat().st_dev}
    with tempfile.TemporaryDirectory(prefix='.hardlink-probe-', dir=destination) as temporary:
        link = pathlib.Path(temporary) / 'artifact'
        try:
            os.link(source, link)
            one, two = source.stat(), link.stat()
            if (one.st_dev, one.st_ino, one.st_size) != (two.st_dev, two.st_ino, two.st_size):
                raise StorageError('hardlink syscall did not preserve artifact identity')
            result.update(supported=True, errno=None)
        except OSError as error:
            if error.errno not in (errno.EXDEV, errno.EOPNOTSUPP, errno.ENOSYS,
                                  errno.EPERM, errno.EMLINK):
                raise
            result.update(supported=False, errno=error.errno, error=str(error))
    return result


def freeze_artifacts(session, data, output, reserve):
    target = session / 'target'
    retained, recoverable = artifacts(target, data.get('elf_policy', 'all'))
    blobs = output / 'blobs'
    missing = {}
    for artifact in retained:
        destination = blobs / artifact['sha256']
        if destination.exists():
            if destination.is_symlink() or digest(destination) != artifact['sha256']:
                raise StorageError('existing retained blob failed integrity verification')
        else:
            missing[artifact['sha256']] = artifact['bytes']
    requested = data.get('retention_method', 'copy')
    hardlink = requested in ('hardlink', 'hardlink-or-copy')
    if hardlink and missing:
        # Probe every actual artifact before deciding a mode or copying bytes.
        # A later artifact cannot silently trigger an unbudgeted partial copy.
        probes = []
        for artifact in retained:
            if artifact['sha256'] in missing:
                probes.append(probe_hardlink(target / artifact['relative_path'], blobs))
        failures = [row for row in probes if not row['supported']]
        probe_path = session / 'hardlink.probes.json'
        atomic_json(probe_path, probes)
        data['hardlink_capability'] = {'probed_artifacts': len(probes),
                                      'supported': not failures, 'failed_artifacts': len(failures),
                                      'first_failures': failures[:8],
                                      'raw': {'path': str(probe_path), 'bytes': probe_path.stat().st_size,
                                              'sha256': digest(probe_path)}}
        atomic_json(session / 'session.json', data)
        if failures:
            if requested != 'hardlink-or-copy':
                raise StorageError('actual hardlink capability unavailable; explicit budgeted copy policy required')
            hardlink = False
    data['selected_retention_method'] = 'hardlink' if hardlink else 'copy'
    payload_bytes = 0 if hardlink else sum(missing.values())
    requirement = payload_bytes + (len(missing) * 4096 + len(retained) * 1024 + 8192 if hardlink else 0)
    if requirement > data.get('retention_budget_bytes', requirement):
        raise StorageError('artifact bytes exceed explicit retention budget; target remains intact')
    if filesystem(output)['available_bytes'] < requirement + reserve:
        raise StorageError('insufficient artifact-retention capacity; target remains intact')
    blobs.mkdir(parents=True, exist_ok=True)
    for artifact in retained:
        original = target / artifact['relative_path']
        destination = blobs / artifact['sha256']
        if not destination.exists():
            if hardlink:
                # Only our leased, terminal, freshly-created target is eligible.
                # Do not chmod a hardlink: its original mode and inode are custody.
                os.link(original, destination)
                artifact['retention_method'] = 'hardlink'
            else:
                with tempfile.NamedTemporaryFile(dir=blobs, delete=False) as stream:
                    temporary = pathlib.Path(stream.name)
                    with original.open('rb') as source:
                        shutil.copyfileobj(source, stream, 1024 * 1024)
                    stream.flush()
                    os.fsync(stream.fileno())
                if digest(temporary) != artifact['sha256']:
                    raise StorageError('artifact copy did not preserve exact bytes')
                temporary.chmod(artifact['mode'] | (0o500 if artifact['kind'] == 'elf' else 0))
                os.replace(temporary, destination)
                artifact['retention_method'] = 'copy'
        else:
            artifact['retention_method'] = 'deduplicated'
        if digest(original) != artifact['sha256'] or digest(destination) != artifact['sha256']:
            raise StorageError('artifact changed during retention; target remains intact')
        artifact['blob'] = str(destination)
    manifest = output / 'manifests' / (data['job'] + '.json')
    if manifest.exists():
        raise StorageError('artifact manifest job identity already exists')
    value = {key: data[key] for key in ('schema', 'ticket', 'source', 'phase', 'job', 'command_exit', 'profile')}
    value.update(artifacts=retained, target=str(target), completed_at=utc())
    atomic_json(manifest, value)
    # Re-read the published manifest and every retained blob before cache deletion.
    if json.loads(manifest.read_text()) != value:
        raise StorageError('artifact manifest publication changed')
    for artifact in retained:
        if digest(pathlib.Path(artifact['blob'])) != artifact['sha256']:
            raise StorageError('retained blob changed before cache retirement')
    data.update(artifact_manifest={'path': str(manifest), 'sha256': digest(manifest),
                                 'bytes': manifest.stat().st_size},
                retained_files=len(retained), retained_elf_files=sum(a['kind'] == 'elf' for a in retained),
                retained_logical_bytes=sum(a['bytes'] for a in retained),
                copied_unique_bytes=payload_bytes, retention_capacity_required_bytes=requirement,
                recoverable_files=len(recoverable), recoverable_bytes=sum(a['bytes'] for a in recoverable),
                recoverable_manifest_sha256=hashlib.sha256(json.dumps(recoverable, sort_keys=True).encode()).hexdigest(),
                retention_complete=True)
    atomic_json(session / 'session.json', data)


def retire_locked(session, data):
    if data.get('state') != 'TERMINAL' or not data.get('retention_complete'):
        raise StorageError('RUNNING/UNKNOWN or unretained target cannot be retired')
    manifest = pathlib.Path(data['artifact_manifest']['path'])
    if digest(manifest) != data['artifact_manifest']['sha256']:
        raise StorageError('artifact manifest changed before retirement')
    for artifact in json.loads(manifest.read_text())['artifacts']:
        if digest(pathlib.Path(artifact['blob'])) != artifact['sha256']:
            raise StorageError('retained executable/source changed before retirement')
    target = session / 'target'
    if target.is_symlink() or target.resolve() != target:
        raise StorageError('target identity changed')
    if target.exists():
        shutil.rmtree(target)
    data['target_retired'] = True
    data['retired_at'] = utc()
    atomic_json(session / 'session.json', data)


def run(args):
    ticket, job, source = identifier(args.ticket), identifier(args.job), source_id(args.source)
    if os.environ.get('COMMIT') and os.environ['COMMIT'] != source:
        raise StorageError('source does not match the native launcher COMMIT')
    scratch = canonical(args.scratch_parent, native=True)
    output, retained = canonical(args.output_root), canonical(args.retained_root)
    if output not in retained.parents:
        raise StorageError('retained executables must remain beneath the explicit output root')
    if scratch == output or scratch in output.parents:
        raise StorageError('raw output must remain outside managed scratch')
    baseline = None
    if args.phase == 'after':
        if not args.baseline_manifest or not args.baseline_source:
            raise StorageError('after requires explicit frozen baseline manifest/source')
        baseline = validated_manifest(args.baseline_manifest, source_id(args.baseline_source))
    elif args.baseline_manifest or args.baseline_source:
        raise StorageError('before cannot consume another baseline')
    profile = {key: os.environ.get(key, value) for key, value in PROFILE.items()}
    if baseline and json.loads(pathlib.Path(baseline['path']).read_text())['profile'] != profile:
        raise StorageError('before/after Cargo profile must match')
    if args.retention_method == 'hardlink' and filesystem(scratch)['device'] != filesystem(retained)['device']:
        raise StorageError('hardlink retention requires the same actual filesystem')
    admit = capacity(scratch, output, args.build_budget_bytes,
                     args.retention_budget_bytes, args.reserve_bytes)
    receipt = {'schema': 1, 'utc': utc(), 'ticket': ticket, 'job': job, 'source': source,
               'phase': args.phase, 'capacity': admit, 'profile': profile, 'baseline': baseline}
    if not admit['admitted']:
        print(json.dumps(dict(receipt, error='insufficient capacity; no command or target started')))
        return 3
    session = scratch / ticket / source / args.phase / job
    if session.exists():
        raise StorageError('fresh source/phase/job target identity already exists')
    session.mkdir(parents=True)
    with (session / 'lease.lock').open('x') as lease:
        fcntl.flock(lease, fcntl.LOCK_EX)
        target = session / 'target'
        target.mkdir()
        data = dict(receipt, state='RUNNING', owner_pid=os.getpid(), session=str(session),
                    scratch_parent=str(scratch), target=str(target), command=args.command,
                    retention_complete=False, retention_method=args.retention_method,
                    elf_policy=args.elf_policy, retention_budget_bytes=args.retention_budget_bytes,
                    retire_after_retention=(args.retention_method != 'hardlink-or-copy' or args.retire_after_retention))
        atomic_json(session / 'session.json', data)
        environment = dict(os.environ, **profile, CARGO_TARGET_DIR=str(target))
        # The existing native launcher owns timeout/supervision. Keep the lease
        # until its real command returns; an interrupted owner stays RUNNING.
        completed = subprocess.run(args.command, env=environment, check=False)
        code = completed.returncode if completed.returncode >= 0 else 128 - completed.returncode
        data.update(state='TERMINAL', command_exit=code, command_returncode=completed.returncode,
                    completed_at=utc())
        atomic_json(session / 'session.json', data)
        try:
            freeze_artifacts(session, data, retained, args.reserve_bytes)
            if data['retire_after_retention']:
                retire_locked(session, data)
            else:
                data['target_retired'] = False
            data['storage_exit'] = 0
        except (StorageError, OSError) as error:
            data.update(storage_exit=3, storage_error=str(error))
        data['free_after'] = [filesystem(scratch), filesystem(output)]
        atomic_json(session / 'session.json', data)
        print(json.dumps(data, separators=(',', ':')))
        return code if code else data['storage_exit']


def publish_raw(args):
    """Publish one terminal private raw directory; never delete a copy source."""
    source = canonical(args.directory, native=True)
    destination = canonical(args.destination)
    if not source.is_dir() or destination.exists() or destination.is_symlink():
        raise StorageError('raw publication needs a real source and fresh destination')
    if source == destination or source in destination.parents or destination in source.parents:
        raise StorageError('raw source/destination overlap')
    rows = []
    directories = []
    for directory, dirs, files in os.walk(source, followlinks=False):
        for name in dirs + files:
            if (pathlib.Path(directory) / name).is_symlink():
                raise StorageError('raw publication refuses symlink operands')
        directories += [str((pathlib.Path(directory) / name).relative_to(source)) for name in dirs]
        for name in files:
            path = pathlib.Path(directory) / name
            if not stat.S_ISREG(path.stat().st_mode):
                raise StorageError('raw publication refuses nonregular files')
            rows.append({'path': str(path.relative_to(source)), 'bytes': path.stat().st_size,
                         'sha256': digest(path), 'mode': stat.S_IMODE(path.stat().st_mode)})
    destination.parent.mkdir(parents=True, exist_ok=True)
    receipt = {'schema': 1, 'source_directory': str(source), 'destination': str(destination),
               'raw_files': len(rows), 'raw_bytes': sum(row['bytes'] for row in rows),
               'inventory_sha256': hashlib.sha256(json.dumps(rows, sort_keys=True).encode()).hexdigest(),
               'source_device': source.stat().st_dev,
               'destination_device': destination.parent.stat().st_dev,
               'copy_budget_bytes': args.copy_budget_bytes, 'complete': False}
    try:
        # The actual rename is the capability check. Equal st_dev is not proof.
        os.rename(source, destination)
        receipt.update(method='rename', source_retained=False, rename_errno=None)
    except OSError as error:
        if error.errno != errno.EXDEV:
            raise
        receipt['rename_errno'] = error.errno
        required = receipt['raw_bytes'] + 4096 * (len(rows) + len(directories)) + 8192
        receipt['required_copy_bytes'] = required
        if required > args.copy_budget_bytes:
            raise StorageError('raw EXDEV requires an explicit sufficient copy budget; source intact')
        if filesystem(destination)['available_bytes'] < required + args.reserve_bytes:
            raise StorageError('raw copy capacity unavailable; source intact')
        destination.mkdir()  # Exclusive owner; never overwrite a prior raw proof.
        for directory in directories:
            (destination / directory).mkdir(parents=True, exist_ok=True)
        for row in rows:
            old, new = source / row['path'], destination / row['path']
            with new.open('xb') as stream, old.open('rb') as original:
                shutil.copyfileobj(original, stream, 1024 * 1024)
                stream.flush()
                os.fsync(stream.fileno())
            new.chmod(row['mode'])
        receipt.update(method='copy', source_retained=True)
    for row in rows:
        new = destination / row['path']
        if new.stat().st_size != row['bytes'] or digest(new) != row['sha256']:
            raise StorageError('raw published bytes changed; retained sources are never removed')
        if receipt['source_retained'] and (source / row['path']).stat().st_size != row['bytes']:
            raise StorageError('raw source changed during publication')
        if receipt['source_retained'] and digest(source / row['path']) != row['sha256']:
            raise StorageError('raw source changed during publication')
    expected = {row['path'] for row in rows}
    for tree in ([source, destination] if receipt['source_retained'] else [destination]):
        actual = set()
        for directory, dirs, files in os.walk(tree, followlinks=False):
            for name in dirs + files:
                if (pathlib.Path(directory) / name).is_symlink():
                    raise StorageError('raw tree changed during publication')
            actual.update(str((pathlib.Path(directory) / name).relative_to(tree)) for name in files)
        if actual != expected:
            raise StorageError('raw inventory changed during publication')
    receipt['complete'] = True
    print(json.dumps(receipt, separators=(',', ':')))
    return 0


def retire(args):
    session = canonical(args.session, native=True)
    with (session / 'lease.lock').open('r+') as lease:
        try:
            fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise StorageError('live lease; target remains intact')
        data = json.loads((session / 'session.json').read_text())
        expected = canonical(data['scratch_parent'], native=True) / identifier(data['ticket']) / source_id(data['source']) / data['phase'] / identifier(data['job'])
        if data.get('phase') not in ('before', 'after') or expected != session or data.get('session') != str(session):
            raise StorageError('session identity changed')
        retire_locked(session, data)
        print(json.dumps(data, separators=(',', ':')))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest='mode', required=True)
    run_parser = modes.add_parser('run')
    for name in ('ticket', 'source', 'phase', 'job', 'output-root', 'retained-root'):
        run_parser.add_argument('--' + name, required=True)
    run_parser.add_argument('--scratch-parent', default='/tmp/amazing-native-storage')
    for name in ('build-budget-bytes', 'retention-budget-bytes'):
        run_parser.add_argument('--' + name, type=int, required=True)
    run_parser.add_argument('--reserve-bytes', type=int, default=64 * 1024 * 1024)
    run_parser.add_argument('--retention-method', choices=('copy', 'hardlink', 'hardlink-or-copy'), default='copy')
    run_parser.add_argument('--retire-after-retention', action='store_true',
                            help='explicitly retire the verified terminal target under hardlink-or-copy')
    run_parser.add_argument('--elf-policy', choices=('all', 'runnable'), default='all')
    run_parser.add_argument('--baseline-manifest')
    run_parser.add_argument('--baseline-source')
    run_parser.add_argument('command', nargs=argparse.REMAINDER)
    retire_parser = modes.add_parser('retire')
    retire_parser.add_argument('--session', required=True)
    check_parser = modes.add_parser('check')
    check_parser.add_argument('--scratch-parent', default='/tmp/amazing-native-storage')
    check_parser.add_argument('--output-root', required=True)
    for name in ('build-budget-bytes', 'retention-budget-bytes'):
        check_parser.add_argument('--' + name, type=int, required=True)
    check_parser.add_argument('--reserve-bytes', type=int, default=64 * 1024 * 1024)
    raw_parser = modes.add_parser('publish-raw')
    raw_parser.add_argument('--directory', required=True)
    raw_parser.add_argument('--destination', required=True)
    raw_parser.add_argument('--copy-budget-bytes', type=int, default=0,
                            help='explicit additional payload capacity; zero rejects EXDEV')
    raw_parser.add_argument('--reserve-bytes', type=int, default=64 * 1024 * 1024)
    args = parser.parse_args()
    if args.mode == 'publish-raw' and (args.copy_budget_bytes < 0 or args.reserve_bytes < 0):
        parser.error('raw copy budgets must be nonnegative')
    if args.mode in ('run', 'check'):
        if min(args.build_budget_bytes, args.retention_budget_bytes) <= 0 or args.reserve_bytes < 0:
            parser.error('positive explicit budgets are required')
    if args.mode == 'run':
        if args.phase not in ('before', 'after'):
            parser.error('phase must be before or after')
        if args.command[:1] == ['--']:
            args.command = args.command[1:]
        if not args.command:
            parser.error('native command is required')
    try:
        if args.mode == 'check':
            result = capacity(canonical(args.scratch_parent, native=True), canonical(args.output_root),
                              args.build_budget_bytes, args.retention_budget_bytes, args.reserve_bytes)
            print(json.dumps(dict(schema=1, utc=utc(), capacity=result), separators=(',', ':')))
            return 0 if result['admitted'] else 3
        if args.mode == 'publish-raw':
            return publish_raw(args)
        return run(args) if args.mode == 'run' else retire(args)
    except (StorageError, OSError, ValueError, KeyError) as error:
        print(json.dumps({'schema': 1, 'utc': utc(), 'storage_exit': 3, 'error': str(error)}))
        return 3


if __name__ == '__main__':
    sys.exit(main())
