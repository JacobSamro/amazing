#!/usr/bin/env python3
"""Delta source upload for scripts/linux-run.sh (Mac planner, Boat applier).

Every Boat run used to upload the whole source archive (~75 MB). A box now keeps up to three
verified source archives under ~/amazing/src-cache (one regular file each, so nothing is written
file-by-file through the lazy HOME filesystem). The Mac asks which commits a box has, picks the
cached tree with the fewest differences from the target (nearest commit by git distance, then the
smallest manifest delta), uploads only the changed files plus a deletion list, and the box rebuilds
the target tree from cache + delta and verifies it file by file against a manifest of
path/mode/sha256 computed on the Mac before anything runs. Any mismatch, missing cache or tool
failure exits non-zero and the caller falls back to the full upload.

Manifest: {path: ["f", exec_bit, sha256, size] | ["l", 0, link_target, 0]}; identity of a tree is
the sha256 of its canonical manifest JSON. Top-level .git, target, target-*, .agents, out, corpora
and source-transfer.manifest.json are never part of a source tree (same exclusions as the full
archive).

Subcommands
  manifest  --root DIR --out FILE           Mac: write the manifest, print its identity
  upload    ...                             Mac: plan + send delta + apply on the box (exit 10: no base)
  register  ...                             Mac: after a full upload, add the box's archive to its cache
  list | show SHA | apply | ingest          Box-side helpers (run through ssh)
"""
import argparse
import base64
import gzip
import hashlib
import json
import os
import pathlib
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time

EXCLUDED_TOP = ('.git', 'target', '.agents', 'out', 'corpora', 'source-transfer.manifest.json')
KEEP = 3
CACHE = pathlib.Path(os.path.expanduser('~/amazing/src-cache'))
NO_BASE = 10


def excluded_top(name):
    return name in EXCLUDED_TOP or name.startswith('target-')


def sha_file(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''):
            digest.update(block)
    return digest.hexdigest()


def build_manifest(root):
    root = pathlib.Path(root)
    manifest = {}

    def walk(directory, prefix):
        with os.scandir(directory) as entries:
            for entry in sorted(entries, key=lambda e: e.name):
                if not prefix and excluded_top(entry.name):
                    continue
                relative = prefix + entry.name
                if entry.is_symlink():
                    manifest[relative] = ['l', 0, os.readlink(entry.path), 0]
                elif entry.is_dir(follow_symlinks=False):
                    walk(entry.path, relative + '/')
                elif entry.is_file(follow_symlinks=False):
                    status = entry.stat(follow_symlinks=False)
                    manifest[relative] = ['f', 1 if status.st_mode & 0o111 else 0,
                                          sha_file(entry.path), status.st_size]
    walk(str(root), '')
    return manifest


def manifest_identity(manifest):
    text = json.dumps(sorted(manifest.items()), separators=(',', ':'))
    return hashlib.sha256(text.encode()).hexdigest()


def write_json(path, value):
    pathlib.Path(path).write_text(json.dumps(value, separators=(',', ':')))


# ---------------------------------------------------------------- box side
def cache_entries():
    entries = []
    if CACHE.is_dir():
        for meta in CACHE.glob('*.json'):
            try:
                row = json.loads(meta.read_text())
                archive = CACHE / (row['manifest_sha256'] + '.tar.gz')
                if archive.is_file() and archive.stat().st_size == row['archive_bytes']:
                    entries.append({'manifest_sha256': row['manifest_sha256'], 'label': row['label'],
                                    'archive_sha256': row['archive_sha256'], 'used': meta.stat().st_mtime})
            except (OSError, ValueError, KeyError):
                continue
    entries.sort(key=lambda row: -row['used'])
    return entries


def prune(keep_sha):
    entries = cache_entries()
    survivors = [row for row in entries if row['manifest_sha256'] == keep_sha]
    survivors += [row for row in entries if row['manifest_sha256'] != keep_sha][:KEEP - len(survivors)]
    keep = {row['manifest_sha256'] for row in survivors}
    for path in CACHE.iterdir():
        stem = path.name.split('.')[0]
        if len(stem) == 64 and stem not in keep:
            path.unlink(missing_ok=True)


def pack_tree(tree, manifest, destination):
    with tarfile.open(destination, 'w:gz', compresslevel=1) as archive:
        for relative in sorted(manifest):
            archive.add(os.path.join(tree, relative), arcname=relative, recursive=False)
    return sha_file(destination)


def register(archive_path, manifest, label):
    """Copy a verified archive into the cache (one regular file + a meta file)."""
    import fcntl
    CACHE.mkdir(parents=True, exist_ok=True)
    with open(CACHE / '.lock', 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        return register_locked(archive_path, manifest, label)


def register_locked(archive_path, manifest, label):
    identity = manifest_identity(manifest)
    target = CACHE / (identity + '.tar.gz')
    pending = CACHE / (identity + '.tar.gz.pending')
    shutil.copyfile(archive_path, pending)
    os.replace(pending, target)
    row = {'manifest_sha256': identity, 'label': label, 'archive_sha256': sha_file(target),
           'archive_bytes': target.stat().st_size, 'created': time.time(), 'manifest': manifest}
    meta_pending = CACHE / (identity + '.json.pending')
    write_json(meta_pending, row)
    os.replace(meta_pending, CACHE / (identity + '.json'))
    prune(identity)
    return row


def extract_verified(archive, work):
    tree = os.path.join(work, 'tree')
    os.makedirs(tree)
    with tarfile.open(archive) as stream:
        stream.extractall(tree, filter='data')
    return tree


def cmd_list(_args):
    print(json.dumps(cache_entries()))


def cmd_show(args):
    path = CACHE / (args.sha + '.json')
    print(json.dumps(json.loads(path.read_text())['manifest'], separators=(',', ':')))


def cmd_apply(args):
    work = tempfile.mkdtemp(prefix='amazing-delta-work.', dir='/tmp')
    try:
        if args.base:
            tree = extract_verified(CACHE / (args.base + '.tar.gz'), work)
            (CACHE / (args.base + '.json')).touch()
        else:
            tree = os.path.join(work, 'tree'); os.makedirs(tree)
        with tarfile.open(args.delta) as delta:
            expected = json.load(delta.extractfile('.delta/manifest.json'))
            deletions = json.load(delta.extractfile('.delta/deletions.json'))
            for member in delta.getmembers():
                if member.name.startswith('.delta/'):
                    continue
                destination = os.path.join(tree, member.name)
                if os.path.lexists(destination) and not os.path.isdir(destination):
                    os.unlink(destination)
                delta.extract(member, tree, filter='data')
        for relative in deletions:
            path = os.path.join(tree, relative)
            if os.path.lexists(path):
                os.unlink(path)
        # Verification: every shipped file must match its entry, and the rebuilt tree's manifest
        # identity (path+mode+sha256 of every file) must equal the one the Mac computed.
        actual = build_manifest(tree)
        bad = sorted(k for k, v in expected.items() if actual.get(k) != v)[:3]
        if bad:
            raise SystemExit(f'delta file mismatch: {bad}')
        if manifest_identity(actual) != args.manifest_sha:
            raise SystemExit('delta tree identity mismatch (cache or delta does not reproduce the source)')
        archive_sha = pack_tree(tree, actual, args.out)
        row = register(args.out, actual, args.label)
        print(json.dumps({'archive_sha256': archive_sha, 'archive_bytes': os.path.getsize(args.out),
                          'manifest_sha256': args.manifest_sha, 'files': len(actual),
                          'cache_entries': len(cache_entries()), 'base': args.base or None}))
        return row
    finally:
        shutil.rmtree(work, ignore_errors=True)


def cmd_ingest(args):
    work = tempfile.mkdtemp(prefix='amazing-delta-work.', dir='/tmp')
    try:
        tree = extract_verified(args.archive, work)
        actual = build_manifest(tree)
        identity = manifest_identity(actual)
        if identity != args.manifest_sha:
            raise SystemExit('ingest manifest mismatch; cache not updated')
        register(args.archive, actual, args.label)
        print(json.dumps({'manifest_sha256': identity, 'cache_entries': len(cache_entries())}))
    finally:
        shutil.rmtree(work, ignore_errors=True)


# ---------------------------------------------------------------- Mac side
def remote(ssh, args, script_b64, timeout):
    """Run this file on the box: ssh ... python3 - <subcommand> <args> (script via base64)."""
    command = ('timeout --kill-after=5 %d bash -c %s' % (timeout, shlex.quote(
        "printf %s " + shlex.quote(script_b64) + " | base64 -d | python3 -I - " + ' '.join(shlex.quote(a) for a in args))))
    return subprocess.run(shlex.split(ssh) + [command], capture_output=True, text=True, timeout=timeout + 30)


def git_distance(repo, left, right):
    try:
        out = subprocess.run(['git', '-C', repo, 'rev-list', '--count', f'{left}...{right}'],
                             capture_output=True, text=True, timeout=30)
        return int(out.stdout.strip()) if out.returncode == 0 else 10 ** 9
    except (OSError, ValueError, subprocess.SubprocessError):
        return 10 ** 9


def choose_base(entries, repo, commit):
    best = None
    for index, row in enumerate(entries):
        label_commit = row['label'].split('+')[0]
        distance = git_distance(repo, label_commit, commit) if len(label_commit) == 40 and commit else 10 ** 9
        key = (distance, index)
        if best is None or key < best[0]:
            best = (key, row)
    return best[1] if best else None


def build_delta(root, manifest, base_manifest, delta_path):
    """Tar of the changed/added files plus the target manifest and the deletion list."""
    import io
    changed = sorted(p for p, v in manifest.items() if base_manifest.get(p) != v)
    deletions = sorted(p for p in base_manifest if p not in manifest)
    with tarfile.open(delta_path, 'w:gz', compresslevel=6) as archive:
        for relative in changed:
            archive.add(os.path.join(root, relative), arcname=relative, recursive=False)
        # Only the changed entries travel; the target identity is checked on the box by recomputing
        # the whole tree's manifest hash (a few hundred KB cheaper than shipping the full manifest).
        for name, value in (('.delta/manifest.json', {p: manifest[p] for p in changed}),
                            ('.delta/deletions.json', deletions)):
            blob = json.dumps(value, separators=(',', ':')).encode()
            info = tarfile.TarInfo(name); info.size = len(blob); info.mtime = 0
            archive.addfile(info, io.BytesIO(blob))
    return changed, deletions


def cmd_manifest(args):
    manifest = build_manifest(args.root)
    write_json(args.out, manifest)
    if args.list:
        # NUL-separated path list: the full archive is cut from exactly the manifest's files.
        pathlib.Path(args.list).write_bytes(b''.join(name.encode() + b'\0' for name in sorted(manifest)))
    print(json.dumps({'manifest_sha256': manifest_identity(manifest), 'files': len(manifest)}))


def cmd_upload(args):
    started = time.time()
    script_b64 = base64.b64encode(pathlib.Path(__file__).read_bytes()).decode()
    manifest = json.loads(pathlib.Path(args.manifest).read_text())
    identity = manifest_identity(manifest)
    listing = remote(args.ssh, ['list'], script_b64, 90)
    if listing.returncode != 0:
        print('src-delta: cache listing failed: ' + listing.stderr[-300:], file=sys.stderr); return 1
    entries = json.loads(listing.stdout.strip().splitlines()[-1])
    base = choose_base(entries, args.repo, args.commit)
    if base is None:
        print('src-delta: no cached source on the box', file=sys.stderr); return NO_BASE
    store = pathlib.Path(args.store); store.mkdir(parents=True, exist_ok=True)
    cached = store / (base['manifest_sha256'] + '.json')
    if not cached.is_file():
        shown = remote(args.ssh, ['show', base['manifest_sha256']], script_b64, 120)
        if shown.returncode != 0:
            print('src-delta: base manifest fetch failed', file=sys.stderr); return 1
        cached.write_text(shown.stdout.strip().splitlines()[-1])
    base_manifest = json.loads(cached.read_text())
    if manifest_identity(base_manifest) != base['manifest_sha256']:
        cached.unlink(missing_ok=True)
        print('src-delta: stored base manifest does not match its identity', file=sys.stderr); return 1
    delta_path = pathlib.Path(args.work) / 'source-delta.tar.gz'
    changed, deletions = build_delta(args.root, manifest, base_manifest, delta_path)
    delta_bytes = delta_path.stat().st_size
    send = subprocess.run([sys.executable, args.source_upload, '--receipt', args.delta_receipt, 'send',
                           str(delta_path), args.commit or 'working-tree', args.host, str(args.port), args.key,
                           args.native_dir + '/source-delta.tar.gz', '--seconds', '900'])
    if send.returncode != 0:
        print('src-delta: delta upload failed', file=sys.stderr); return 1
    applied = remote(args.ssh, ['apply', '--delta', args.native_dir + '/source-delta.tar.gz', '--base', base['manifest_sha256'],
                                '--manifest-sha', identity, '--label', args.label, '--out', args.native_dir + '/source.tar.gz'],
                     script_b64, 600)
    if applied.returncode != 0:
        print('src-delta: box rebuild failed: ' + (applied.stderr or applied.stdout)[-400:], file=sys.stderr); return 1
    result = json.loads(applied.stdout.strip().splitlines()[-1])
    if result['manifest_sha256'] != identity:
        print('src-delta: box reports a different manifest identity', file=sys.stderr); return 1
    store_manifest = store / (identity + '.json')
    store_manifest.write_text(json.dumps(manifest, separators=(',', ':')))
    for old in sorted(store.glob('*.json'), key=lambda p: -p.stat().st_mtime)[20:]:
        old.unlink(missing_ok=True)
    summary = {'mode': 'delta', 'source_commit': args.commit or 'working-tree', 'manifest_sha256': identity,
               'base_manifest_sha256': base['manifest_sha256'], 'base_label': base['label'],
               'changed_files': len(changed), 'deleted_files': len(deletions), 'delta_bytes': delta_bytes,
               'files': len(manifest), 'box_archive_sha256': result['archive_sha256'],
               'box_archive_bytes': result['archive_bytes'], 'seconds': round(time.time() - started, 1)}
    write_json(args.summary, summary)
    print(json.dumps(summary))
    return 0


def cmd_register(args):
    script_b64 = base64.b64encode(pathlib.Path(__file__).read_bytes()).decode()
    manifest = json.loads(pathlib.Path(args.manifest).read_text())
    done = remote(args.ssh, ['ingest', '--archive', args.archive, '--manifest-sha', manifest_identity(manifest),
                             '--label', args.label], script_b64, 600)
    if done.returncode != 0:
        print('src-delta: cache registration skipped: ' + (done.stderr or done.stdout)[-200:], file=sys.stderr)
        return 1
    store = pathlib.Path(args.store); store.mkdir(parents=True, exist_ok=True)
    (store / (manifest_identity(manifest) + '.json')).write_text(json.dumps(manifest, separators=(',', ':')))
    print(done.stdout.strip().splitlines()[-1])
    return 0


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('manifest'); p.add_argument('--root', required=True); p.add_argument('--out', required=True)
    p.add_argument('--list', default='')
    p.set_defaults(run=cmd_manifest)
    p = sub.add_parser('upload')
    for name in ('manifest', 'root', 'ssh', 'host', 'port', 'key', 'native-dir', 'label', 'store', 'work', 'repo',
                 'source-upload', 'delta-receipt', 'summary'):
        p.add_argument('--' + name, required=True)
    p.add_argument('--commit', default='')
    p.set_defaults(run=cmd_upload)
    p = sub.add_parser('register')
    for name in ('manifest', 'ssh', 'archive', 'label', 'store'):
        p.add_argument('--' + name, required=True)
    p.set_defaults(run=cmd_register)
    sub.add_parser('list').set_defaults(run=cmd_list)
    p = sub.add_parser('show'); p.add_argument('sha'); p.set_defaults(run=cmd_show)
    p = sub.add_parser('apply')
    p.add_argument('--delta', required=True); p.add_argument('--base', default='')
    p.add_argument('--manifest-sha', required=True); p.add_argument('--label', required=True)
    p.add_argument('--out', required=True); p.set_defaults(run=cmd_apply)
    p = sub.add_parser('ingest')
    p.add_argument('--archive', required=True); p.add_argument('--manifest-sha', required=True)
    p.add_argument('--label', required=True); p.set_defaults(run=cmd_ingest)
    args = parser.parse_args()
    result = args.run(args)
    sys.exit(result if isinstance(result, int) else 0)


if __name__ == '__main__':
    main()
