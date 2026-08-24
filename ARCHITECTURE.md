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

- Element nodes carry an explicit HTML, SVG, or MathML namespace; local names
  remain separate so selector/layout consumers do not parse serialized names.
- Node removal = freelist insertion; indices stay stable until reused.
- Borrow rules are manual (the tree builder holds `&mut Dom`; delegates use
  indices). This is deliberate — it keeps mutation auditable.

### 4. Hand-written parsers follow spec state machines

HTML tokenizer states and CSS Syntax token names are copied verbatim from the
specs. Error-recovery behavior *is* the interop contract — never invent a
"simpler" grammar.

### 4a. Computed values stay behind the CSS seam

`amazing_css::compute` remains the only external style-computation interface.
Cascade ranks declarations, then a private in-process module owns the
specified-to-computed transition: property metadata, supported shorthand
expansion, case-sensitive inherited custom properties, dependency-cycle
detection, `var()` substitution/fallback, invalid-at-computed-value-time
defaulting, and CSS-wide `initial`/`inherit`/`unset` behavior. Layout and paint
receive `ComputedStyle`; they never inspect custom-property graphs or perform
string substitution. Normal values remain string-backed until their individual
typed grammars land, but substitution preserves token boundaries so it cannot
manufacture a different token during reparse.

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
for HTTPS lives in `crates/amazing-net/src/lib.rs`. Phase 5.1 implements the
exception in `amazing-net-tls`: rustls 0.23.43 with its ring provider and
Mozilla roots, certificate and server-name verification always enabled, ALPN
restricted to HTTP/1.1, and bounded owned bytes returned to `amazing-net`.
Provider types never cross the adapter boundary. An unclean TLS EOF is accepted
only when HTTP `Content-Length` or chunked framing independently proves that the
body is complete.

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

### 6a. GPUI application-shell isolation

The user-directed GPUI shell lives in `amazing-shell-gpui`, outside every
engine core and is excluded from the engine workspace with its own lockfile.
It pins GPUI and `gpui_platform` to one reviewed Zed revision so
their types and Windows DirectX backend cannot drift independently. The shell
may use GPUI's third-party dependency graph; `amazing-html`, `amazing-css`,
`amazing-dom`, `amazing-layout`, `amazing-js`, `amazing-net`, `amazing-media`,
`amazing-paint`, and `amazing-webrtc` retain their std-only boundary. The
first-party shell crate forbids unsafe code. The hand-written Win32 shell stays
available as a fallback until the GPUI viewport owns the complete engine loop.
The root driver is only a process-level launcher for this boundary: a
no-argument Windows run starts a colocated GPUI binary when present, or invokes
the shell's separate locked manifest during development. `amazing gui` remains
the explicit Win32 fallback. GPUI types and dependencies never enter the engine
workspace graph.

### 6a.1. GPUI browsing-session ownership

Cookies, cache, origin storage, persisted history/bookmarks, and downloads are
embedding policy, not parser/layout state. `BrowserSession` owns them in the
GPUI shell and supplies validated request headers and bounded GET/POST bodies
to `amazing-net`. State is persisted only under the current user's local
application-data tree; download payloads go to `Downloads/Amazing`. Engine and
provider types do not enter the persisted format. Cache memory is bounded and
never persisted.

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

Phase 9.12 adds two std-only protocol modules. `ConsentFreshness` owns the
RFC 7675 4–6 second schedule, unique outstanding transaction history, and
30-second irreversible expiry decision anchored at ICE success; the secure
endpoint refuses RTP/RTCP after expiry. The adapter alone generates
transaction IDs with the vetted system RNG and moves authenticated STUN through
the live demux. The RTCP codec now emits/parses RFC 3550 compound Sender or
Receiver Reports followed by mandatory SDES/CNAME, which SRTCP protects
unchanged.

Phase 9.13 adds deterministic RFC transport/statistics state to the std-only
core: `StunRetransmission` owns exponential UDP deadlines and terminal
success/failure/cancellation; role-conflict resolution is a pure tiebreaker
decision; `RtpReceptionStats` converts observed sequence/timestamp/arrival
state into RR loss, extended sequence, jitter, LSR, and DLSR fields. The
driver's probes use these interfaces, not duplicate arithmetic.

Phase 9.15 places the playout seam in `amazing-webrtc::jitter`. A bounded
`RtpJitterBuffer` hides 16-bit sequence extension, wrap-aware reordering,
fixed RTP-clock deadlines, duplicate/late/overflow accounting, and explicit
gap emission behind `push`/`pop_ready`/`stats`. `RtpAudioDecoder` is a real
codec seam with two std-only adapters: network-order L16 PCM and G.711 PCMU.
The platform and secure-transport crates do not learn codec implementation
details.

Phase 9.16 connects that seam after SRTP authentication/decryption. The
two-process driver parses decrypted RTP, passes it through per-source jitter
state, and invokes a negotiated decoder adapter before building reception
statistics and RTCP. Secure transport still handles bytes only; codec and
playout policy remain in the std-only protocol module and caller.

Phase 9.17 makes SDP codec mappings authoritative. `NegotiatedAudioCodecs`
intersects offer/answer `rtpmap` entries without renumbering, rejects changed
clock/channel parameters, and constructs both jitter configuration and a
decoder adapter for an actually negotiated payload type. Secure endpoints
advertise the L16/PCMU set they send; unnegotiated RTP cannot enter playout.

Phase 9.18 keeps adaptive delay inside the jitter module. An optional bounded
controller observes RTP-clock transit variation, applies an RFC-style 1/16
EWMA, raises delay immediately under jitter, decays one millisecond per stable
observation, and reschedules queued packets against the current target. Callers
still drive monotonic time and choose fixed or adaptive construction.

Phase 9.19 makes loss a playout event rather than a missing counter only. The
jitter module estimates a missing packet's RTP timestamp and duration from the
surrounding sequence, advances its output clock, and asks the negotiated codec
adapter for concealment. The secure two-process artifact forces that path and
correlates its one gap with RTCP cumulative loss.

### 6b. Compressed-codec exception (Phase 9.20)

The std-only protocol core does not implement Opus. `amazing-webrtc-codecs` is
an isolated adapter with `#![forbid(unsafe_code)]` that satisfies the core
`RtpAudioDecoder` seam. It pins `opus2` 0.4.0 to the bundled static upstream
libopus backend; dependency-internal C/FFI stays outside every engine core.
The alternative pure-Rust `mousiki` 0.2.1 backend was evaluated and rejected
because a valid 20 ms mono frame panicked in CELT band processing. Bundled
libopus requires CMake and the existing Visual Studio C++ tools on Windows.

Phase 9.21 puts video RTP assembly back in the std-only core rather than the
codec adapter. `VideoFrameAssembler` consumes jitter output, bounds packets and
bytes, concatenates one timestamp through the RTP marker, and discards a
damaged frame until its marker. Encoded frames can later cross a VP8/H.264
decoder seam without teaching transport about codec payload syntax.

Phase 9.22 extends the isolated codec adapter with stateful VP8 decoding. It
pins vcpkg `2026.07.29` and libvpx 1.16.0 static `/MD`; `media-codec-vpx`
contains all FFI/unsafe and first-party Rust continues to forbid unsafe. The
adapter accepts complete encoded frames and returns measured I420 geometry and
pixel data. Encoding remains a separate gate because the compatible maintained
wrapper intentionally ships decoder-only.

Phase 9.23 adds a second isolated video adapter using `openh264` 0.9.8 with its
bundled Cisco source. The safe high-level wrapper encodes and decodes key/delta
access units; dependency-internal C/FFI remains outside first-party Rust. This
provides a complete browser-compatible video codec path while VP8 encoding is
still tracked separately.

Phase 9.24 replaces the flat SDP media fields with ordered
`SdpMediaSection` values. Session identity/ICE/DTLS remains shared while each
audio or video section owns its connection, port, candidates, payload list,
and rtpmap entries. Existing ICE selects the primary section until explicit
BUNDLE/mid demultiplexing lands; no caller silently treats video as audio.

Phase 9.25 makes that shared transport explicit: session BUNDLE lists unique
per-section mids, every bundled section requires rtcp-mux, and negotiated
payload types resolve to exactly one media section. Duplicate mids, missing
members, and cross-media payload collisions fail before RTP dispatch.

Phase 9.26 routes bundled video on the existing authenticated SRTP association.
Payload type resolves video mid 1, then the packet crosses jitter ordering,
bounded frame assembly, and the isolated H.264 decoder. The transport adapter
still handles authenticated bytes only; decoded I420 remains caller-owned.

Phase 9.27 converts decoded I420 into an owned opaque RGBA surface inside the
isolated codec adapter. The GPUI shell consumes only that safe pixel value and
creates its native BGRA render image. Codec/FFI types do not cross the shell
seam. The root and standalone manifests declare Rust 1.89, matching the highest
transitive media dependency MSRV instead of the stale 1.85 claim.

Phase 9.28 adds a codec-agnostic `VideoPlayoutQueue` in the std-only core. It
extends wrapping 32-bit RTP timestamps, orders bounded decoded frames, computes
due times from the negotiated clock, and counts duplicate/late/overflow state.
GPUI owns only the timer and current render-image index; it never learns RTP or
codec state.

Phase 9.29 runs that queue on authenticated media: six H.264 key/delta access
units share one SRTP context, preserve 90 kHz timestamps through jitter/frame
assembly, decode with persistent reference state, and leave the queue in due
order. This proves continuous secure media independently of the UI process.

Phase 9.30 exports the decoded queue through a narrow localhost seam. The
secure child writes bounded `AMVF` records containing index, dimensions, RTP
timestamp, byte length, and owned RGBA. The consumer receives no DTLS, SRTP,
codec, or arena types; EOF terminates the six-frame stream.

Phase 9.31 consumes that seam in GPUI. A named background thread owns the
listener/driver lifecycle and sends validated raw RGBA events through an MPSC
channel. The GPUI timer creates render images on the UI context, replaces the
synthetic frame set on first secure media, and continues cycling without
blocking browser input.

Phase 9.32 uses RTCP Sender Reports as the A/V clock seam. `MediaClockAnchor`
maps wrapping RTP timestamps onto signed nanosecond NTP time; `AvSyncTimeline`
classifies video as hold/present/drop against audio master time. The secure
sender provides separate audio/video anchors on the bundled association.

Phase 9.33 carries original video RTP timestamps through AMVF into GPUI. The UI
computes each next delay from the 90 kHz delta, clamps 16–5000 ms, and uses the
last observed interval for loop wrap. The asynchronous timer is media-clock
driven rather than hard-coded, while audio-master correction remains upstream.

Phase 9.34 versions that process seam to `AMV2`. Each record adds an audio-master
relative `presentation_ms` derived by mapping the decoded video RTP timestamp
through its RTCP Sender Report anchor and subtracting the audio anchor origin.
The reader requires a zero first presentation time and strictly increasing
successors. GPUI schedules directly from those bounded presentation deltas, so
the RTCP A/V timeline—not knowledge of a 90 kHz video clock—now crosses the
process boundary.

Phase 9.35 moves deadline ownership into a pure `VideoPlaybackClock` module.
The clock accepts a validated looping presentation timeline plus monotonic host
time and returns the frame due now, cumulative late drops, and the next bounded
absolute deadline. A late UI wake jumps directly to the due frame and counts
intervening frames once; rendering time therefore cannot accumulate as cadence
drift across later frames.

Phase 9.36 makes AMV2 delivery progressive. When a frame sink exists, the secure
child waits only the remaining duration to each RTCP-derived presentation time
before writing that record. GPUI begins an unsealed playback timeline at the
first record, appends later times without looping, polls a missing next deadline
at 50 ms, and seals the completed six-frame timeline at EOF. Transport pacing
and UI presentation now share the same audio-master-relative time values.

Phase 9.37 moves pacing upstream to authenticated media. The parent encodes its
fixture access units, sends audio/video RTCP Sender Reports first, then transmits
each SRTP video packet on its 500 ms presentation boundary. The child constructs
the A/V timeline before video arrives and processes each packet through the 30 ms
jitter deadline, frame assembly, persistent H.264 decode, decoded playout, and
AMV2 write before receiving its successor. The non-UI secure-media command keeps
its fast unpaced path through the same implementation.

Phase 9.38 bounds the final background-thread/UI seam with a synchronous channel
of three events. A full queue blocks the AMV2 reader, which stops consuming TCP
and propagates backpressure to the child writer and its existing timeout rather
than accumulating decoded RGBA indefinitely. GPUI drains in order on its timer;
the current two-frame-per-second secure path completes losslessly with the bound.

Phase 9.39 adds bounded video-loss recovery. The std-only RTCP core encodes and
parses the exact RFC 4585 PSFB/FMT=1 Picture Loss Indication. A damaged secure
delta produces no decoded picture, so the child sends PLI over SRTCP; the parent
validates its sender/media SSRCs and calls the isolated encoder's safe
`force_intra_frame` interface. The next access unit is an IDR, restoring the same
decoder and incremental presentation path after one explicit timeline gap.

Phase 9.40 moves the fault to transport ordering. The encoder advances through
one delta whose SRTP datagram is deliberately omitted; receipt of the following
sequence makes `RtpJitterBuffer` emit exactly one gap. Frame assembly discards
that dependent delta and triggers the authenticated PLI. A forced IDR on the next
sequence restores decode, so the native status can distinguish one RTP loss and
one recovery without treating damaged codec bytes as network loss.

Phase 9.41 makes the audio device the UI clock. AMA3 prepends bounded decoded
48 kHz PCM to the AMV3 video stream. The Windows-only `audio_output` adapter owns
WinMM `waveOut` handle/header/buffer state on a named thread and publishes only
an atomic consumed-sample cursor. All unsafe calls and lifetime justifications
remain in that module; the crate root denies unsafe elsewhere. GPUI samples the
device cursor after event polling and feeds it to `VideoPlaybackClock`, whose
existing absolute catch-up policy now follows audible progress.

Phase 9.42 adds authenticated TURN UDP allocation and relaying behind one
`run_turn_udp_probe` interface. The std-only module owns legacy long-term-key
MD5, 401 REALM/NONCE challenge state, MESSAGE-INTEGRITY/FINGERPRINT validation,
Allocate, CreatePermission, Send/Data indications, and Refresh deletion. Its
network adapter binds distinct control, relay, client, and peer UDP sockets and
proves payloads in both directions through the allocated relay address.

Phase 9.43 integrates that allocation into ICE. `IceCandidateKind` distinguishes
host, server-reflexive, peer-reflexive, and relay candidates; candidates retain
their related transport address and use RFC-style type/local/component priority.
SDP parses and emits `typ relay raddr ... rport ...`. After TURN payloads succeed
in both directions, the relay/peer pair crosses the ordinary checklist interface
and is marked succeeded plus nominated, completing component 1.

Phase 9.44 ingests the observed Chromium offer subset. Candidate parsing accepts
bounded extension pairs, skips unresolved mDNS host candidates, and retains
literal relay candidates with related addresses. Media sections carry explicit
sendrecv/sendonly/recvonly/inactive direction and invert offer direction for an
answer. Opus and H.264 selection now resolves the common rtpmap dynamically, so
secure media no longer assumes payload types 111 and 121.

Phase 9.45 models order-independent codec format parameters and RTCP feedback at
the media-section seam. Chromium answer synthesis keeps offered Opus/PCMU IDs,
selects constrained-baseline H.264 packetization-mode 1 at its offered ID, copies
that exact RFC 6184 configuration point, and advertises only implemented PLI
feedback. Local ICE identity/candidates and inverted media directions produce a
canonical answer that parses back identically.

Phase 9.46 implements RFC 6184 packetization-mode 1. The packetizer splits
Annex-B access units into single-NAL RTP payloads or consecutive FU-A fragments
under a caller MTU, preserving F/NRI/type and marking only the access unit's last
packet. The stateful depacketizer validates sequence/timestamp/FU state and byte
caps, reconstructs normalized Annex-B, drops damage on gaps, and resumes at a
later complete access unit.

Phase 9.47 closes the external-browser loop. A bounded local HTTP module serves
the Chromium offer page, returns the synthesized SDP answer, adopts a connected
UDP socket as controlled ICE, completes DTLS-SRTP with the browser, and sends 45
timed OpenH264 frames through RFC 6184. The page reports only after five decoded
video callbacks and a canvas checksum, producing cross-implementation visual and
terminal evidence from the same session.

## Conformance tracking

| Phase | Suite | Gate |
|-------|-------|------|
| 1 | html5lib tree tests migrated to WPT parsing resources | report discovered/pass/fail/skip + % |
| 2–3.5 | actual named WPT css/* reftests plus curated diagnostics | report both scopes separately |
| 4 | screenshot diffs vs reference renderer | visual review |
| 6+ | Test262 | report % |

Numbers go in each phase's completion report. Estimates are forbidden.
