# IPC-7 evidence

Base: `4acb72e` (compositor snapshot API merged). The IPC lane changed
`amazing-ipc`, `amazing-ipc-platform`, and `amazing-process`.

## Interface and process behavior

- Bootstrap advertises a min/max version for each interface. Negotiation chooses
  the highest common version, ignores unknown optional interfaces, and rejects
  malformed or duplicate declarations. `MinVersion<T>` gates optional wire
  fields by the negotiated version. Unknown methods return `UnknownMethod`
  without closing the connection. The IPC crate compatibility matrix covers
  overlap, disjoint ranges, malformed ranges, duplicates, unknown methods,
  optional fields, and a frame payload large enough for tile uploads.
- The GPU child receives a bounded, versioned compositor frame snapshot with
  passes, quads, property trees, tile lookup, and resource deltas. It validates
  the snapshot with `CompositorFrame::from_snapshot`, retains resources under a
  256 MiB cap, and presents through `ReferenceCpuPresenter`. IPC payloads remain
  bounded by the 8 MiB transport frame limit.
- GPU parity tests compare process output with the in-process CPU presenter for
  retained resources over two frames and for a frame with a separate blend pass
  and property trees.

## Leak and crash gate

- Linux: 1,000 renderer spawn, navigate, shared-frame map, kill cycles passed
  on Venus; the `/proc/self/fd` count stayed within 64 of baseline every 100
  cycles and within 16 at completion.
- win-002: the same 1,000 cycles passed in 31.861s using
  `GetProcessHandleCount`, with the same bounds.
- Existing browser-crash probes exercise the spawned, published, acquired, and
  mapped shared-frame transfer phases and assert renderer child reaping. The
  Windows workspace gate passed these tests.

## Windows gate

`scripts/win-run.sh ipc` on win-002, with locked nextest and doctests:

| Gate | Result |
| --- | --- |
| IPC/process crates | 34 passed, 0 skipped |
| IPC/process doctests | 0 passed, 0 failed |
| Workspace | 2,048 passed, 5 skipped |
| Workspace doctests | 2 passed, 0 failed |

## Fuzz gate

The std-only `ipc-fuzz` binary uses fixed-seed mutation, an in-memory corpus
of minimized novel decoder-result signatures, 38 typed wire decoders plus the
semantic compositor snapshot decoder, and eight interface dispatchers. Four
committed hex seeds are supplemented with valid typed seeds. Decoder panics
write hex inputs below `target/ipc-fuzz-crashes/` for minimization and
promotion to the committed crash corpus. An oversized 8 MiB input exercises
frame limits on every run.

| Venus target | Duration | Cases | Signals | Panics |
| --- | ---: | ---: | ---: | ---: |
| decoders | 600s | 22,809,432 | 63,413 | 0 |
| dispatchers | 600s | 57,548,715 | 2,261 | 0 |

Both runs used `scripts/linux-run.sh ipc --host venus-003` with
`cargo run -p amazing-process --bin ipc-fuzz --locked -- --target
<decoders|dispatchers> --fuzz-seconds 600`. The decoder run exercised 39
targets, the dispatcher run eight, and both exited 0. The full run log is
`/tmp/ipc7-fuzz-venus003.log` in the lane environment.
