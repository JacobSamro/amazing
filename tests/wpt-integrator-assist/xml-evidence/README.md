# IA-XML cascade evidence

Both before runs were executed on win-001 and collected before the fleet SSH
stand-down. Source revision aa9cb9be3ca79425bb979519c75b442646b37028 (IA-1 rebased
onto b5d1b99, including parser c4b6757; XML dispatch still absent).
WPT pin c2a2fce69e5c2509d9dc28440ec8b42532e5c293; existing cascade selection72 cases.
Each run:35 pass/24 mismatch/0 error/0 timeout/13 unsupported/0 crash.
Normalized result/pixel/detail SHA-256:
643e965ccd8783a3c1c94f395a5812c286286171ac4b0ac02de0dbe5e71d1f86.
Frozen executable target/diagnostics/ia-xml/before.exe SHA-256:
28b53f5950a99b972854ba3095476a77b5b7a91443ce40486a1d4d709a248ec1.

Own tree was refreshed with fingerprint invalidation and re-synced;220 source/input
files matched before build. before-source.json records those hashes. Baseline
captures ran under the shared REFRESH-HOLD. The hold was released only after
both completed and reports were collected. No XML behavior was changed before
these captures.

After dispatch, exact driver revision
`8a7b5c453d45629c94300add25988488601db2fe` (rebased onto `ffb143b`),
frozen executable SHA-256
`950964fe1de5296b5ba9b1a730470fe6cad0d9f58a7bf53a95da5af43c443055`.
Both 72-case runs: 38 pass/32 mismatch/0 error/0 timeout/2 unsupported/0
crash. Stable normalized result/pixel/detail SHA-256:
`8aca2064b4c6309679c4a4b1508a163c4c3be720dbc597a2eb6f59c42d7a5c6f`.
The manifest and WPT pin are unchanged. Three formerly excluded cases pass,
eight mismatch, and two remain Unsupported due to external DTD entities. All
61 unchanged classifications retain their recorded pixel hashes. This is the
named 72-case selection, not a browser-wide WPT rate.

`after-source.json` records 231 local source/input hashes checked against the
refreshed Windows lane tree. The synthetic four-case `protocol-summary.json`
proves two XML error paths, one Unsupported path, one ordinary HTML crash, and
retention of the actual pixel hash when its XML reference fails. Those fixtures
earn zero WPT credit. The first attempted protocol fixture omitted the external
entity use and was corrected before this recorded passing run. An interrupted
earlier sweep used the old private build-slot script; its reports were excluded.
The recorded after runs acquired a slot through the primary shared wrapper.
The final workspace gate on the same refreshed tree passed 1270/1270 nextest
tests (3 skipped) and 2/2 separate doctests (0 failed). `REFRESH-HOLD` was
released after those gates completed.
