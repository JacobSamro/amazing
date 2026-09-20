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
these captures. After evidence is pending; do not interpret these as after results.
