# RC-4 current-source gate after NET-7

Dev base: `f1d0f5013a98c50918bae30d1ffec9e69187681f`.
Compiled lane HEAD: `552fd1f291bc94a5d5983ae4aef695904800b7de`.

Rebased cleanly, ran `scripts/refresh-lane-trees.sh replaced-content` only,
then re-synced all 100 owned changed files. Root lock hash prefix F4FF5727
matches on Mac/Windows. All 207 tracked Rust/Cargo inputs match. The shared
REFRESH-HOLD protects this gate. The isolated image target's first-party
artifacts are separately invalidated before building. Tests run only on win-001,
with --locked and CARGO_INCREMENTAL=0. Root and isolated metadata checks pass.
No shell/lock edits or new feature changes. Prior before and paired evidence
is preserved unchanged; NET-7 and other intervening dev work is not RC-4 credit.

Root commands: `cargo nextest run --workspace --locked` and
`cargo test --workspace --locked --doc`. Image commands use
`--manifest-path crates/amazing-image/Cargo.toml -p amazing-image` with nextest
and a separate cargo test --doc invocation. Both renderers are rebuilt.
Pinned WPT revision remains `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
The five pinned after selections and owned 64x64 render proofs repeat twice;
SVG raw matches requiring unsupported inline CSS earn zero conformance credit.

Use the existing RC-4 scripts with prefix `target/rc4-f1d0f50-`, collect with
`../../collect-rc4-rebase.ps1 -Tag f1d0f50`, then analyze with
`../../analyze-rc4-rebase.py f1d0f50`. Results and limitations follow.

Measured root nextest: **1261 passed / 0 failed / 3 skipped**, 42 binaries.
Separate root docs: **2 passed / 0 failed**, 19 targets (HTML1, JS1). Image
nextest **17 passed / 0 failed**, separate docs **0 present**. Paint **87 passed**.
These test logs were copied locally before the outage; their SHA-256 hashes are
in validation.json. Both renderer builds succeeded. The existing healthy SSH
session completed the owned proofs **9/9 twice** and all pinned pairs: scope
**26/4/1/17**, controls **21/2/1**, PNG **12/12**, SVG **12/12 raw zero credit**,
focused **24/24**, with the pipeline exiting zero. These are observed repeated
counts, **not yet verified stable hashes**: report collection was blocked by the
fleet SSH stand-down, and no new SSH/SCP was attempted after that directive.
Remote reports remain under `target/rc4-f1d0f50-*` for later collection.

Before local rebase, the shared origin/dev ref advanced through IA-1 to
`9bf12246168c7b2bebb4e402853a47baeddc6ee4`. That includes driver, natural image
sizing and WPT root-resource mapping changes, so the results above do not
validate the newer source. Fresh gates on that base remain unrun. The refresh
hold was removed only after the existing gate process ended naturally. All
previous before and verified paired evidence remains untouched. No unrelated
changes are credited to RC-4, and no final readiness is claimed during outage.

## Post-outage collection

The saved reports and executable hashes were collected before the current-source refresh. Five pinned pairs and the 9/9 owned proof agree on classifications, pixel differences, and actual/reference hashes; see `repeatability.json` and `artifacts.json`. This packet is historical compiled `552fd1f` on dev `f1d0f50`, not validation of later IA-1 or current dev.
