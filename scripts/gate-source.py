#!/usr/bin/env python3
"""Materialize an exact ref without historical raw evidence; never prune source."""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess


def main():
    p = argparse.ArgumentParser()
    p.add_argument('repo'); p.add_argument('commit'); p.add_argument('destination')
    args = p.parse_args()
    tree = subprocess.check_output(['git', '-C', args.repo, 'ls-tree', '-rlz', args.commit])
    files = {}
    for row in tree.split(b'\0'):
        if not row:
            continue
        metadata, name = row.split(b'\t', 1)
        mode, kind, oid, size = metadata.split()
        files[name.decode()] = (oid.decode(), int(size) if size != b'-' else 0)
    # Retain every literal Rust include, even when its target is historical-looking.
    protected = set()
    rust = [(name, oid) for name, (oid, _) in files.items() if name.endswith('.rs')]
    request = ''.join(oid + '\n' for _, oid in rust).encode()
    data = subprocess.check_output(['git', '-C', args.repo, 'cat-file', '--batch'], input=request)
    offset = 0
    for name, _ in rust:
        end = data.index(b'\n', offset)
        length = int(data[offset:end].split()[-1]); start = end + 1
        source = data[start:start + length].decode(errors='replace'); offset = start + length + 1
        for value in re.findall(r'include_(?:str|bytes)!\s*\(\s*"([^"\n]+)"', source):
            # Pure lexical normalization; no local sparse-checkout dependency.
            parts = []
            for part in (pathlib.PurePosixPath(name).parent / value).parts:
                if part == '..':
                    if parts: parts.pop()
                elif part != '.': parts.append(part)
            protected.add('/'.join(parts))
    omitted = []
    for name, (_, size) in files.items():
        path = pathlib.PurePosixPath(name)
        evidence = 'evidence' in path.parts or name.startswith('docs/lanes/')
        fixture = any(part in ('fixtures', 'testdata', 'tests') for part in path.parts)
        raw = path.suffix == '.jsonl' or (size > 512 * 1024 and path.suffix in
               ('.json', '.log', '.png', '.ppm', '.tgz', '.gz', '.tar', '.zip', '.zst'))
        if evidence and raw and not fixture and path.suffix != '.tsv' and name not in protected:
            omitted.append(name)
    # Exclusion pathspecs keep raw objects out of the archive itself, not just rsync.
    command = ['git', '-C', args.repo, 'archive', args.commit, '--', '.']
    command.extend(':(exclude,literal)' + name for name in sorted(omitted))
    archive = subprocess.Popen(command, stdout=subprocess.PIPE)
    try:
        extract = subprocess.run(['tar', '-x', '-C', args.destination], stdin=archive.stdout)
    finally:
        archive.stdout.close()
    archive_exit = archive.wait()
    if archive_exit or extract.returncode:
        raise SystemExit(3)
    print(json.dumps({'source_commit': args.commit, 'omitted_raw_files': len(omitted),
                      'omitted_raw_bytes': sum(files[n][1] for n in omitted),
                      'omitted_paths_sha256': hashlib.sha256('\n'.join(sorted(omitted)).encode()).hexdigest(),
                      'retained_literal_includes': len(protected), 'tsv_retained': True}))


if __name__ == '__main__':
    main()
