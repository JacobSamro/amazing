# IPC-8 utility and storage process evidence

Status: gated on local `dev` base `5658b688b948675aae2f6c65d8977592a5f328a4`,
which contains merged NET-28 `461fc0a`. Earlier provisional builds overlaid
the exact NET-28 commit `1e5767a` (`amazing-fetch` source only) onto the IPC
lane build trees. The final gate used the merged source without an overlay.
The Mac lane worktree did not edit net-owned files.

## Behavior covered

- Utility images use the public `amazing_media::decode_image` API (PPM/BMP)
  or `amazing_svg::decode` for SVG. WAV samples use
  `amazing_media::decode_wav`; web fonts use the public
  `amazing_text_provider::decode_web_font_to_sfnt` API. Tests compare decoded
  bytes and metadata with those in-process APIs.
- Decoder input is bounded by the IPC frame limit; decoded output is capped
  at 64 MiB and handed to the browser through an immutable ADR 0007 mapping.
  Acquisition, receipt, and release are generation checked. The browser's
  `decode_untrusted` entry requires a sandboxed Utility child and kills that
  child on decode or mapping failure.
- The Windows Utility AppContainer denied private-file open, loopback
  connect, child spawn, and browser-process handle access while successfully
  returning a shared decoded image. A malformed decode killed only that
  utility child. A separate utility survived a peer crash.
- Storage requests carry browser-minted origin/top-level-site/bucket keys.
  Process results match direct `DiskStore` operations. Tests cover partition
  separation, quota rejection, recovery of an acknowledged write after a
  child kill without checkpoint, and persistence of a later delete.
- V2 NetworkSession carries NET-28 redirect URL lists and tainting, offers
  browser-issued same-epoch manual redirect restart, and transports trusted
  bounded internal bodies for image/font/style/script subresources. Tests
  exercise 302→200 navigation and no-CORS CSS body delivery.

## Provisional runs

| Host | Selection | Result |
| --- | --- | --- |
| venus-001 | amazing-process focused utility/storage/network/process nextest | 19/19 passed, 1 Linux sandbox test skipped |
| win-002 | same focused nextest | 19/19 passed, 0 skipped |
| win-002 | workspace nextest | 2,126/2,126 passed, 5 skipped |
| win-002 | workspace doctests | 2/2 passed |
| venus-001 | `ipc-fuzz --target decoders --fuzz-seconds 600` | 1,266,340,130 cases, 45 targets, 0 panics |
| venus-001 | `ipc-fuzz --target dispatchers --fuzz-seconds 600` | 1,369,202,285 cases, 10 targets, 0 panics |

Logs: `/tmp/ipc8-net28-linux-provisional.log`,
`/tmp/ipc8-net28-win-provisional.log`,
`/tmp/ipc8-net28-win-workspace-provisional.log`, and
`/tmp/ipc8-net28-fuzz.log` on the Mac. The Venus AppArmor host still denies
unprivileged user namespace UID/GID mapping, so the Linux sandbox denial
test remains ignored there; the Linux fail-closed launch test passed.

After strengthening the storage recovery test and asserting the no-CORS CSS
response type, the focused suite passed again on venus-001 (19/19, one sandbox
skip) and win-002 (19/19, no skips). Process doctest commands passed on both
hosts (zero doctests in this crate). Logs: `/tmp/ipc8-net28-linux-focused-latest.log`
and `/tmp/ipc8-net28-win-focused-latest.log`.

## Final merged-base gate

| Host | Selection | Result |
| --- | --- | --- |
| win-002 | amazing-process nextest | 32/32 passed, 0 skipped |
| win-002 | amazing-process doctests | command passed, 0 doctests |
| win-002 | workspace nextest | 2,252/2,252 passed, 5 skipped |
| win-002 | workspace doctests | 2/2 passed |

The lane rebased cleanly onto `5658b68` before the final gate. The Windows
build tree was extracted afresh from that worktree; no NET-28 overlay or stale
pre-refresh lane files were used. Log: `/tmp/ipc8-merged-win-gate.log`.
