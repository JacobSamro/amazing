# NET-4 independent encoder vectors

Generated with Python stdlib zlib 1.2.11 (runtime 1.2.12)
using `python3 crates/amazing-net/tests/coding/generate.py`.
This generates data only; all decoder tests run on win-001.

57 deterministic vectors: empty/text/all-byte/repeated/dynamic/window
payloads, stored/fixed/default compression, raw/zlib/gzip framing, plus explicit
sync-flush multiblock history. The binary carries expected uncompressed bytes;
no zlib dependency is needed to execute the Rust tests. No external corpus or
conformance percentage is claimed. Generated fixture data is original.

NET-6 adds gzip-then-br.bin: Node22.23.2 / Google Brotli1.1.0 encodes the
existing hello-fixed-gzip entry from vectors.bin. Offline generator is
generate-brotli.cjs. Input vectors.bin SHA-256:
`f87f11a692a7af7b92b0daf16ac6d023a39a9a807aee7691f10cc9adcb07dd69`.
Output gzip-then-br.bin SHA-256:
`11ea8baa819e34c3a77bf38761e4b51bf35a960b4b8a73b17e34b59fd2dac101`.
