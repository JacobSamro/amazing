"""Generate independent encoder fixtures; does not run the Rust decoder.
Python stdlib zlib encoder, deterministic payloads; regenerate on a reviewed
zlib version and record the version in PROVENANCE.md. Binary format is
u32-le count, then repeated u32-le lengths and bytes: name, plain, encoded.
"""
from pathlib import Path
import random
import struct
import zlib

root = Path(__file__).parent
rng = random.Random(195119521950)
payloads = {
    'empty': b'',
    'hello': b'hello compressed world!\n',
    'alphabet': bytes(range(256)) * 4,
    'overlap': b'A' * 100000,
    'dynamic': (b'The bounded browser decoder follows canonical Huffman codes.\n' * 1500),
    'window': bytes(rng.randrange(256) for _ in range(32768)),
}
payloads['window'] += payloads['window'][-30000:] * 2
cases = []
for name, plain in payloads.items():
    for level, strategy, kind in [(0, zlib.Z_DEFAULT_STRATEGY, 'stored'), (6, zlib.Z_FIXED, 'fixed'), (6, zlib.Z_DEFAULT_STRATEGY, 'default')]:
        for wbits, framing in [(-15, 'raw'), (15, 'zlib'), (31, 'gzip')]:
            encoder = zlib.compressobj(level, zlib.DEFLATED, wbits, 8, strategy)
            encoded = encoder.compress(plain) + encoder.flush()
            cases.append((f'{name}-{kind}-{framing}', plain, encoded))
# Cross-block history survives sync flush; gzip concatenation is tested in Rust.
plain = payloads['dynamic']
for wbits, framing in [(-15, 'raw'), (15, 'zlib'), (31, 'gzip')]:
    encoder = zlib.compressobj(6, zlib.DEFLATED, wbits)
    encoded = encoder.compress(plain[:50000]) + encoder.flush(zlib.Z_SYNC_FLUSH)
    encoded += encoder.compress(plain[50000:]) + encoder.flush()
    cases.append((f'multiblock-{framing}', plain, encoded))
out = bytearray(struct.pack('<I', len(cases)))
for name, plain, encoded in cases:
    for data in [name.encode(), plain, encoded]:
        out += struct.pack('<I', len(data)) + data
(root / 'vectors.bin').write_bytes(out)
(root / 'PROVENANCE.md').write_text(f'''# NET-4 independent encoder vectors

Generated with Python stdlib zlib {zlib.ZLIB_VERSION} (runtime {zlib.ZLIB_RUNTIME_VERSION})
using `python3 crates/amazing-net/tests/coding/generate.py`.
This generates data only; all decoder tests run on win-001.

{len(cases)} deterministic vectors: empty/text/all-byte/repeated/dynamic/window
payloads, stored/fixed/default compression, raw/zlib/gzip framing, plus explicit
sync-flush multiblock history. The binary carries expected uncompressed bytes;
no zlib dependency is needed to execute the Rust tests. No external corpus or
conformance percentage is claimed. Generated fixture data is original.
''')
plain = payloads['hello']
gzip_encoder = zlib.compressobj(6, zlib.DEFLATED, 31)
gzipped = gzip_encoder.compress(plain) + gzip_encoder.flush()
(root / 'stacked.bin').write_bytes(zlib.compress(gzipped))
