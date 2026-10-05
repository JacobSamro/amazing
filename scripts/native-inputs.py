#!/usr/bin/env python3
"""Finite public source/corpus staging. Engine and test bytes are never rewritten."""
import argparse
import hashlib
import fcntl
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import signal
import sysconfig
import time
import uuid

REVISION = '3655e7464de3d52643ecddd4b5f9f4f3e7f62398'
INPUT_SHA = '91109b8248e3825d658903f2496535b9500c8a776794355da7f32b6942ceddc5'


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def receipt(path, facts):
    facts = {'schema': 1, 'source_commit': os.environ.get('COMMIT', 'working-tree'), **facts}
    text = json.dumps(facts, sort_keys=True, separators=(',', ':')) + '\n'
    destination = Path(path)
    destination.parent.mkdir(parents=True, exist_ok=True)
    pending = destination.with_name(destination.name + '.' + uuid.uuid4().hex + '.pending')
    try:
        with pending.open('x') as stream:
            stream.write(text); stream.flush(); os.fsync(stream.fileno())
        os.replace(pending, destination)
        if sys.platform.startswith('linux'):
            fd = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
            try: os.fsync(fd)
            finally: os.close(fd)
    finally:
        pending.unlink(missing_ok=True)
    print(text, end='', flush=True)


def extract(archive, expected, destination):
    archive, destination = Path(archive), Path(destination)
    if digest(archive) != expected:
        raise ValueError('source archive SHA256 mismatch')
    destination.mkdir(exist_ok=False, parents=True)
    if destination.is_symlink() or destination.stat().st_dev != Path('/tmp').stat().st_dev:
        raise ValueError('source destination is not native')
    with tarfile.open(archive) as stream:
        # Python data extraction rejects escaping links/devices/traversal.
        # Ordinary tracked symlinks are retained within the authenticated tree.
        stream.extractall(destination, filter='data')
    files = []
    for path in sorted(destination.rglob('*')):
        if path.is_file() and not path.is_symlink():
            files.append((path.relative_to(destination).as_posix(), digest(path)))

    if not (destination / 'Cargo.toml').is_file():
        raise ValueError('source archive has no workspace Cargo.toml')
    # Archive/commit dates are data provenance, not Cargo freshness. Give the
    # authenticated regular source files one genuine native staging generation;
    # preserve every byte and mode, and never follow a symlink into a corpus.
    stamp = time.time_ns()
    refreshed = []
    for relative, unused_digest in files:
        path = destination / relative
        os.utime(path, ns=(stamp, stamp), follow_symlinks=False)
        refreshed.append(path.stat().st_mtime_ns)
    if any(value != stamp for value in refreshed):
        raise ValueError('native source filesystem did not preserve staging timestamp')
    identity = hashlib.sha256(json.dumps(files, separators=(',', ':')).encode()).hexdigest()
    return {'archive_sha256': expected, 'source_tree_sha256': identity,
            'regular_files': len(files), 'source_path': str(destination),
            'source_timestamps': {'mode': 'native-staging-generation', 'stamp_ns': stamp,
                'regular_files_refreshed': len(refreshed), 'archive_dates_ignored': True}}


def inventory(root):
    root = Path(root)
    tests = sorted((root / 'test').rglob('*.js'))
    helpers = sorted((root / 'harness').rglob('*.js'))
    identity = hashlib.sha256()
    for path in tests + helpers:
        identity.update(path.relative_to(root).as_posix().encode() + b'\0')
        identity.update(bytes.fromhex(digest(path)))
    return len(tests), len(helpers), identity.hexdigest()


def corpus(root):
    root = Path(root)
    with (root.parent / (root.name + '.lock')).open('a') as lock:
        # The bootstrap stage's outer deadline also bounds waiting for another
        # lane's first native download. No competing checkout is overwritten.
        fcntl.flock(lock, fcntl.LOCK_EX)
        return corpus_locked(root)


def corpus_locked(root):
    if root.is_symlink() or (root.exists() and root.stat().st_dev != Path('/tmp').stat().st_dev):
        raise ValueError('Test262 destination is not native')
    env = {**os.environ, 'GIT_CONFIG_GLOBAL': '/dev/null', 'GIT_CONFIG_NOSYSTEM': '1'}
    if not root.exists():
        pending = Path(tempfile.mkdtemp(prefix='.test262-stage-', dir=root.parent))
        try:
            subprocess.run(['git', 'init', str(pending)], env=env, check=True, timeout=30)
            subprocess.run(['git', '-C', str(pending), 'fetch', '--depth=1',
                'https://github.com/tc39/test262.git', REVISION], env=env, check=True, timeout=900)
            subprocess.run(['git', '-C', str(pending), 'checkout', '--detach', 'FETCH_HEAD'],
                env=env, check=True, timeout=120)
            validate_corpus(pending, env)
            pending.rename(root)
        finally:
            if pending.exists(): shutil.rmtree(pending)
    return validate_corpus(root, env)


def validate_corpus(root, env):
    actual = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'],
        env=env, timeout=30, text=True).strip()
    if actual != REVISION: raise ValueError('Test262 revision mismatch')
    subprocess.run(['git', '-C', str(root), 'diff', '--exit-code', 'HEAD', '--'],
        env=env, check=True, timeout=60)
    untracked = subprocess.check_output(['git', '-C', str(root), 'ls-files', '--others', '--exclude-standard'],
        env=env, timeout=30)
    if untracked: raise ValueError('Test262 untracked input')
    tests, helpers, identity = inventory(root)
    if (tests, helpers, identity) != (53872, 43, INPUT_SHA):
        raise ValueError(f'Test262 input mismatch: {tests}/{helpers}/{identity}')
    return {'revision': actual, 'test_files': tests, 'helpers': helpers,
            'input_sha256': identity, 'root': str(root), 'origin': 'https://github.com/tc39/test262.git'}


def python_input(destination):
    destination = Path(destination)
    destination.mkdir(exist_ok=False, parents=True)
    # Locate the genuine installed package without importing its C
    # extension. Snapshot bytes are copied before the native import probe.
    version = f'python{sys.version_info.major}.{sys.version_info.minor}'
    candidates = [Path(sysconfig.get_path('purelib')) / 'yaml',
        Path('/usr/local/lib') / version / 'dist-packages/yaml',
        Path('/usr/lib/python3/dist-packages/yaml'),
        Path.home() / '.local/lib' / version / 'site-packages/yaml']
    origin = next((p for p in candidates if (p / '__init__.py').is_file()
        and re.search(r'''__version__\s*=\s*['"]6\.0\.3['"]''', (p / '__init__.py').read_text())), None)
    if origin is None: raise ValueError('installed PyYAML missing')
    rows = []
    for source in sorted(origin.rglob('*')):
        if source.is_file() and source.suffix in ('.py', '.so'):
            relative = source.relative_to(origin)
            data = source.read_bytes()
            target = destination / 'yaml' / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            identity = hashlib.sha256(data).hexdigest()
            if digest(target) != identity: raise ValueError('PyYAML native copy mismatch')
            rows.append((relative.as_posix(), identity))
    binary = destination / 'python3'
    shutil.copyfile(Path(sys.executable).resolve(), binary)
    binary.chmod(0o755)
    env = {k: v for k, v in os.environ.items() if not k.startswith(('PYTHON', '_PYTHON'))}
    env.update(PYTHONPATH=str(destination), PYTHONNOUSERSITE='1')
    probe = subprocess.check_output([str(binary), '-s', '-S', '-P', '-c',
        'import json,sys,yaml;print(json.dumps({"version":yaml.__version__,"origin":yaml.__file__,"python":sys.version}))'],
        env=env, timeout=30, text=True)
    facts = json.loads(probe)
    if facts['version'] != '6.0.3' or not Path(facts['origin']).is_relative_to(destination):
        raise ValueError('native PyYAML import provenance mismatch')
    return {'python_path': str(binary), 'python_sha256': digest(binary), 'python': facts['python'],
            'pyyaml': facts['version'], 'pyyaml_original': str(origin), 'pyyaml_native': facts['origin'],
            'pyyaml_files': len(rows), 'pyyaml_tree_sha256': hashlib.sha256(json.dumps(rows).encode()).hexdigest()}


def transfer(command, seconds):
    if command and command[0] == '--': command = command[1:]
    if not command or not 1 <= seconds <= 1200: raise ValueError('invalid finite transfer')
    proc = subprocess.Popen(command, start_new_session=True)
    timed_out = False
    try:
        code = proc.wait(timeout=seconds)
    except subprocess.TimeoutExpired:
        timed_out = True
        # This is the direct, still-unreaped child created by this function.
        # It is never a PID recovered from an output file or a different run.
        try: os.killpg(proc.pid, signal.SIGTERM)
        except ProcessLookupError: pass
        try: code = proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try: os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError: pass
            code = proc.wait(timeout=5)
    return {'command': command, 'command_exit': code, 'timed_out': timed_out,
            'actual_exit': 124 if timed_out else (code if code >= 0 else 128-code)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--receipt', required=True)
    sub = parser.add_subparsers(dest='phase', required=True)
    p = sub.add_parser('source'); p.add_argument('archive'); p.add_argument('sha256'); p.add_argument('destination')
    p = sub.add_parser('corpus'); p.add_argument('destination')
    p = sub.add_parser('python'); p.add_argument('destination')
    p = sub.add_parser('transfer'); p.add_argument('--seconds', type=int, required=True); p.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    try:
        if args.phase == 'source': facts = extract(args.archive, args.sha256, args.destination)
        elif args.phase == 'corpus': facts = corpus(args.destination)
        elif args.phase == 'python': facts = python_input(args.destination)
        else: facts = transfer(args.command, args.seconds)
    except BaseException as error:
        receipt(args.receipt, {'phase': args.phase, 'ready': False,
            'error': {'type': type(error).__name__, 'errno': getattr(error, 'errno', None), 'message': str(error)}})
        raise
    code = facts.get('actual_exit', 0)
    receipt(args.receipt, {'phase': args.phase, 'ready': code == 0, **facts})
    if code: raise SystemExit(code)


if __name__ == '__main__': main()
