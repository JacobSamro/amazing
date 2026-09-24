# IPC child bootstrap readiness regression

Base: `5240d39`. The role handler table was populated after the child sent
its handshake acknowledgment. Under parallel nextest scheduling, the browser
could send Ping to an unbound interface and receive `UnknownInterface`.
Negotiation already included interface 0 at version 1 on both peers; the
failure was handler registration order, not version selection.

The child now binds its role interfaces before sending the acknowledgment.
The host treats a Ping failure as a launch failure, without retrying
`UnknownInterface`. A delayed-bind regression test exercises the ordering.
AppContainer profile names already include a fresh 128-bit random nonce for
each launch, so profile-name reuse was not the observed cause.

## Repeat evidence

- Venus-001: 50 consecutive `cargo nextest run --workspace --locked
  --no-fail-fast` cycles under the workspace runner, 24/24 runnable
  `amazing-process` tests passed per cycle (1,200/1,200 executions). Two
  Linux sandbox tests remain ignored because Venus AppArmor blocks user
  namespace UID/GID mapping. Per-cycle logs:
  `venus-001:~/amazing/out/ipc-ping-50-complete/1.log` through `50.log`;
  local summary: `/tmp/ipc8-ping-linux50-complete.log`.
- Windows win-002: pending 50-cycle result. Per-cycle logs:
  `C:\Users\win-002\out\ipc-ping-50\1.log` onward; local summary:
  `/tmp/ipc8-ping-win50.log`.

The Linux workspace command itself was not green in every cycle: unrelated
`amazing` driver tests failed with IndexedDB `Corrupt` and render errors.
`--no-fail-fast` ensured all process tests ran in every cycle; the 50/50
claim above is for `amazing-process`, not the whole workspace. Windows
workspace cycles have been green so far.
