# IPC-12: sealed-frame EPERM on Ubuntu 24.04 / kernel 6.8

Base: local `dev` at `5b923bf7`. Commit: `d1efc373`.

## Symptom

`amazing-ipc-platform`'s sealed shared-frame path (`crates/amazing-ipc-platform/src/linux.rs`)
failed with `Os error 1` (EPERM) on worker-3 (Ubuntu 24.04, kernel
6.8.0-71-generic), 11 tests affected, but passed unchanged on venus-001
(kernel 7.0). Reported failures included both an in-process unit test
(`linux::tests::sealed_mapping_rejects_forgery_and_closes_descriptors`) and
several `amazing-process` integration tests that spawn real child processes
(`process_leaks::thousand_spawn_navigate_map_kill_cycles_bound_handles`,
`utility_storage::utility_image_corpus_decodes_bounded_web_formats_and_exif_orientation`,
`decoder_crash_does_not_kill_browser_or_other_utility`, ...).

## Diagnosis

`strace -f -e trace=memfd_create,ftruncate,mmap,fcntl,dup,fstat` around the
failing unit test on worker-3 pinpointed the exact failing call:

```
memfd_create("amazing-frame", MFD_CLOEXEC|MFD_ALLOW_SEALING) = 3
ftruncate(3, 16384)                     = 0
mmap(NULL, 16384, PROT_READ|PROT_WRITE, MAP_SHARED, 3, 0) = 0x74289c8ae000
fcntl(3, F_ADD_SEALS, F_SEAL_SEAL|F_SEAL_SHRINK|F_SEAL_GROW|F_SEAL_WRITE) = 0
fcntl(4, F_GET_SEALS)                   = 0xf (all four seals present)
mmap(NULL, 16384, PROT_READ, MAP_SHARED, 4, 0) = -1 EPERM
```

`memfd_create`, `ftruncate`, the initial writable `mmap`, `munmap`, and
`F_ADD_SEALS` all succeed -- the seals really do land (`F_GET_SEALS` confirms
`0xf`). The failing call is the **read-only** `mmap(PROT_READ, MAP_SHARED, ...)`
in `FrameView::from_fd`, on a descriptor (`fd 4`, a `dup()` of the sealed
memfd) that only ever requested read access. This contradicts the documented
semantics of `F_SEAL_WRITE` (`man memfd_create`: "prevents ... creating new
**writable** shared memory-maps"), but is reproducible independent of this
crate's own code -- a minimal `ctypes`-based probe against libc directly
(`memfd_create` -> write, `munmap` -> `F_ADD_SEALS` -> `mmap(PROT_READ,
MAP_SHARED, ...)`) hits the same EPERM on worker-3, isolating the seal
combination:

| Seals added | `mmap(PROT_READ, MAP_SHARED, ...)` |
| --- | --- |
| none | OK |
| `F_SEAL_SEAL` only | OK |
| `F_SEAL_WRITE` alone | **EPERM** |
| `F_SEAL_WRITE\|SHRINK\|GROW` | **EPERM** |
| all four (production combination) | **EPERM** |

`F_SEAL_WRITE` is the seal that triggers it, on this specific kernel build;
none of `apparmor_restrict_unprivileged_userns=1`, `vm.memfd_noexec=0`, or an
AppArmor profile are involved (`aa-status` shows the test process
`unconfined`, and `dmesg`/the audit log have no `apparmor="DENIED"` entries
around the failing calls) -- this was checked explicitly since the ticket's
listed environment details raised AppArmor as a candidate.

A follow-up probe confirmed the fix: on the same kernel, sealing a memfd with
all four seals and then mapping it `PROT_READ` with **`MAP_PRIVATE`** instead
of `MAP_SHARED` succeeds and reads back the correct bytes.

## Fix

`FrameView::from_fd`'s read-only mapping now uses `MAP_PRIVATE` instead of
`MAP_SHARED`. Since all four immutability seals
(`F_SEAL_WRITE|SHRINK|GROW|SEAL`) are verified present immediately before
this call, the file's contents can never change again through any path (no
writer can exist), so a private copy-on-write mapping is observationally
identical to a shared one for pure reads, and it is accepted regardless of
this kernel's `F_SEAL_WRITE` + `MAP_SHARED` restriction. `FrameHandle::new`'s
earlier *writable* construction mapping is untouched (still `MAP_SHARED`,
required so writes actually land in the memfd's page cache before sealing).

This single change fixes all 11 originally failing tests, since every
consumer of a sealed frame -- the in-process `local_view` path and the
cross-process `HandleChannel::receive` path used throughout
`amazing-process` -- goes through the same `FrameView::from_fd`.

## Regression test

`sealed_frame_delivered_across_a_socket_maps_without_map_shared_eperm`
(`crates/amazing-ipc-platform/src/linux.rs`) exercises the cross-process
fd-passing path directly: a real `socketpair`-backed `HandleChannel` pair,
`FrameHandle::publish` -> `send` (`SCM_RIGHTS`) -> `receive` ->
`FrameView::from_fd`, with no child process needed to reproduce the kernel
behavior (the existing
`sealed_mapping_rejects_forgery_and_closes_descriptors` test already covers
the in-process `local_view` path and was the original repro).

## Gate

| Host | Kernel | `cargo nextest run --workspace --locked --no-fail-fast` | `cargo test --workspace --locked --doc` |
| --- | --- | --- | --- |
| worker-3 (`root@178.104.111.133`) | Ubuntu 24.04, 6.8.0-71-generic | **2902/2902 passed**, 10 skipped, exit 0 (198.2s) | all doctests ok |
| venus-001 | 7.0 | **2902/2902 passed**, 10 skipped, exit 0 (68.6s) | all doctests ok |

Both hosts include `process_leaks::thousand_spawn_navigate_map_kill_cycles_bound_handles`
passing, which was one of the 11 originally-EPERM-failing tests on worker-3.
Zero regressions on either host. `-p amazing-ipc-platform` (3/3) and
`-p amazing-process` (65/65, 3 skipped) were also run standalone on worker-3
during iteration, both green.
