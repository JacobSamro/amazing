#!/usr/bin/env python3
"""One finite source upload; native receiver publishes only exact bytes."""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import time
import uuid


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def describe(path, source):
    path = Path(path)
    before = path.stat()
    identity = sha(path)
    after = path.stat()
    if (before.st_size, before.st_mtime_ns, before.st_ino) != (after.st_size, after.st_mtime_ns, after.st_ino):
        raise ValueError('source packet changed during authentication')
    if not after.st_size:
        raise ValueError('empty source packet')
    return {'source_commit': source, 'archive_path': str(path), 'archive_bytes': after.st_size,
            'archive_sha256': identity, 'archive_mtime_ns': after.st_mtime_ns}


def receive(destination, size, expected, source, stream, progress=None):
    destination = Path(destination)
    directory = destination.parent.resolve(strict=True)
    if not directory.is_relative_to(Path('/tmp')) or directory.stat().st_dev != Path('/tmp').stat().st_dev:
        raise ValueError('source receiver requires native /tmp storage')
    if destination.parent != directory or destination.exists() or destination.is_symlink():
        raise ValueError('source receiver requires an unused canonical packet path')
    if size <= 0 or len(expected) != 64 or any(c not in '0123456789abcdef' for c in expected):
        raise ValueError('invalid authenticated source packet')
    free = shutil.disk_usage(directory).free
    if free < size:
        raise OSError(28, 'native source packet capacity unavailable')
    if progress: progress('capacity-admitted', received=0, admitted_free_bytes=free)
    partial = destination.with_name(destination.name + '.' + uuid.uuid4().hex + '.partial')
    identity = hashlib.sha256()
    total = 0
    last_progress = time.monotonic()
    with partial.open('xb') as output:
        if progress: progress('streaming', received=0, partial=str(partial))
        while total < size:
            read = getattr(stream, 'read1', stream.read)
            chunk = read(min(1024 * 1024, size - total))
            if not chunk:
                raise ValueError(f'source packet truncated: received {total} of {size} bytes; partial {partial}')
            if output.write(chunk) != len(chunk):
                raise OSError(5, f'short native source write; partial {partial}')
            identity.update(chunk)
            total += len(chunk)
            if progress and time.monotonic() - last_progress >= 5:
                progress('streaming', received=total, partial=str(partial))
                last_progress = time.monotonic()
        if progress: progress('awaiting-eof', received=total, partial=str(partial))
        if stream.read(1):
            raise ValueError(f'source packet has extra bytes; partial {partial}')
        if progress: progress('syncing', received=total, partial=str(partial))
        output.flush()
        os.fsync(output.fileno())
    if progress: progress('hash-readback', received=total, partial=str(partial))
    actual = sha(partial)
    if identity.hexdigest() != expected or actual != expected or partial.stat().st_size != size:
        raise ValueError(f'source packet SHA256 mismatch; partial {partial}')
    # A fresh destination must not overwrite another run even after streaming.
    if progress: progress('publishing', received=total, partial=str(partial))
    os.link(partial, destination)
    partial.unlink()
    fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)
    return {'source_commit': source, 'archive_bytes': total, 'archive_sha256': actual,
            'native_packet': str(destination), 'native_device': directory.stat().st_dev,
            'admitted_free_bytes': free, 'published': True}


@contextlib.contextmanager
def receiver_deadline(seconds):
    """Receiver-local cutoff covers read, EOF, sync, readback and publication.

    The production SSH command also uses native timeout as an outer kill bound.
    No deadline failure is a qualified publication, even if bytes arrived.
    """
    if not 1 <= seconds <= 1200:
        raise ValueError('invalid receiver deadline')
    previous_handler = signal.getsignal(signal.SIGALRM)
    previous_timer = signal.getitimer(signal.ITIMER_REAL)
    if previous_timer != (0.0, 0.0):
        raise ValueError('receiver requires its own unarmed process timer')
    def expired(signum, frame):
        raise TimeoutError('native source receiver deadline expired')
    signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, seconds)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous_handler)


def receiver_progress(stage, **facts):
    # Diagnostic only; qualification still requires the exact final stdout row.
    print(json.dumps({'phase': 'source-receiver', 'stage': stage,
                      'monotonic_ns': time.monotonic_ns(), **facts},
                     separators=(',', ':')), file=sys.stderr, flush=True)


def upload(archive, source, command, seconds):
    facts = describe(archive, source)
    if not 1 <= seconds <= 1200 or not command:
        raise ValueError('invalid finite source transfer')
    started = time.time_ns()
    timed_out = False
    with Path(archive).open('rb') as stream:
        child = subprocess.Popen(command, stdin=stream, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, start_new_session=True)
        try:
            stdout, stderr = child.communicate(timeout=seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                stdout, stderr = child.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                stdout, stderr = child.communicate(timeout=5)
        except BaseException:
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                child.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.communicate(timeout=5)
            raise
    actual = 124 if timed_out else child.returncode if child.returncode >= 0 else 128 - child.returncode
    receiver = None
    verification_error = None
    try:
        receiver = json.loads(stdout)
    except ValueError:
        pass
    if actual == 0:
        try:
            if not isinstance(receiver, dict):
                raise ValueError('receiver did not return source metadata')
            if receiver.get('actual_exit') != 0 or not receiver.get('published') or any(
                    receiver.get(k) != facts[k] for k in ('source_commit', 'archive_bytes', 'archive_sha256')):
                raise ValueError('receiver source identity does not match immutable packet')
            if describe(archive, source) != facts:
                raise ValueError('local source packet changed during transfer')
        except (ValueError, OSError) as error:
            verification_error = str(error)
    return {**facts, 'phase': 'source-transfer', 'command': command,
            'started_ns': started, 'finished_ns': time.time_ns(),
            'command_exit': child.returncode, 'actual_exit': actual,
            'runner_exit': actual if actual else 3 if verification_error else 0,
            'timed_out': timed_out, 'receiver': receiver,
            'verification_error': verification_error,
            'diagnostics': {'stdout_bytes': len(stdout), 'stdout_sha256': hashlib.sha256(stdout).hexdigest(),
                            'stderr_bytes': len(stderr), 'stderr_sha256': hashlib.sha256(stderr).hexdigest(),
                            'stderr_tail': stderr[-4096:].decode('utf-8', 'replace')},
            'ready': actual == 0 and verification_error is None}


def write_receipt(path, facts):
    text = json.dumps({'schema': 1, **facts}, separators=(',', ':')) + '\n'
    if path:
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        pending = path.with_name(path.name + '.' + uuid.uuid4().hex + '.pending')
        with pending.open('x') as output:
            output.write(text)
            output.flush()
            os.fsync(output.fileno())
        os.replace(pending, path)
        if sys.platform.startswith('linux'):
            fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
    print(text, end='', flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--receipt')
    sub = parser.add_subparsers(dest='phase', required=True)
    p = sub.add_parser('describe'); p.add_argument('archive'); p.add_argument('source')
    p = sub.add_parser('receive'); p.add_argument('destination'); p.add_argument('bytes', type=int); p.add_argument('sha256'); p.add_argument('source'); p.add_argument('--seconds', type=int, default=900)
    p = sub.add_parser('send'); p.add_argument('archive'); p.add_argument('source'); p.add_argument('host'); p.add_argument('port', type=int); p.add_argument('key'); p.add_argument('destination'); p.add_argument('--seconds', type=int, default=900)
    args = parser.parse_args()
    try:
        if args.phase == 'describe':
            facts = describe(args.archive, args.source)
        elif args.phase == 'receive':
            receiver_progress('started', expected_bytes=args.bytes)
            with receiver_deadline(args.seconds):
                facts = {'actual_exit': 0, **receive(args.destination, args.bytes, args.sha256, args.source, sys.stdin.buffer, receiver_progress)}
        else:
            facts = describe(args.archive, args.source)
            code = Path(__file__).read_text()
            remote = shlex.join(['/usr/bin/timeout', '--kill-after=5', str(args.seconds),
                                 '/usr/bin/python3', '-I', '-c', code, 'receive', args.destination,
                                 str(facts['archive_bytes']), facts['archive_sha256'], args.source,
                                 '--seconds', str(args.seconds)])
            command = ['ssh', '-T', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=30',
                       '-o', 'ServerAliveInterval=30', '-o', 'ServerAliveCountMax=2',
                       '-o', 'StrictHostKeyChecking=no', '-o', 'UserKnownHostsFile=/dev/null',
                       '-o', 'LogLevel=ERROR', '-i', args.key, '-p', str(args.port), args.host, remote]
            facts = upload(args.archive, args.source, command, args.seconds)
    except Exception as error:
        error_exit = 124 if args.phase == 'receive' and isinstance(error, TimeoutError) else 3
        write_receipt(args.receipt, {'phase': args.phase, 'source_commit': getattr(args, 'source', None),
                      'actual_exit': error_exit, 'ready': False, 'error': {'type': type(error).__name__,
                      'errno': getattr(error, 'errno', None), 'message': str(error)},
                      **({'archive_bytes': args.bytes, 'archive_sha256': args.sha256,
                          'native_packet': args.destination, 'published': None}
                         if args.phase == 'receive' else {})})
        return error_exit
    write_receipt(args.receipt, facts)
    return facts.get('runner_exit', facts.get('actual_exit', 0))


if __name__ == '__main__':
    raise SystemExit(main())
