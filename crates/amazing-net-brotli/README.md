# Bounded network Brotli decoder

This is the decode-only network exception in architecture6d / ADR0002, separate
from the font-scoped approval in4d. First-party code forbids unsafe. The public
API takes borrowed encoded bytes and a caller-selected maximum decoded length,
returning owned bytes or an owned error; dependency types stay inside the crate.

`brotli-decompressor =5.0.3` uses its safe default `std` feature. Cargo.lock pins
alloc-no-stdlib2.0.4 and alloc-stdlib0.2.4 as well as registry checksums. No
encoder, unsafe, seccomp, or ffi-api feature is enabled. Final gates inspect
workspace feature unification so another consumer cannot silently widen this.

Output length is checked before every reserve/copy. An8 KiB stack chunk probes
at most one byte beyond the remaining allowance, without growing the output.
A separate32 MiB budget bounds live decoder scratch allocation requests across
byte, word, and Huffman allocators. Strict RFC7932 decoding excludes the optional
large-window extension. Truncated streams, trailing bytes and concatenated
streams reject. Dependency panics fail closed at the adapter boundary.

The output cap is not a cap on all memory: bounded history/table scratch may
exceed a small requested output cap. Scratch accounting bounds requested payload
sizes, not allocator bookkeeping/rounding or process RSS. Input remains borrowed;
the HTTP transport separately limits encoded input. Default workspace panic
unwinding is required for the dependency-panic boundary.

Tests use18 vectors generated offline by Node22.23.2 / Google Brotli1.1.0:
empty, text, Unicode, binary,1 MiB repetition and deterministic incompressible
bytes at qualities0/5/11. A19th valid large-window vector verifies strict mode
rejection. `tests/vectors/pin.json` records hashes; the generator is retained.
These are independent encoder vectors, not an encoder dependency. Tests also
cover exact/zero/one-below limits, small-vector prefix truncations, malformed and
trailing bytes, output capacity unchanged on rejection, scratch overflow and
aggregate limits/release, and a compression bomb with a16-byte output allowance.

HTTP integration lives in amazing-net: every coding layer receives the same
caller ceiling; output-limit failures map to ResponseTooLarge, truncation to
IncompleteBody, and other errors fail as malformed responses. Native loopback,
chunked, stacked gzip+br and TLS-EOF framing tests exercise that route.
