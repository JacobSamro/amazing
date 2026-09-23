# IPC-5 Linux renderer syscall inventory

Observed on venus-001, x86_64, Rust 1.97.1, before sandbox installation. `strace -ff` ran the existing IPC-3 `renderer_network_frame_parity_and_input_events` and `shared_frames_match_byte_fallback_at_multiple_viewports` tests from the lane target tree. Both passed. Renderer child main PIDs were 3784028 and 3784719; each created IPC worker threads. This trace is a starting inventory, not evidence that the new sandbox profile runs successfully.

| Phase | Observed calls |
|---|---|
| ELF/child bootstrap and Rust startup | `execve`, `access`, `openat`, `pread64`, `arch_prctl`, `set_tid_address`, `dup2`, `mmap`, `mprotect`, `munmap`, `brk`, `fstat`, `close`, `read`, `getrandom`, `rt_sigaction`, `rt_sigprocmask`, `prlimit64`, `rseq` |
| IPC-3 renderer steady state in traced fixtures | `read`, `write`, `poll`, `futex`, `clone3` (thread creation), `gettid`, `getsockopt`, `fcntl`, `lseek`, `madvise`, `mmap`, `mprotect`, `munmap`, `clock_nanosleep`, `sched_getaffinity`, `set_robust_list`, `sigaltstack` |
| Shared-frame operations | `memfd_create`, `ftruncate`, `fcntl` seals, `sendmsg`/`recvmsg` on the inherited Unix frame socket, mapping calls. These appeared in worker-thread trace files rather than the renderer main-thread file. |

The profile installs after ELF loading and the authenticated bootstrap but before IPC worker threads and page input. It explicitly denies `openat`, `socket`, `connect`, `execve`, namespace creation and process-style `clone`. `clone3` returns `ENOSYS` so the runtime can fall back to a thread-only `clone`, which is filtered by `CLONE_THREAD`. The steady-state allowlist is in `crates/amazing-ipc-platform/src/linux_sandbox.rs`; it may need tightening or a reviewed addition after the first permitted host run.

Both venus-001 and venus-003 currently reject `unshare -Urn true` while writing `/proc/self/uid_map` (`Operation not permitted`) under Ubuntu AppArmor's unprivileged-userns restriction. The launcher must fail closed there. The deny and frame-parity tests are ignored with that reason until a narrow host profile permits the sandbox test binary. A successful unsandboxed trace or fail-closed result earns no sandbox denial/parity claim.

IPC-5 contract gate on this base: Linux `amazing-process` 19 passed, 2 ignored; Linux workspace nextest and doctests passed. The final win-002 gate passed `amazing-process` 18/18 and workspace 1,954/1,954 with 5 skipped; workspace doctests 2/2. The Linux sandbox deny and frame-parity tests remain unrun pending the narrow AppArmor host profile. These numbers prove the unsandboxed V2 contract and fail-closed launcher path, not sandbox efficacy.
