# IPC-3 shared-memory frame transport evidence

Base: `7a1e31e`. Implementation head: `1395a2f`. Accepted ADR 0007's demand-driven amendment is included in the branch. The final gate used that exact commit on win-002. `origin/dev` was `0034b44` when checked; its drift from the base did not touch the IPC lane's crates.

## Boundary review

- `amazing-ipc` and `amazing-process` forbid unsafe code. The platform crate denies unsafe code except in its Linux and Windows modules; every unsafe block states its invariant. The only new external dependencies are exact pins `libc = 0.2.189` and `windows-sys = 0.61.2` in that crate.
- The launcher creates the handle channel only for renderers. Both sides confirm a random nonce during bootstrap. Linux also checks the sender's kernel PID and immutability seals before mapping. Windows duplicates read-only access from the known child process handle, not from a renderer-supplied browser-local handle. Network and utility roles have no frame channel.
- `FrameReady` carries metadata. `AcquireFrame` transfers only the chosen generation; old requests return `Superseded`, and a second acquisition before receipt returns `Busy`. The browser validates format, dimensions, exact byte length, generation and its 256 MiB/three-frame budget before asking for a handle. Receipt acknowledgement limits the side channel to one handle in flight. Release closes the renderer handle; dropping the browser view closes its mapping and releases its budget charge.
- The copied byte path remains behind the default `byte-frame-fallback` feature. macOS is unsupported pending its separate adapter review. A future renderer sandbox must allow only the mapping and parent transfer operations already isolated here.

## Measured gates

| Gate | Result |
| --- | --- |
| Linux crate nextest at `1395a2f` | 11/11 passed; platform/process doctests 0/0 |
| Windows crate nextest at `1395a2f` | 11/11 passed; platform/process doctests 0/0 |
| Windows workspace nextest at `1395a2f` | passed, exit 0 |
| Windows workspace doctests at `1395a2f` | 2/2 passed |
| Fuzz and budget | 10,000 random metadata values, random/truncated/oversized control decodes, forged length/handle IDs, stale and duplicate generations, 256 MiB aggregate limit and three-frame cap |
| Crash phases | Renderer and browser exit at spawn, metadata, acquired-handle and mapped-view phases; Linux descriptor count returns to baseline and Windows local mapping handle count does not grow |
| Parity | Byte-for-byte on the styled image/network fixture and at three additional static viewports |

Linux debug end-to-end frame times for the pinned static fixture, from the final commit (microseconds):

| Viewport | Copied bytes | Shared mapping |
| --- | ---: | ---: |
| 96 × 64 | 14,716 | 12,479 |
| 112 × 72 | 12,499 | 12,520 |
| 128 × 96 | 15,875 | 12,351 |

These times include rendering and RPC overhead, so they are parity observations rather than isolated copy benchmarks. Inspection of the transfer paths shows at least two full byte copies in the fallback serialization/decoding path and one renderer-to-mapping copy in the shared path; the browser reads the mapping without copying it. The final conformance gate is Windows; these Linux times are diagnostic only.

The Windows upload used `win-run.sh` with legacy SCP selected by a temporary local wrapper. SFTP left the 56 MiB archive locked after upload, causing Windows `tar` to report a sharing violation before any build. Legacy SCP closed the file promptly; the runner script and repository source were unchanged by this workaround.
