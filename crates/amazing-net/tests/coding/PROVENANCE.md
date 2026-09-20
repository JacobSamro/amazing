# NET-4 independent encoder vectors

Generated with Python stdlib zlib 1.2.11 (runtime 1.2.12)
using `python3 crates/amazing-net/tests/coding/generate.py`.
This generates data only; all decoder tests run on win-001.

57 deterministic vectors: empty/text/all-byte/repeated/dynamic/window
payloads, stored/fixed/default compression, raw/zlib/gzip framing, plus explicit
sync-flush multiblock history. The binary carries expected uncompressed bytes;
no zlib dependency is needed to execute the Rust tests. No external corpus or
conformance percentage is claimed. Generated fixture data is original.
