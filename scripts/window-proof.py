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
        capture_facts = image['capture']
        if capture_facts['backend'] != 'ImageMagick import +frame +border -window':
            raise ValueError('unsupported capture backend')
        before, after = capture_facts['before'], capture_facts['after']
        if before != after: raise ValueError('capture geometry/identity was unstable')
        if after['exe'] != image['exe'] or after['pid'] != image['pid'] or str(after['window_id']) != str(image['window']['id']):
            raise ValueError('capture identity differs from manifest')
        check_geometry(after['geometry'], after['display_size'], image['viewport']['display_size'])
        if after['geometry'][2:] != image['viewport']['window_size']:
            raise ValueError('window geometry differs from viewport')
        captured = datetime.datetime.fromisoformat(image['captured_at'].replace('Z', '+00:00'))
        began = datetime.datetime.fromisoformat(capture_facts['began_at'].replace('Z', '+00:00'))
        if captured.utcoffset() is None or began.utcoffset() is None: raise ValueError('capture timestamp requires timezone')
        if began > captured: raise ValueError('capture interval is reversed')
    if total > PROOF_LIMIT: raise ValueError('whole visual proof exceeds 2 MiB')
    return proof


def verify_image(image, local):
    path = pathlib.Path(local)
    if path.stat().st_size != image['bytes'] or digest(path) != image['sha256']:
        raise ValueError('PNG hash/size mismatch')
    if png_size(path) != image['viewport']['window_size']:
        raise ValueError('PNG dimensions differ from captured window')


def snapshot(args):
    exe = pathlib.Path(os.readlink(f'/proc/{args.pid}/exe')).resolve()
    if exe != pathlib.Path(args.exe).resolve(): raise ValueError('running executable mismatch')
    window_pid = int(subprocess.check_output(['xdotool', 'getwindowpid', args.window], text=True).strip())
    if window_pid != args.pid: raise ValueError('window does not belong to actual shell PID')
    raw = subprocess.check_output(['xdotool', 'getwindowgeometry', '--shell', args.window], text=True)
    fields = dict(line.split('=', 1) for line in raw.splitlines() if '=' in line)
    if int(fields['WINDOW']) != int(args.window, 0): raise ValueError('geometry belongs to another window')
    geometry = [int(fields[key]) for key in ('X', 'Y', 'WIDTH', 'HEIGHT')]
    display = [int(n) for n in subprocess.check_output(['xdotool', 'getdisplaygeometry'], text=True).split()]
    return {'exe': {'path': str(exe), 'sha256': digest(exe)}, 'pid': args.pid,
            'window_id': str(args.window), 'geometry': geometry, 'display_size': display}


def check_geometry(geometry, display, requested):
    if display != requested: raise ValueError(f'actual display {display} differs from requested {requested}')
    if len(geometry) != 4 or len(display) != 2: raise ValueError('invalid capture geometry')
    x, y, width, height = geometry
    if width <= 0 or height <= 0: raise ValueError('empty window geometry')
    if x < 0 or y < 0 or x + width > display[0] or y + height > display[1]:
        raise ValueError(f'window {geometry} is clipped by display {display}; full-window proof rejected')


def capture(args):
    path = pathlib.Path(args.image).resolve()
    exact_path(str(path), args.lane)
    if path.exists(): raise ValueError('capture image already exists')
    before = snapshot(args)
    requested = [int(n) for n in args.size.split('x')]
    check_geometry(before['geometry'], before['display_size'], requested)
    began = datetime.datetime.now(datetime.timezone.utc).isoformat()
    # Default window capture excludes frame/border; explicit switches preserve
    # that contract independently of backend defaults. Never resize the PNG.
    command = ['import', '-silent', '+frame', '+border', '-window', args.window, str(path)]
    subprocess.run(command, check=True)
    after = snapshot(args)
    if after != before: raise ValueError(f'window identity/geometry changed during capture: {before} -> {after}')
    dimensions = after['geometry'][2:]
    if png_size(path) != dimensions:
        raise ValueError(f'PNG {png_size(path)} differs from full window {after["geometry"]} on display {after["display_size"]}')
    image = {'path': str(path), 'bytes': path.stat().st_size, 'sha256': digest(path),
             'exe': after['exe'], 'pid': args.pid,
             'source_commit': args.source_commit, 'fixture': args.fixture,
             'viewport': {'display_size': after['display_size'], 'window_size': dimensions},
             'window': {'id': args.window, 'title': subprocess.check_output(['xdotool', 'getwindowname', args.window], text=True).strip()},
             'capture': {'backend': 'ImageMagick import +frame +border -window',
                         'before': before, 'after': after, 'began_at': began},
             'captured_at': datetime.datetime.now(datetime.timezone.utc).isoformat()}
    manifest = pathlib.Path(args.manifest)
    proof = json.loads(manifest.read_text()) if manifest.exists() else {'schema': 1, 'kind': 'actual-window-proof', 'lane': args.lane, 'images': []}
    proof['images'].append(image)
    validate(proof, args.lane)
    manifest.write_text(json.dumps(proof, separators=(',', ':')) + '\n')


def failure(args, error):
    # Compact diagnostics belong to this failed shot, never to the success
    # manifest. They preserve actual bounds for a subsequent tooling ticket.
    facts = {'kind': 'window-capture-failure', 'source_commit': args.source_commit,
             'image': args.image, 'fixture': args.fixture, 'pid': args.pid,
             'window_id': args.window, 'error': str(error)[:4000],
             'timestamp': datetime.datetime.now(datetime.timezone.utc).isoformat()}
    try: facts['observed'] = snapshot(args)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as observation:
        facts['observation_error'] = str(observation)[:1000]
    path = pathlib.Path(args.image)
    if path.is_file():
        facts['image_bytes'] = path.stat().st_size; facts['image_sha256'] = digest(path)
        try: facts['png_size'] = png_size(path)
        except ValueError as image_error: facts['image_error'] = str(image_error)
    exact_path(str(path.resolve()), args.lane)
    diagnostic = path.with_suffix('.failure.manifest.json')
    diagnostic.write_text(json.dumps(facts, separators=(',', ':')) + '\n')
    print(json.dumps(facts, separators=(',', ':')), file=__import__('sys').stderr)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='command', required=True)
    r = sub.add_parser('capture')
    for name in ('lane', 'manifest', 'image', 'exe', 'window', 'source-commit', 'fixture', 'size'):
        r.add_argument('--' + name, required=True)
    r.add_argument('--pid', required=True, type=int)
    v = sub.add_parser('validate'); v.add_argument('manifest'); v.add_argument('lane')
    c = sub.add_parser('verify'); c.add_argument('manifest'); c.add_argument('lane'); c.add_argument('directory')
    args = parser.parse_args()
    if args.command == 'capture':
        try: capture(args)
        except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            try: failure(args, error)
            except (OSError, ValueError, KeyError, subprocess.SubprocessError) as diagnostic_error:
                print(f'capture failed: {error}; diagnostic publication failed: {diagnostic_error}', file=__import__('sys').stderr)
            raise SystemExit(3)
    else:
        proof = validate(json.loads(pathlib.Path(args.manifest).read_text()), args.lane)
        if args.command == 'verify':
            for image in proof['images']: verify_image(image, pathlib.Path(args.directory) / pathlib.PurePosixPath(image['path']).name)
        else:
            for image in proof['images']: print(f"{image['sha256']} {image['bytes']} {image['path']}")


if __name__ == '__main__':
    main()
