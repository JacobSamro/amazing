# UTF-16 Array.prototype.join

Recorded origin/dev: eb8e42d3bb8c823cb6c9f663539eb4ecb0745891.
Own paired Linux gate passed on frozen source3f2280435a21ebc58e63f7e31a6f540927c37a58; root merge qualification remains separate.

Port the complete generic join sequence from boa-dev/boa core/engine/src/builtins/array/mod.rs at 39cd11214ecb366200157dab96b3919e61e9a391, verified MIT at LICENSE-MIT. Amazing retains its own receiver, LengthOfArrayLike, property Get and ToString protocols. Separator and each element are coerced once in their observable order, then original UTF-16 units are appended. Holes/null/undefined contribute empty strings; inherited getters, proxies, length mutation and arbitrary abrupt identity remain observable. Allocation grows from actual appended units rather than trusting a potentially huge array-like length.

Retain the existing thread-local identity cycle guard and RAII cleanup exactly. Do not adopt upstream's direct self-element elision: authored self ToString must still execute. The existing reentrant identity policy contributes empty before another length/separator read; no new broader cycle or worker policy is claimed. Generic Array.toString keeps its existing join lookup/callability/fallback dispatch and receives exact join output.

One direct native-call group compares independent original unit vectors without parser/JS concatenation, and eight whole grouped controls exercise both genuine interpreted and Bytecode callable bodies: UTF-16 element/separator cross-products; generic primitive receiver; conversion order/hints/count; holes/inheritance/Proxy/mutation; LengthOfArrayLike and large first-abrupt cases; Symbol/nullish/BigInt errors; abrupt guard cleanup; self/mutual/authored cycles; Array.toString callable Proxy and native fallback. The original JS49 primitive_symbol_utf16_and_proxy_reads_preserve_keys_and_receiver fixture remains owned by Lang unchanged; its key log requires this real join prerequisite. No Reference implementation or test oracle is bundled.

Measured own qualification:4451→4460 workspace passes/17skips,4doctests both stages; six whole selectors both modes preserve32749 passingfiles/61733 passingvariants with zero losses and unchanged aggregate classification counts. All24 controllers completed normally on matched default4CPU8GB baremetal bx_bq7wyr6x. Native729 original-unit checks and eight genuine AST/Bytecode body groups pass; these nine new groups are candidate-only, with no inferred baseline result. Compact proof is array-join-utf16-current-proof.json.

Qualification scope: workspace nextest and doctests plus whole Array (including join/toString), String, Object, Proxy, Reflect and language selectors in both default and VM modes, comparing exact file and variant classifications. Full 16-shard merge gate belongs to root. Raw stays Boat; commit compact counts/gains/losses/raw hashes only.

Primary: https://tc39.es/ecma262/multipage/indexed-collections.html#sec-array.prototype.join
Upstream: https://github.com/boa-dev/boa/blob/39cd11214ecb366200157dab96b3919e61e9a391/core/engine/src/builtins/array/mod.rs
License: https://github.com/boa-dev/boa/blob/39cd11214ecb366200157dab96b3919e61e9a391/LICENSE-MIT
