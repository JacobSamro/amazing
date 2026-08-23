# Amazing Architecture

A from-scratch, zero-dependency (std-only) web browser engine. This document
records decisions that are **locked** — changing them later is a project-level
event, not a refactor.

## Component graph

```
amazing-net ──bytes──▶ amazing-html ──tree ops──▶ amazing-dom
      │                                            │
      │                              styles        │ tree mutations
      │                                 ▼          ▼
      │                             amazing-css   amazing-js  ◀── DOM/JS bridge (Phase 7)
      │                                 │
      │                                 ▼ computed style tree
      │                            amazing-layout ──layout tree──▶ amazing-paint ──pixels──▶ GUI shell
      └──image bytes──▶ amazing-media
amazing-media ──intrinsic size via PageMedia──▶ amazing-layout
amazing-media ──RGBA8 via PageMedia───────────▶ amazing-paint
amazing-media ──validated PCM WAV─────────────▶ GUI shell (WinMM adapter)
UDP sockets ◀──authenticated ICE──▶ amazing-webrtc codecs
SDP text (amazing driver) ───────────────────────────▶ amazing-webrtc signaling
UDP demux ──DTLS──▶ amazing-webrtc-crypto ◀──SRTP/SRTCP── amazing-webrtc codecs
GUI platform seam ──D3D11 device/swap-chain present──▶ Windows graphics driver
```

Dependency direction always points "downward" in the pipeline; nothing in a
lower layer may import a higher one except through the `amazing` driver binary.

## Locked decisions

### 1. Single-threaded first

The entire engine v1 runs on one thread with an explicit, cooperative event
order: *network → parse → style → layout → paint*, then JS microtask/job queue
drains at defined points (HTML event loop semantics, WHATWG §8.1).

Rationale: correctness and debuggability before throughput. Every real engine
parallelized *after* its data model was stable, not before.

### 2. Where parallelism will land later (seams reserved now)

These are noted so Phase 0 code doesn't accidentally foreclose them, not as
v1 features:

- **Style resolution** — per-subtree work-stealing over the arena once
  selector matching exists; requires the style-sharing candidate check to be
  pure (no interior mutability).
- **Layout fragmentation** — block formatting contexts partition naturally;
  inline FC does not. Parallelize BFC subtrees only.
- **Paint tiling** — rasterizer already operates on rectangular tiles of the
  pixel buffer; tiles are embarrassingly parallel.
- The DOM arena (`Vec<Node>` + u32 indices) makes all of the above possible:
  subtrees are contiguous-ish index ranges, snapshots are cheap clones, and
  there is no shared mutable `Rc` state to fight over.

### 3. Arena-allocated DOM

`amazing_dom::Dom` = flat `Vec<Node>`, references are `NodeIdx(u32)`.
No `Rc<RefCell<..>>` anywhere in the engine core. Consequences we accept:

- Node removal = freelist insertion; indices stay stable until reused.
- Borrow rules are manual (the tree builder holds `&mut Dom`; delegates use
  indices). This is deliberate — it keeps mutation auditable.

### 4. Hand-written parsers follow spec state machines

HTML tokenizer states and CSS Syntax token names are copied verbatim from the
specs. Error-recovery behavior *is* the interop contract — never invent a
"simpler" grammar.

### 5. unsafe policy

No first-party `unsafe` outside platform-boundary modules (windowing FFI,
Phase 8+). Every first-party `unsafe` block carries a comment justifying: what
invariant upholds safety, why std cannot express it. Engine crates compile with
`unsafe_code` denied (enforced via workspace lint config as crates appear).
The vetted TLS exception below may contain dependency-internal unsafe code;
its first-party adapter forbids unsafe and does not expose dependency types to
the engine cores.

### 6. TLS exception (decided at Phase 5)

Amazing uses vetted TLS implementations rather than hand-rolled TLS. Rationale
for HTTPS lives in `crates/amazing-net/src/lib.rs`; HTTPS remains unimplemented.

Phase 9 applies the exception in the isolated `amazing-webrtc-crypto` adapter.
It pins WebRTC.rs DTLS 0.12.0 and SRTP 0.17.2, plus their RustCrypto/rustls/ring
stack, while `amazing-webrtc` remains safe and `std`-only. `Cargo.lock` is
committed because exact security-provider resolution is part of the reviewed
artifact. The adapter uses a current-thread Tokio runtime, preserving the
single-threaded cooperative v1 decision. Its first milestone negotiates DTLS
1.2 with `TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256`, negotiates
`AEAD_AES_128_GCM` through `use_srtp`, pins both SHA-256 certificate
fingerprints, exports `EXTRACTOR-dtls_srtp` keying material, and delegates
SRTP/SRTCP protection and replay defense. Revisit this exception only as a
project-level security decision.

Phase 9.10 keeps that DTLS association live while media flows. A single
RFC 7983-style reader routes DTLS and SRTP/SRTCP into separate channels on the
same authenticated ICE-nominated UDP five-tuple. Routing is nonblocking:
queue saturation drops that UDP datagram instead of allowing one protocol
class to starve the other. `SecureIdentity` hides
provider certificates/keys behind a SHA-256 fingerprint, while
`SecureMediaSession` hides directional provider contexts behind four
protect/unprotect methods. ICE candidates/credentials, setup roles, and peer
fingerprints come from SDP that has passed the std-only parser/serializer,
not from provider defaults.
These are internal seams for the loopback milestone; a caller-constructible
single-endpoint handshake interface remains a Phase 9 follow-on.

Phase 9.11 promotes that seam through `SecureEndpointConfig` and
`SecureUdpEndpoint`. Callers provide only a connected `std::net::UdpSocket`,
an opaque `SecureIdentity`, parsed local/remote SDP, offerer status, and ICE
role. The endpoint owns its current-thread runtime, live DTLS association,
demultiplexer, and directional SRTP contexts. The provider remains invisible
to the caller. The two-process artifact uses this exact interface on each side.
A timeout-bounded loopback TCP control channel carries canonical SDP, including
the ICE credentials required by STUN; it never carries certificate private
keys, DTLS/exporter secrets, or SRTP keys.

## Conformance tracking

| Phase | Suite | Gate |
|-------|-------|------|
| 1 | html5ever/html5lib tokenizer + tree tests | report % |
| 2–3.5 | WPT css/* subdirs per phase | report % |
| 4 | screenshot diffs vs reference renderer | visual review |
| 6+ | Test262 | report % |

Numbers go in each phase's completion report. Estimates are forbidden.
