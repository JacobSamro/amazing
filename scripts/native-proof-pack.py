#!/usr/bin/env python3
"""Lossless declared native target packs; no source omissions or external references."""
import gzip
import hashlib
import os
from pathlib import Path, PurePosixPath
import shutil
import tarfile


class BoundedOutput:
    def __init__(self, stream, root, base_required, previous_bytes, budget, reserve):
        self.stream, self.root = stream, root
        self.base_required, self.previous_bytes = base_required, previous_bytes
        self.budget, self.reserve, self.bytes = budget, reserve, 0
    def write(self, data):
        required = self.base_required + self.previous_bytes + self.bytes + len(data)
        if required > self.budget or shutil.disk_usage(self.root).free < self.base_required + self.reserve + len(data):
            raise OSError(28, 'lossless native pack exceeds explicit capacity; original inputs untouched')
        written = self.stream.write(data)
        if written != len(data): raise OSError('short native pack write')
        self.bytes += written
        return written
    def flush(self): self.stream.flush()
    def tell(self): return self.bytes


def valid_name(name):
    p = PurePosixPath(name)
    if p.is_absolute() or '..' in p.parts or str(p) != name or name == '':
        raise ValueError('noncanonical packed member: ' + name)
    return name


def write_pack(path, rows, directories, original, fingerprint, root,
               base_required, previous_bytes, budget, reserve):
    if any(row['elf'] for row in rows):
        raise ValueError('measured/native ELF targets must retain ordinary independently readable payloads')
    with path.open('xb') as output:
        bounded = BoundedOutput(output, root, base_required, previous_bytes, budget, reserve)
        with gzip.GzipFile(filename='', mode='wb', fileobj=bounded, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode='w|', format=tarfile.GNU_FORMAT) as pack:
                for directory in directories:
                    member = tarfile.TarInfo(valid_name(directory['path']))
                    member.type = tarfile.DIRTYPE; member.mode = 0o700
                    pack.addfile(member)
                for row in rows:
                    source = original(row)
                    if list(fingerprint(source.stat())) != row['source_identity']:
                        raise ValueError('packed target changed before read: ' + str(source))
                    member = tarfile.TarInfo(valid_name(row['path']))
                    member.size = row['bytes']; member.mode = row['mode']
                    with source.open('rb') as stream: pack.addfile(member, stream)
                    if list(fingerprint(source.stat())) != row['source_identity']:
                        raise ValueError('packed target changed during read: ' + str(source))
        output.flush(); os.fsync(output.fileno())
    with path.open('rb') as stream: digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    return {'path': str(path), 'bytes': path.stat().st_size, 'sha256': digest,
            'encoding': 'gzip-tar-gnu', 'logical_bytes': sum(row['bytes'] for row in rows)}


def readback(path, rows, directories, restore=None):
    """Read every decompressed byte; optional native restoration never follows links."""
    expected = {row['path']: row for row in rows}
    expected_dirs = {row['path'] for row in directories}
    seen, seen_dirs = set(), set()
    if restore is not None:
        restore = Path(restore)
        if restore.exists() or restore.is_symlink(): raise ValueError('restore namespace must be fresh')
        restore.mkdir(mode=0o700)
    with tarfile.open(path, mode='r|gz') as pack:
        for member in pack:
            name = valid_name(member.name)
            if member.isdir():
                if name not in expected_dirs or name in seen_dirs: raise ValueError('unexpected/duplicate packed directory')
                seen_dirs.add(name)
                if restore is not None: (restore / name).mkdir(parents=True, exist_ok=True, mode=0o700)
                continue
            if not member.isfile() or name not in expected or name in seen:
                raise ValueError('unexpected/duplicate/nonregular packed file: ' + name)
            row = expected[name]
            if member.size != row['bytes'] or member.mode != row['mode']:
                raise ValueError('packed member byte/mode identity changed')
            digest = hashlib.sha256(); total = 0
            stream = pack.extractfile(member)
            final = restore / name if restore is not None else None
            output = None
            try:
                if final is not None:
                    final.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                    output = final.open('xb')
                while True:
                    data = stream.read(1024 * 1024)
                    if not data: break
                    total += len(data); digest.update(data)
                    if total > row['bytes']: raise ValueError('oversized packed member')
                    if output is not None: output.write(data)
                if total != row['bytes'] or digest.hexdigest() != row['sha256']:
                    raise ValueError('decompressed target bytes/hash mismatch: ' + name)
                if output is not None:
                    output.flush(); os.fsync(output.fileno()); output.close(); output = None
                    final.chmod(row['mode'])
                    with final.open('rb') as check:
                        if hashlib.file_digest(check, 'sha256').hexdigest() != row['sha256']:
                            raise ValueError('restored target hash mismatch')
                        os.fsync(check.fileno())
                seen.add(name)
            finally:
                if output is not None: output.close()
    if seen != set(expected) or seen_dirs != expected_dirs:
        raise ValueError('packed capture omitted original files or directories')
    return {'files': len(seen), 'directories': len(seen_dirs),
            'logical_bytes': sum(row['bytes'] for row in rows), 'all_member_hashes_verified': True,
            'restored': restore is not None}
