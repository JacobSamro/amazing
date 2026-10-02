# Document encoding URL queries and hyperlink components

Fixed base `2cf369bc`; NET43 helper is `b2c6c93f` (upstream lane `a94e4587`).
HTML a/area expose all URL components. Reflected URLs resolve against their
owner document and first base href, using its character encoding for queries.
Path and fragment remain UTF-8; component search setters use ordinary UTF-8.
No parser/codec algorithms or upstream code were ported here.

The WPT runner preserves verified upstream bytes through encoding detection.
Existing HTML decoding supplies parser text and matching document metadata;
explicit BOM/meta overrides the runner's preserved UTF-8 fallback. This fixes
an execution limitation, independently of installing hyperlink APIs. The
intermediate API-only candidate passed127/451; the remaining324 used incorrect
UTF-8 document metadata. No WPT fixture was edited.

Boat native `/tmp` source and target, final gate log in MAIN scratch
`.agents/dom-bindings/hyperlink-decoded-final.log`: workspace nextest
**3523 passed /17 skipped**, workspace doctests **4/4**. Full314-file dom/nodes:
**12028 pass /951 fail /6 subtest errors**, zero gained/lost passing assertions
by multiplicity against the previous ticket's committed raw after.json.
File classifications183pass82fail17timeout6error26unsupported; one timeout
recovery versus the previous ticket is uncredited to the URL changes.

Complete legacy encoding selection **0→451 pass**, **451→0 fail**: Big513,
Big5ASCII123, GBK49, GB18030254, ISO2022JP12. Unsupported-encoding XHR **14/14**
unchanged. Full before/after assertions record expected and actual query output.
Fixture provenance records raw SHA256 at pinned WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`; source hashes identify the gated
candidate. Native tests cover declared legacy bytes, BOM precedence, UTF-8
fallback, owner document encodings, conversion side effects, and component
getter/setter behavior.

Reproduce via MAIN `scripts/linux-run.sh dom-bindings --host boat -- '<batch>'`:
copy source and target to native `/tmp`, preserve corpora symlink, then
`bash scripts/ensure-wpt.sh && cargo build -p amazing --release --locked &&
cargo build -p amazing-net --example wpt_shim --locked &&
cargo nextest run --workspace --locked --no-fail-fast &&
cargo test --workspace --locked --doc`, full dom/nodes using
`evidence/dom-b-audit/run-corpus.py --reuse-binary --directory dom/nodes`, and
the six files listed in before.jsonl against the existing wpt_shim.
