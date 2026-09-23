# ADR 0007 — Shared-memory frame transport

Status: accepted (2026-09-24)
Owner: IPC lane. Depends on ADR 0005 and IPC-2's byte-frame parity gate.

## Context

IPC-2 returns RGBA frames as bounded copied bytes through stdio. This proves process separation but copies every frame at least twice and caps each frame at the 8 MiB IPC message limit. ADR 0005 keeps `amazing-ipc` safe and std-only and reserves OS handle duplication for a separately reviewed platform layer. A shared-memory path must preserve the same renderer/browser ownership and frame validation while introducing platform-specific unsafe code.

## Proposed decision

- Add an isolated `amazing-ipc-platform` adapter. Its public API returns owned safe `FrameHandle` values and read-only `FrameView` values; OS mapping, duplication, and all unsafe code stay inside audited Windows and Unix modules. `amazing-ipc` and `amazing-process` remain `#![forbid(unsafe_code)]`.
- Keep stdio for typed control messages. A platform side channel transfers OS handles: Windows uses `DuplicateHandle` into the known browser process; Unix uses a private `AF_UNIX` socket with `SCM_RIGHTS`. The launcher authenticates the side channel using a random bootstrap nonce and the expected child identity. No shared object name is accepted from page content.
- One frame allocation is immutable after publication. The renderer writes a fresh mapping, seals its writable view, transfers a read-only browser handle, then sends `FrameReady { generation, width, height, stride, format, byte_len, handle_id }` over typed IPC. The browser validates generation, exact `stride * height == byte_len`, RGBA8 format, maximum dimensions and per-tab memory budget before mapping. A handle ID is scoped to one pipe and cannot be reused while outstanding.
- The browser sends an explicit release acknowledgement after presentation. Both sides close every handle on release, timeout, disconnect, process crash, or navigation replacement. Backpressure caps outstanding frames per renderer at three; a full queue drops an unpublished newer frame rather than allocating without bound. The byte-message path remains a feature-gated fallback and parity oracle.
- Sandboxing later grants renderer creation of frame mappings and transfer only to its parent browser. Network and utility roles receive no frame-handle capability.

## Alternatives

- Raw RGBA over stdio remains the fallback but has repeated copies and an 8 MiB cap.
- A persistent mutable ring needs atomics and reader/writer synchronization across processes. Immutable one-shot mappings make lifetime, visibility, and crash recovery easier to audit first.
- Named mappings avoid handle transfer but create a global naming and access-control surface. The proposal requires transferred capabilities instead.

## Required gate before adoption

1. Review unsafe blocks, handle ownership, access rights, bootstrap authentication, and sandbox policy on each target platform.
2. Fuzz all control metadata: truncated, forged, overflowed, stale-generation, duplicate and oversized handles must fail before mapping.
3. Compare shared-memory frames byte-for-byte with IPC-2 byte frames on pinned fixtures. Measure copy count and frame time at several viewport sizes; retain the byte fallback until parity is stable.
4. Kill renderer and browser at each transfer phase and assert no leaked handles or unbounded mapped bytes. Include Windows and Linux gates; macOS support requires its own reviewed adapter.

## Decisions taken at acceptance

- The isolated `crates/amazing-ipc-platform` crate is approved and owned by the `ipc`
  lane. Its scoped unsafe exception covers only its Windows and Unix modules. Its only
  permitted external dependencies are `windows-sys` (Windows, the minimal feature set)
  and `libc` (Unix), with exact pinned versions recorded in the crate README. Every
  unsafe block carries a SAFETY comment naming the invariant it relies on.
- The mapped-byte limit is 256 MiB per renderer process, with at most three outstanding
  frames.
- Under pressure, the **oldest unpresented** frame is replaced (latest wins), because the
  browser only ever presents the newest frame. An unpublished frame is never allocated
  past the limit.

## Amendment (2026-09-24): demand-driven handle transfer

The IPC-3 unsafe review found that with eager transfer on Unix, a superseded frame's
descriptor can remain queued in the kernel socket after the renderer replaces the frame,
so the 256 MiB bound could not be proven under sustained backpressure. Handle transfer is
therefore **demand-driven**: `FrameReady` carries only metadata, the browser sends
`AcquireFrame { generation }` for the one frame it will present, and the renderer transfers
that handle alone, or replies `Superseded` if a newer generation exists. At most one handle
per renderer is ever in flight. Everything else in this ADR stands. This costs one round
trip per presented frame, which is acceptable because presentation already waits for vsync.

