#!/usr/bin/env python3
"""Bounded actual-window proof metadata, independent of conformance results."""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import re
import struct
import subprocess

IMAGE_LIMIT = 512 * 1024
PROOF_LIMIT = 2 * 1024 * 1024


def digest(path):
    with pathlib.Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def exact_path(path, lane):
    if not re.fullmatch(r'[a-z0-9-]+', lane): raise ValueError('invalid lane')
    if not re.fullmatch(r'[A-Za-z0-9_./-]+', path) or '..' in pathlib.PurePosixPath(path).parts:
        raise ValueError('explicit image path required; no globs or traversal')
    if not path.startswith(f'/home/user/amazing/out/{lane}/') or not path.endswith('.png'):
        raise ValueError('image must be an exact PNG under the lane output directory')
    return path


def png_size(path):
    with pathlib.Path(path).open('rb') as stream: header = stream.read(24)
    if len(header) != 24 or header[:8] != b'\x89PNG\r\n\x1a\n' or header[12:16] != b'IHDR':
        raise ValueError('not a PNG image')
    return list(struct.unpack('>II', header[16:24]))


def validate(proof, lane):
    if proof.get('schema') != 1 or proof.get('kind') != 'actual-window-proof' or proof.get('lane') != lane:
        raise ValueError('invalid window proof manifest')
    images = proof.get('images', [])
    if not images: raise ValueError('proof has no actual window images')
    paths, names = set(), set()
    total = len((json.dumps(proof, separators=(',', ':')) + '\n').encode())
    if total > IMAGE_LIMIT: raise ValueError('manifest exceeds 512 KiB')
    for image in images:
        path = exact_path(image['path'], lane)
        name = pathlib.PurePosixPath(path).name
        if path in paths or name in names: raise ValueError('duplicate image path/name')
        paths.add(path); names.add(name)
        if not 0 < image['bytes'] <= IMAGE_LIMIT: raise ValueError('PNG exceeds 512 KiB')
        total += image['bytes']
        if not re.fullmatch(r'[0-9a-f]{64}', image['sha256']): raise ValueError('invalid PNG hash')
        if not re.fullmatch(r'[0-9a-f]{64}', image['exe']['sha256']): raise ValueError('invalid executable hash')
        if not image['exe']['path'] or image['pid'] <= 0 or int(image['window']['id']) <= 0:
            raise ValueError('missing actual executable/process/window identity')
        if not re.fullmatch(r'[0-9a-f]{40}', image['source_commit']) or not image['fixture']:
            raise ValueError('missing source/fixture identity')
        for dimensions in (image['viewport']['display_size'], image['viewport']['window_size']):
            if len(dimensions) != 2 or any(not isinstance(n, int) or n <= 0 for n in dimensions):
                raise ValueError('invalid viewport')
        captured = datetime.datetime.fromisoformat(image['captured_at'].replace('Z', '+00:00'))
        if captured.utcoffset() is None: raise ValueError('capture timestamp requires timezone')
    if total > PROOF_LIMIT: raise ValueError('whole visual proof exceeds 2 MiB')
    return proof


def verify_image(image, local):
    path = pathlib.Path(local)
    if path.stat().st_size != image['bytes'] or digest(path) != image['sha256']:
        raise ValueError('PNG hash/size mismatch')
    if png_size(path) != image['viewport']['window_size']:
        raise ValueError('PNG dimensions differ from captured window')


def record(args):
    path = pathlib.Path(args.image).resolve()
    exact_path(str(path), args.lane)
    exe = pathlib.Path(os.readlink(f'/proc/{args.pid}/exe')).resolve()
    if exe != pathlib.Path(args.exe).resolve(): raise ValueError('running executable mismatch')
    window_pid = int(subprocess.check_output(['xdotool', 'getwindowpid', args.window], text=True).strip())
    if window_pid != args.pid: raise ValueError('window does not belong to actual shell PID')
    geometry = subprocess.check_output(['xdotool', 'getwindowgeometry', '--shell', args.window], text=True)
    geometry = dict(line.split('=', 1) for line in geometry.splitlines() if '=' in line)
    dimensions = [int(geometry['WIDTH']), int(geometry['HEIGHT'])]
    if png_size(path) != dimensions: raise ValueError('capture is not the actual window dimensions')
    image = {'path': str(path), 'bytes': path.stat().st_size, 'sha256': digest(path),
             'exe': {'path': str(exe), 'sha256': digest(exe)}, 'pid': args.pid,
             'source_commit': args.source_commit, 'fixture': args.fixture,
             'viewport': {'display_size': [int(n) for n in args.size.split('x')], 'window_size': dimensions},
             'window': {'id': args.window, 'title': subprocess.check_output(['xdotool', 'getwindowname', args.window], text=True).strip()},
             'captured_at': datetime.datetime.now(datetime.timezone.utc).isoformat()}
    manifest = pathlib.Path(args.manifest)
    proof = json.loads(manifest.read_text()) if manifest.exists() else {'schema': 1, 'kind': 'actual-window-proof', 'lane': args.lane, 'images': []}
    proof['images'].append(image)
    validate(proof, args.lane)
    manifest.write_text(json.dumps(proof, separators=(',', ':')) + '\n')


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='command', required=True)
    r = sub.add_parser('record')
    for name in ('lane', 'manifest', 'image', 'exe', 'window', 'source-commit', 'fixture', 'size'):
        r.add_argument('--' + name, required=True)
    r.add_argument('--pid', required=True, type=int)
    v = sub.add_parser('validate'); v.add_argument('manifest'); v.add_argument('lane')
    c = sub.add_parser('verify'); c.add_argument('manifest'); c.add_argument('lane'); c.add_argument('directory')
    args = parser.parse_args()
    if args.command == 'record': record(args)
    else:
        proof = validate(json.loads(pathlib.Path(args.manifest).read_text()), args.lane)
        if args.command == 'verify':
            for image in proof['images']: verify_image(image, pathlib.Path(args.directory) / pathlib.PurePosixPath(image['path']).name)
        else:
            for image in proof['images']: print(f"{image['sha256']} {image['bytes']} {image['path']}")


if __name__ == '__main__':
    main()
