# amazing-ipc-platform

ADR 0007's isolated OS mapping and handle-transfer adapter. First-party unsafe
code exists only in `src/linux.rs` and `src/windows.rs`; each unsafe block states
its invariant. The public API owns handles and yields immutable frame views.

Pinned external dependencies: `libc = 0.2.189` on Linux and
`windows-sys = 0.61.2` on Windows, with only Foundation, Security, Memory,
Threading, and Cryptography Win32 features. The rest of the browser engine
remains std-only.

On Linux the renderer creates a sealed memfd, removes its writable view, and
passes the descriptor on demand over a private `SOCK_SEQPACKET` socket with `SCM_RIGHTS`.
The browser checks the nonce, the sender's kernel PID, the exact file size, and
all immutability seals before mapping read-only. On Windows the renderer writes
an unnamed mapping, unmaps it, closes the writable handle, and retains only a
read-only handle. The browser duplicates that handle from the launcher-owned
child process with `FILE_MAP_READ`, then maps it read-only. A nonce is verified
in the stdio bootstrap on both platforms. No page-supplied name or arbitrary
browser-local handle is accepted.

`FrameReady` carries metadata only. The browser asks for the newest selected
generation with `AcquireFrame`, and the renderer sends only that handle; an old
request receives `Superseded`. A receipt acknowledgement limits the side
channel to one handle in flight per renderer. The renderer budgets at most
three outstanding frames and 256 MiB of mappings. The browser separately
charges each live view to the same limit and releases its charge on drop. IPC-2 byte frames remain available as the fallback and
parity oracle. macOS requires its own reviewed adapter and is not enabled.
