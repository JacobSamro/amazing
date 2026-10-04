# Template sites and native tagged templates

Recorded published base: `7b5a62c10ea9d2588356d83be982a8792eba639c`, including qualified corrected Super. This independent ticket contains the complete template-site/cache and native tagged-template prerequisite, without Source47, UTF16 frontend, or eval-declaration changes.

Each parse allocates an opaque site token. AST cloning retains it while semantic AST equality ignores it. The interpreter owns a strong template-object registry keyed by actual realm and site. Cached values are cloned before allocation or callbacks; the existing frozen cooked/raw UTF16 construction remains shared by AST and VM. This adds no collector or cross-thread AST ownership claim.

The compiler roots the captured tag and receiver, then obtains the template object and evaluates every substitution in order. Values remain uncoerced across suspension and quantum interruption. Tagged eval is an ordinary indirect call. Original shared site/worker/realm/error/UTF16/class/default tests and private native-opcode/receiver/quantum tests are retained.

Port provenance: lumen `crates/lumen/src/eval.rs` at `f6c9860d44fa52249cfd647122edea184d32c863` (MIT; Copyright 2026 Lucid Softworks limited), and Boa `core/engine/src/bytecompiler/expression/mod.rs` at `39cd11214ecb366200157dab96b3919e61e9a391` (MIT; Copyright 2019 Jason Williams). Licenses at both exact commits were verified. Headers and appended notice rows accompany source.

Historical qualification of the complete area was recorded by `6d5d55b4d8497ae037d3f0ac440d0db34a6e033c` on its older fixed source: 4,362 workspace passes, 17 skips, 4 doctests, and 16 paired selected-corpus reports with no lost files or variants. These historical counts do not qualify this new published-base package. Fresh supported-script qualification on the recorded base completed: BEFORE 4,495 passes/13 new-template failures / 17 skips; AFTER 4,508 passes/0 failures / 17 skips, and 4 doctests on both. All 28 paired reports across 14 whole directories in default and VM modes retained identical PASS file/variant sets: 35,587 files and 67,264 variants per mode. Raw output stays on Boat; exact receipts and source/host identities are in [compact evidence](evidence/template-sites.compact.json).

This source is independently composed on the recorded published base. All sixteen original groups are preserved. Native and selected-corpus qualification is complete. Root combined-head and full 16-shard gate remain pending; no full-Test262 claim is made. No private controller was used.
