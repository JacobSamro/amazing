#!/usr/bin/env python3
"""Generate the compact IANA TZif transition table for amazing-js.

Input is the exact IANA tzdata2026d.tar.gz archive. The source hash is checked
before extraction. zic compiles its rules over 1800-2500; this generator then
keeps only UTC transition seconds and offsets. Aliases share their target data.
Run on Linux with zic installed; no system zoneinfo database is read.
"""
import argparse
import hashlib
import os
from pathlib import Path
import struct
import subprocess
import tarfile
import tempfile

VERSION = '2026d'
SHA256 = '0cb2aa8e333c3dc049badc42a0c61f21987b8cd44e107fa900bad764aacc7767'
SOURCES = ('africa', 'antarctica', 'asia', 'australasia', 'europe',
           'northamerica', 'southamerica', 'etcetera', 'backward')


def tzif(path):
    data = path.read_bytes()
    assert data[:4] == b'TZif' and data[4:5] in (b'2', b'3', b'4')

    def header(offset):
        return struct.unpack_from('>6I', data, offset + 20)

    def size(counts, width):
        gmt, std, leap, times, types, chars = counts
        return times * (width + 1) + types * 6 + chars + leap * (width + 4) + std + gmt

    first = header(0)
    second_at = 44 + size(first, 4)
    assert data[second_at:second_at + 4] == b'TZif'
    gmt, std, leap, time_count, type_count, chars = header(second_at)
    offset = second_at + 44
    times = struct.unpack_from(f'>{time_count}q', data, offset)
    offset += 8 * time_count
    indices = data[offset:offset + time_count]
    offset += time_count
    offsets = [struct.unpack_from('>i', data, offset + index * 6)[0]
               for index in range(type_count)]
    initial = offsets[0]
    transitions = []
    for epoch, index in zip(times, indices):
        utc_offset = offsets[index]
        if not transitions or transitions[-1][1] != utc_offset:
            transitions.append((epoch, utc_offset))
    return initial, transitions


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('archive', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    assert hashlib.sha256(args.archive.read_bytes()).hexdigest() == SHA256, 'tzdata hash mismatch'
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        with tarfile.open(args.archive, 'r:gz') as archive:
            for source in SOURCES:
                archive.extract(source, path=root, filter='data')
        zones = root / 'zones'
        zones.mkdir()
        subprocess.run(['zic', '-b', 'fat', '-d', str(zones),
                        '-r', '@-5364662400/@16725225600',
                        *(str(root / source) for source in SOURCES)], check=True)
        records = []
        for path in sorted(zones.rglob('*')):
            if not path.is_file():
                continue
            name = path.relative_to(zones).as_posix()
            target = path.resolve().relative_to(zones.resolve()).as_posix()
            if target == name:
                initial, transitions = tzif(path)
            else:
                initial, transitions = 0, []
            records.append((name, target, initial, transitions))
        output = bytearray(b'ATZ1')
        output.extend(struct.pack('>H', len(records)))
        for name, target, initial, transitions in records:
            encoded, canonical = name.encode(), target.encode()
            assert len(encoded) < 256 and len(canonical) < 256
            output.extend(struct.pack('B', len(encoded)))
            output.extend(encoded)
            output.extend(struct.pack('B', len(canonical)))
            output.extend(canonical)
            output.extend(struct.pack('>iI', initial, len(transitions)))
            for epoch, offset in transitions:
                output.extend(struct.pack('>qi', epoch, offset))
        args.output.write_bytes(output)
        print(f'{VERSION}: {len(records)} names, {len(output)} bytes, sha256={hashlib.sha256(output).hexdigest()}')


if __name__ == '__main__':
    main()
