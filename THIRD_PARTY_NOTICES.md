# Third-party notices

Amazing is written in Rust and std-only in its engine cores. Some algorithms are ported
(copied and adapted, not linked) from other open-source projects under permissive licenses,
per the porting policy in `AGENTS.md`. Each upstream is listed here with its license and the
files of ours that contain ported code; those files also carry a header comment naming the
upstream commit.

Only MIT and Apache-2.0 sources may be ported. The full license texts are reproduced below
the table, once per license.

| Upstream | Commit | License | Copyright | Our files |
|----------|--------|---------|-----------|-----------|
| [boa-dev/temporal](https://github.com/boa-dev/temporal) (temporal_rs) | `eadf8039d2577e4375493a0eb3f46a66f25616ec` | Apache-2.0 (dual MIT/Apache-2.0 upstream) | Copyright (c) 2024 Boa | `crates/amazing-js/src/builtins_temporal_arithmetic.rs` |
| [lucid-softworks/lumen](https://github.com/lucid-softworks/lumen) | `f6c9860d44fa52249cfd647122edea184d32c863` | MIT | Copyright (c) 2026 Lucid Softworks limited | `crates/amazing-js/src/interp.rs`, `crates/amazing-js/src/interp_stmt.rs`, `crates/amazing-js/src/interp_disposable.rs`, `crates/amazing-js/src/value.rs` |
| [lucid-softworks/lumen](https://github.com/lucid-softworks/lumen) | `f6c9860d44fa52249cfd647122edea184d32c863` | MIT | Copyright (c) 2026 Lucid Softworks limited | `crates/amazing-js/src/interp_helpers.rs`, `crates/amazing-js/src/interp_class_private.rs`, `crates/amazing-js/src/interp_stmt.rs` (JSRT-19: NamedEvaluation shape and Annex B.3.4 if-position function-declaration sync) |
| [DioxusLabs/taffy](https://github.com/DioxusLabs/taffy) | `e661b35980d9644ad8f1df64e183d27e5884533b` | MIT | Copyright (c) 2018 Visly Inc.; Copyright (c) 2026 Taffy Authors | `crates/amazing-layout/src/grid_algorithm.rs` |
| [lucid-softworks/lumen](https://github.com/lucid-softworks/lumen) (`crates/lumen/src/coroutine.rs`, pooled-OS-thread generator coroutines) | `f6c9860d44fa52249cfd647122edea184d32c863` | MIT | Copyright (c) 2026 Lucid Softworks limited | `crates/amazing-js/src/interp_coroutine.rs` |
| [lucid-softworks/lumen](https://github.com/lucid-softworks/lumen) (`crates/lumen/src/interpreter.rs` `drive_async`, `crates/lumen/src/coroutine.rs` `coroutine_await`: await-driven coroutine async functions) | `f6c9860d44fa52249cfd647122edea184d32c863` | MIT | Copyright (c) 2026 Lucid Softworks limited | `crates/amazing-js/src/interp_coroutine.rs` (JSVM-9) |
| [lucid-softworks/lumen](https://github.com/lucid-softworks/lumen) (`crates/lumen/src/temporal.rs`) | `f6c9860d44fa52249cfd647122edea184d32c863` | MIT | Copyright (c) 2026 Lucid Softworks limited | `crates/amazing-js/src/builtins_temporal_calendar.rs` (JSS-calendars: non-ISO Temporal calendar math -- calendar id table/eras, coptic/ethiopic/ethioaa, islamic-civil/tbla/umalqura(approximated), indian, persian, hebrew, chinese/dangi) |
| [lucid-softworks/lumen](https://github.com/lucid-softworks/lumen) (`crates/lumen/src/temporal.rs` `diff_date_cal`) | `f6c9860d44fa52249cfd647122edea184d32c863` | MIT | Copyright (c) 2026 Lucid Softworks limited | `crates/amazing-js/src/builtins_temporal_arithmetic.rs` (JSS-calendars: month-structure calendar-native since/until bracket search, replacing an ordinal-month-position search that mis-counted chinese/dangi) |
| [lucid-softworks/lumen](https://github.com/lucid-softworks/lumen) (`crates/lumen/src/umalqura.rs`, numeric Umm al-Qura calendar tables) | `f6c9860d44fa52249cfd647122edea184d32c863` | MIT | Copyright (c) 2026 Lucid Softworks limited | `crates/amazing-js/src/builtins_temporal_umalqura.rs` |
| [boa-dev/boa](https://github.com/boa-dev/boa) (`core/engine/src/builtins/promise/mod.rs`) | `39cd11214ecb366200157dab96b3919e61e9a391` | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/interp_promise_capability.rs` (JSRT-27: Promise capability construction and resolution) |
| [boa-dev/boa](https://github.com/boa-dev/boa) (`core/engine/src/builtins/promise/mod.rs`) | `39cd11214ecb366200157dab96b3919e61e9a391` | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/interp_promise_resolution.rs` (JSRT-28: resolving functions and thenable jobs) |
| [lucid-softworks/lumen](https://github.com/lucid-softworks/lumen) (`crates/lumen/src/interpreter.rs`) | `f6c9860d44fa52249cfd647122edea184d32c863` | MIT | Copyright (c) 2026 Lucid Softworks limited | `crates/amazing-js/src/interp_function_environments.rs` (JSRT-29: ContainsExpression and parameter/body environment separation) |
| [boa-dev/boa](https://github.com/boa-dev/boa) (`core/engine/src/builtins/promise/mod.rs`) | `39cd11214ecb366200157dab96b3919e61e9a391` | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/interp_promise_reactions.rs`, `crates/amazing-js/src/interp_builtins.rs` (JSRT-30: internal PerformPromiseThen and optional reaction capability) |
| [boa-dev/boa](https://github.com/boa-dev/boa) (`core/engine/src/builtins/promise/mod.rs`) | `39cd11214ecb366200157dab96b3919e61e9a391` | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/interp_promise_finally.rs` (JSRT-31: Promise.prototype.finally and cleanup closures) |
| [boa-dev/boa](https://github.com/boa-dev/boa) (`core/engine/src/builtins/promise/mod.rs`) | `39cd11214ecb366200157dab96b3919e61e9a391` | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/interp_promise_keyed.rs` (JSRT-32: keyed Promise combinators and element closures) |

## MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy of this software
and associated documentation files (the "Software"), to deal in the Software without
restriction, including without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## Apache License 2.0

Ported Apache-2.0 code is used under the Apache License, Version 2.0
(https://www.apache.org/licenses/LICENSE-2.0). Any upstream `NOTICE` file content is
reproduced in the table row for that upstream.

| servo/html5ever | MIT | Copyright (c) 2014 The html5ever Project Developers; source copyright 2014-2017 The html5ever Project Developers | `crates/amazing-html/src/tree/doctype.rs` — doctype mode tables and classification adapted from `html5ever/src/tree_builder/data.rs` at `6db7bbf1c4dfaecd28731950d427e40c8b911bef` |

| servo/html5ever | MIT | Copyright (c) 2014 The html5ever Project Developers | `crates/amazing-html/src/tree/noscript.rs` — complete in-head-noscript insertion mode adapted from `html5ever/src/tree_builder/rules.rs` at `6db7bbf1c4dfaecd28731950d427e40c8b911bef` |
| boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 | MIT | Copyright (c) 2019 Jason Williams | Optional-chain receiver, short-circuit and final-link delete algorithms adapted from core/engine/src/bytecompiler/mod.rs into crates/amazing-js/src/interp_optional_chain.rs. |
| servo/html5ever | MIT | Copyright 2014 The html5ever Project Developers | Complete form start/end algorithm adapted from html5ever/src/tree_builder/rules.rs @6db7bbf1c4dfaecd28731950d427e40c8b911bef in crates/amazing-html/src/tree/form.rs. |

| boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 | MIT | Copyright (c) 2019 Jason Williams | Object-literal method definition name/HomeObject operations adapted from core/engine/src/bytecompiler/expression/object_literal.rs into crates/amazing-js/src/interp_object_methods.rs. |

| boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 | MIT | Copyright (c) 2018 Boa Developers | crates/amazing-js/src/module_graph.rs (ordered module graph walks and ResolveExport) |
| ridiculousfish/regress @ 771e6c8fa1e472ef86c45b5206b37fa4e4494e25 | MIT | Copyright (c) 2020 ridiculous_fish | `crates/amazing-regex/src/vm.rs` — reverse concatenation, directional capture/input/backreference matching and greedy-loop retry algorithms adapted from src/ir.rs, src/cursor.rs and src/classicalbacktrack.rs. |
| lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 | MIT | Copyright (c) 2026 Lucid Softworks limited | Decorator grammar and class/context/initializer algorithms adapted from crates/lumen/src/parser.rs and crates/lumen/src/eval.rs into crates/amazing-js/src/parser_decorators.rs, interp_decorators.rs, interp_class_eval.rs and interp_class_private.rs. |

| boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 | MIT | Copyright (c) 2019 Jason Williams | crates/amazing-js/src/interp_symbol.rs — Array iterator live length, cursor advancement and exhaustion adapted from core/engine/src/builtins/array/array_iterator.rs. |
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Adapted reference and assignment-pattern evaluation in crates/amazing-js/src/interp_references.rs and interp_stmt.rs (JS27).
| boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 | MIT | Copyright (c) 2019 Jason Williams | PerformEval context flags, fresh lexical environment and restoration adapted from core/engine/src/builtins/eval/mod.rs into crates/amazing-js/src/interp_eval_context.rs. |
| lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 | MIT | Copyright (c) 2026 Lucid Softworks limited | Annex B declaration discovery and block lexical/function instantiation adapted from crates/lumen/src/interpreter.rs and crates/lumen/src/eval.rs into crates/amazing-js/src/vm/compiler_declarations.rs and vm/mod.rs. |

| boa-dev/boa @ `39cd11214ecb366200157dab96b3919e61e9a391` (`core/engine/src/builtins/function/arguments.rs`) | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/interp_arguments.rs`, `crates/amazing-js/src/builtins_std_object_model.rs`: mapped/unmapped arguments creation and exotic descriptor algorithms adapted to BindingCell aliases. |

| boa-dev/boa @ `39cd11214ecb366200157dab96b3919e61e9a391` (`core/engine/src/builtins/proxy/mod.rs`) | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/interp_proxy.rs`: Proxy internal-method descriptor compatibility, extensibility and own-key invariants adapted to Amazing. |

| boa-dev/boa @ `39cd11214ecb366200157dab96b3919e61e9a391` (`core/engine/src/builtins/json/mod.rs`) | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/builtins_std_json_global.rs`: raw JSON creation/brand, reviver internalization and UTF-16 serialization algorithms adapted to semantic Amazing object operations. |

- lucid-softworks/lumen — MIT, Copyright (c) 2026 Lucid Softworks Limited. Module phases, canonical requests, asynchronous DFS/SCC evaluation and namespace operations adapted at f6c9860d44fa52249cfd647122edea184d32c863 from crates/lumen/src/modules.rs into crates/amazing-js/src/module_requests.rs, module_async.rs, value_module.rs, interp_module_evaluation.rs and interp_module_namespace.rs.
| lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 | MIT | Copyright (c) 2026 Lucid Softworks limited | Whole AsyncFromSync for-of and async yield-star continuation adapted from crates/lumen/src/eval.rs into crates/amazing-js/src/vm/async_from_sync.rs, vm/async_delegate.rs, vm/mod.rs and interp_async_iter.rs (JSVM-20). |
| boa-dev/boa @ `39cd11214ecb366200157dab96b3919e61e9a391` (`core/engine/src/builtins/object/mod.rs`, `core/engine/src/object/operations.rs`) | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/interp_object_extra.rs`, `crates/amazing-js/src/interp_builtins.rs`, `crates/amazing-js/src/builtins_std_object_model.rs`: Object prototype lookup/tag/receiver operations and SetIntegrityLevel algorithms adapted to semantic runtime gateways. |
| lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 | MIT | Copyright (c) 2026 Lucid Softworks limited | collect_annexb_funcs direct-block-function lexical barriers adapted from crates/lumen/src/interpreter.rs into crates/amazing-js/src/interp.rs; the existing vm/compiler_declarations.rs algorithm now covers suspended callable bodies (JSVM-21). |

| lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 | MIT | Copyright (c) 2026 Lucid Softworks limited | IteratorRecord, array-pattern step/elision and normal IteratorClose adapted from crates/lumen/src/eval.rs into crates/amazing-js/src/interp_helpers.rs and interp_stmt.rs (JS-30). |

| boa-dev/boa @ `39cd11214ecb366200157dab96b3919e61e9a391` (`core/engine/src/builtins/array/mod.rs`) | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/interp_array_exotic.rs`, `crates/amazing-js/src/builtins_std_object_model.rs`, `crates/amazing-js/src/interp_property_access.rs`, `crates/amazing-js/src/interp_builtins.rs`: Array exotic definition/length, species/create and semantic Array.isArray algorithms adapted to existing runtime descriptor operations. |

| boa-dev/boa @ `39cd11214ecb366200157dab96b3919e61e9a391` (`core/engine/src/builtins/array/from_async.rs`, `core/engine/src/builtins/iterable/async_from_sync_iterator.rs`) | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/interp_array_from_async.rs`: resumable Array.fromAsync and private AsyncFromSyncIterator await/close algorithms adapted to native Promise reactions and private native home state. |

| boa-dev/boa @ `39cd11214ecb366200157dab96b3919e61e9a391` (`core/engine/src/builtins/object/mod.rs`) | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/interp_builtins.rs`: Object.getPrototypeOf, isExtensible, preventExtensions and keys primitive/conversion boundaries adapted to existing boxed brands and semantic dispatch. |
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. String escape decoding adapted from crates/lumen/src/lexer.rs into crates/amazing-js/src/lexer.rs (JS33).
| jsdom/w3c-xmlserializer | MIT | Copyright Sebastian Mayr | XML namespace/prefix and node serialization ported from lib/serialize.js, lib/attributes.js, lib/constants.js @265f1e1a192594708281809c8c7730b65acb36f3 in crates/amazing-dom/src/xml_serializer.rs; full license beside source. |
| lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 | MIT | Copyright (c) 2026 Lucid Softworks limited | Whole GlobalDeclarationInstantiation validation, lexical reservation and object-backed variable/function installation adapted from crates/lumen/src/interpreter.rs run_program into crates/amazing-js/src/interp_global_declarations.rs (JSVM-23). |

| boa-dev/boa @ `39cd11214ecb366200157dab96b3919e61e9a391` (`core/engine/src/property/mod.rs`) | MIT | Copyright (c) 2019 Jason Williams | `crates/amazing-js/src/value.rs`: completed Data/Accessor descriptor-kind model and classification adapted to a scalar tag; existing semantic descriptor algorithms consume the explicit kind. |

- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Synchronous yield_delegate/GetIterator adapted from crates/lumen/src/eval.rs into crates/amazing-js/src/interp_yield_delegate.rs; raw-result adapters in interp_coroutine.rs and vm/mod.rs (JS35).
- **boa-dev/boa** — MIT, Copyright (c) 2019 Jason Williams; `core/engine/src/builtins/math/mod.rs` @ `39cd11214ecb366200157dab96b3919e61e9a391`; Math intrinsic method metadata adapted in `crates/amazing-js/src/interp_install.rs`.

- **boa-dev/boa** — MIT, Copyright (c) 2019 Jason Williams; `core/engine/src/builtins/promise/mod.rs` @ `39cd11214ecb366200157dab96b3919e61e9a391`; Promise.try/withResolvers and Promise prototype tag installation adapted in `crates/amazing-js/src/interp_promise_static.rs` and `crates/amazing-js/src/interp_install.rs`.

- **boa-dev/boa** — MIT, Copyright (c) 2019 Jason Williams; `core/engine/src/builtins/math/mod.rs` @ `39cd11214ecb366200157dab96b3919e61e9a391`; Number::exponentiate guards adapted in `crates/amazing-js/src/value.rs`, reused by Math.pow, AST Pow and the VM numeric Pow consumer.

- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. StatementList/try completion algorithms adapted in `crates/amazing-js/src/interp_stmt.rs` and `crates/amazing-js/src/vm/compiler_completions.rs`.
- boa-dev/boa, MIT, Copyright (c) 2019 Jason Williams and contributors; core/engine/src/builtins/promise/mod.rs @ 39cd11214ecb366200157dab96b3919e61e9a391: classic Promise combinator algorithms adapted in crates/amazing-js/src/interp_promise_combinators.rs.

- boa-dev/boa, MIT, Copyright (c) 2019 Jason Williams and contributors; core/engine/src/builtins/promise/mod.rs @ 39cd11214ecb366200157dab96b3919e61e9a391: intrinsic Promise constructor allocation/order adapted in crates/amazing-js/src/interp_builtins.rs.

- boa-dev/boa, MIT, Copyright (c) 2019 Jason Williams and contributors; core/engine/src/object/operations.rs @ 39cd11214ecb366200157dab96b3919e61e9a391: OrdinaryHasInstance algorithm bound-target delegation adapted in crates/amazing-js/src/interp.rs.

- boa-dev/boa — MIT, Copyright (c) 2019 Jason Williams and contributors; core/engine/src/builtins/math/mod.rs @ 39cd11214ecb366200157dab96b3919e61e9a391: Math.sumPrecise iterator algorithm adapted in crates/amazing-js/src/interp_math_sum.rs and its dispatch in builtins_std_number_math.rs.

- Gumichocopengin8/xsum.rs — MIT, Copyright 2015, 2018, 2021, 2024 Radford M. Neal; Copyright 2025 Keita Nonaka; src/xsum_small.rs, src/accumulators/small_accumulator.rs and src/constants.rs @ 7668ca84cb00dc8f4a5fc69229d1d317e9735601: safe exact small superaccumulator adapted in crates/amazing-js/src/interp_math_sum.rs.

- boa-dev/boa — MIT, Copyright (c) 2019 Jason Williams and contributors; core/engine/src/builtins/number/mod.rs @ 39cd11214ecb366200157dab96b3919e61e9a391: whole toFixed/toExponential/toPrecision entry conversion, validation and nonfinite ordering adapted in crates/amazing-js/src/builtins_std_number_math.rs, preserving Amazing's existing exact decimal formatter.

- boa-dev/boa — MIT, Copyright (c) 2019 Jason Williams and contributors; core/engine/src/builtins/iterable/{wrap_for_valid_iterator,iterator_constructor,iterator_prototype}.rs and iterator_helper/{mod,map,filter,take,drop,flat_map,zip}.rs @ 39cd11214ecb366200157dab96b3919e61e9a391: whole synchronous iterator wrapper/lazy-helper/zip protocols adapted to interpreter-owned states in crates/amazing-js/src/builtins_std_iterator.rs.
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Async-generator ReturnStatement and injected-yield Return Await adapted from crates/lumen/src/eval.rs into crates/amazing-js/src/interp_stmt.rs, interp_coroutine.rs and interp_async_generator_return.rs; Amazing coroutine request/continuation adapter uses existing internal Promise machinery.

- boa-dev/boa — MIT, Copyright (c) 2019 Jason Williams and contributors; core/engine/src/builtins/{map,set}/mod.rs @ 39cd11214ecb366200157dab96b3919e61e9a391: whole strong Map/Set constructors, upsert, live callback iteration, Set methods and iterator protocols adapted in crates/amazing-js/src/builtins_std_collections.rs and builtins_std_collection_iter.rs.
- boa-dev/boa, MIT, Copyright (c) 2019 Jason Williams and contributors; core/engine/src/bytecompiler/mod.rs @ 39cd11214ecb366200157dab96b3919e61e9a391: optional-chain reference preservation, short-circuit links and deletion adapted in crates/amazing-js/src/vm/compiler_chains.rs.
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Async-generator Await/Yield/delegation algorithms adapted in `crates/amazing-js/src/interp_async_generator_yield.rs`, with shared protocol implementation in `crates/amazing-js/src/vm/async_delegate.rs`.

- boa-dev/boa — MIT, Copyright (c) 2019 Jason Williams and contributors; core/engine/src/builtins/array/mod.rs @ 39cd11214ecb366200157dab96b3919e61e9a391: whole generic Array length/index/trap, copy-allocation, streaming FlattenIntoArray and metadata algorithms adapted in crates/amazing-js/src/interp_array.rs, interp_array_protocol.rs and Array-only search/reduction entries in interp_builtins.rs.
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Synchronous ForIn/OfBodyEvaluation cached iterator and abrupt-close algorithm adapted from crates/lumen/src/eval.rs into crates/amazing-js/src/interp_stmt.rs, using Amazing PatternIter and the existing VM iterator state.
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Rooted ClassDefinitionEvaluation phases adapted in `crates/amazing-js/src/interp_class_evaluation_state.rs`.
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Ordered resumable ClassDefinitionEvaluation lowering adapted in `crates/amazing-js/src/vm/compiler_classes.rs`.
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. NumericLiteral scanning adapted from crates/lumen/src/lexer.rs into crates/amazing-js/src/lexer.rs; normalized Amazing Numeric payload and original scanner snapshots preserve legacy provenance, while existing BinaryBigInt handles unrestricted radix conversion.

- boa-dev/boa — MIT, Copyright (c) 2019 Jason Williams and contributors; core/engine/src/builtins/error/{mod,aggregate,type}.rs @ 39cd11214ecb366200157dab96b3919e61e9a391: whole Error-family allocation, metadata, message/cause and strict iterable algorithms adapted in crates/amazing-js/src/interp_errors.rs, interp_install.rs, interp_builtins.rs and the owned constructor dispatch in interp_stmt.rs; shared allocation adapted to current normative SuppressedError sequence.
- tafia/quick-xml @ 2eaa844b4b40117fb7ed4d1f512ede918125aeab — MIT, Copyright (c) 2016 Johann Tuffe. Complete supported XML byte-detection area from src/encoding.rs adapted in crates/amazing-html/src/xml_encoding.rs; full license adjacent.

- html5lib/html5lib-python @ fd4f032bc090d44fb11a84b352dad7cbee0a4745 — MIT; Copyright (c) 2006-2013 James Graham and other contributors. Complete EncodingParser.getAttribute adapted into crates/amazing-html/src/encoding_prescan.rs; full license encoding_prescan.LICENSE-MIT.

| W3C XML Test Suite 20130923 (https://www.w3.org/XML/Test/) | W3C Software Notice and License (19980720) | Original catalog copyrights include Sun Microsystems1998–1999, OASIS1999–2001 and individual fixture authors; full preserved notices in CATALOG-NOTICES.md | crates/amazing-html/evidence/html70-xml-conformance/manifest.json: normative fixture IDs/metadata/descriptions only; no upstream engine implementation code |

- jsdom/jsdom, MIT, Copyright (c) 2010 Elijah Insua; Selection-impl.js @9547fbf5b60af45d54c9e34d0891c7808f6ee2ce adapted in crates/amazing-dom/src/selection.rs.
- lucid-softworks/lumen, MIT, Copyright (c) 2026 Lucid Softworks limited; interpreter.rs build_hoist_ops/apply_hoist_ops @ f6c9860d44fa52249cfd647122edea184d32c863 declaration instantiation adapted in crates/amazing-js/src/module.rs.

- boa-dev/boa — MIT, Copyright (c) 2019 Jason Williams and contributors; core/engine/src/builtins/iterable/iterator_prototype.rs @ 39cd11214ecb366200157dab96b3919e61e9a391: whole SetterThatIgnoresPrototypeProperties adapted in crates/amazing-js/src/interp_builtins.rs for Error.stack.
- boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 — MIT; Copyright (c) 2019 Jason Williams. Whole call-reference/receiver and direct-eval argument lowering from `core/engine/src/bytecompiler/mod.rs::call` adapted in `crates/amazing-js/src/vm/compiler.rs` and `crates/amazing-js/src/vm/compiler_chains.rs`.
- lucid-softworks/lumen, MIT, Copyright (c) 2026 Lucid Softworks limited; eval.rs delete expression member/index paths @ f6c9860d44fa52249cfd647122edea184d32c863 adapted in crates/amazing-js/src/interp_optional_chain.rs.
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Terminal Super Delete reference-evaluation/rejection ordering from `crates/lumen/src/eval.rs` adapted in `crates/amazing-js/src/vm/compiler_chains.rs`.
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Super Reference/GetValue evaluation order from `crates/lumen/src/eval.rs` adapted in the first-super-link lowering in `crates/amazing-js/src/vm/compiler_chains.rs`.
- boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 — MIT; Copyright (c) 2019 Jason Williams. Whole tagged-template callee/receiver capture, site lookup and ordered substitutions from `core/engine/src/bytecompiler/expression/mod.rs` adapted in `crates/amazing-js/src/vm/compiler_templates.rs`.

- lucid-softworks/lumen (MIT), Copyright (c) 2026 Lucid Softworks limited. GetTemplateObject interpreter-owned registry/cache adapted from crates/lumen/src/eval.rs @ f6c9860d44fa52249cfd647122edea184d32c863 in crates/amazing-js/src/interp_stmt.rs; stable parse-site tokens, realm identity and UTF-16 values use Amazing types.

| boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391, core/engine/src/builtins/{number,string,boolean}/mod.rs | MIT | Copyright (c) 2019 Jason Williams | Whole primitive Number/String/Boolean constructor call/construct allocation protocol: crates/amazing-js/src/runtime_primitive_constructors.rs, interp_builtins.rs, builtins_std_number_math.rs |
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Whole Super Reference capture/GetValue/PutValue algorithm from `crates/lumen/src/eval.rs` adapted with corrected computed-key ordering in `crates/amazing-js/src/vm/compiler_super_writes.rs`, `compiler.rs`, `compiler_patterns.rs` and `mod.rs`.
- lucid-softworks/lumen — MIT — Copyright (c) 2026 Lucid Softworks limited — CopyDataProperties adapted from crates/lumen/src/eval.rs @ f6c9860d44fa52249cfd647122edea184d32c863 into crates/amazing-js/src/interp_copy_data_properties.rs.

- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Super Reference/GetValue algorithm from `crates/lumen/src/eval.rs` reused with corrected rooted-chain evaluation order for suspended standalone/nested reads in `crates/amazing-js/src/vm/compiler_super_reads.rs` and `crates/amazing-js/src/vm/compiler.rs`.

| boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391, core/engine/src/builtins/typed_array/object.rs | MIT | Copyright (c) 2019 Jason Williams | Whole TypedArray IsTypedArrayFixedLength and PreventExtensions protocol: crates/amazing-js/src/runtime_binary_integrity.rs, interp_proxy.rs; ordinary integrity algorithm retained in interp_object_extra.rs |
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. ClassDefinitionEvaluation heritage capture from `crates/lumen/src/eval.rs` adapted through Amazing's corrected rooted Super Reference algorithm in `crates/amazing-js/src/vm/compiler_classes.rs`.

- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Whole live GetSuperConstructor/SuperCall and original-constructor instance-element algorithm from `crates/lumen/src/eval.rs` adapted to existing frame/lexical function roots and rooted bytecode arguments in `crates/amazing-js/src/interp_stmt.rs`, `crates/amazing-js/src/vm/compiler.rs` and `crates/amazing-js/src/vm/mod.rs`.
- boa-dev/boa @39cd11214ecb366200157dab96b3919e61e9a391, MIT, Copyright (c) 2019 Jason Williams: UTF-16 SourceCharacter decoding and lossless literal cursor/scanning adapted from core/parser/src/source/utf16.rs and core/parser/src/lexer/{cursor,string,template}.rs in crates/amazing-js/src/lexer_source.rs and lexer.rs.

- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Whole Super Reference/GetValue/PutValue algorithms from `crates/lumen/src/eval.rs` reused for ordinary native read/write/call dispatch in `crates/amazing-js/src/vm/compiler.rs` and `crates/amazing-js/src/vm/compiler_super_reads.rs`.

- boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 — MIT; Copyright (c) 2019 Jason Williams. Existing whole optional-chain reference/short-circuit/call/delete algorithm from `core/engine/src/bytecompiler/mod.rs` reused for ordinary expression dispatch in `crates/amazing-js/src/vm/compiler.rs` and `crates/amazing-js/src/vm/compiler_chains.rs`; existing Lumen Super-reference portions retain their upstream headers.
| lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 | MIT | Copyright (c) 2026 Lucid Softworks limited | crates/amazing-js/src/runtime_string_normalization_impl.rs (crates/lumen/src/unicode_norm_impl.rs; whole normalization algorithm, no upstream tables adopted pending data license review) |

- Unicode, Inc. — Unicode-3.0 data-only exception (ADR0014), Copyright ©1991-2026 Unicode, Inc. Official Unicode17.0.0 UnicodeData.txt, DerivedNormalizationProps.txt, CompositionExclusions.txt and NormalizationTest.txt from https://www.unicode.org/Public/17.0.0/ucd/; originals, license and exact URL/SHA provenance in crates/amazing-js/data/unicode17/, generated normalization tables in crates/amazing-js/src/runtime_string_normalization_data_{canonical,compatibility,combining}.rs. No Unicode Rust algorithm, CLDR or IANA material adopted.
- lucid-softworks/lumen — MIT — Copyright (c) 2026 Lucid Softworks limited — CopyDataProperties adapted from crates/lumen/src/eval.rs @ f6c9860d44fa52249cfd647122edea184d32c863 into crates/amazing-js/src/interp_copy_data_properties.rs.
- lucid-softworks/lumen @ f6c9860d44fa52249cfd647122edea184d32c863 — MIT; Copyright (c) 2026 Lucid Softworks limited. Super Reference/GetValue evaluation order from crates/lumen/src/eval.rs adapted in crates/amazing-js/src/interp_optional_chain.rs and the first-super-link lowering in crates/amazing-js/src/vm/compiler_chains.rs.

| boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391, core/engine/src/builtins/array/mod.rs | MIT | Copyright (c) 2019 Jason Williams | Whole Array.prototype.join ordered generic receiver/length/separator/element protocol adapted to UTF-16 accumulation in crates/amazing-js/src/interp_builtins.rs; existing identity-cycle guard retained |

- boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 — MIT; Copyright (c) 2019 Jason Williams. ForIn head initializer ordering from `core/engine/src/bytecompiler/statement/loop.rs::compile_for_in_loop` adapted in `crates/amazing-js/src/interp_stmt.rs` and `crates/amazing-js/src/vm/compiler_patterns.rs`.

- boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 — MIT, Copyright (c) 2019 Jason Williams; Function.prototype.bind/BoundFunctionCreate (core/engine/src/builtins/function/{mod,bound}.rs), adapted in crates/amazing-js/src/interp_function_bind.rs.
- boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 — MIT; Copyright (c) 2019 Jason Williams. Whole call-reference/receiver and direct-eval argument lowering from `core/engine/src/bytecompiler/mod.rs::call` adapted in `crates/amazing-js/src/vm/compiler.rs` and `crates/amazing-js/src/vm/compiler_chains.rs`.
- lucid-softworks/lumen (MIT), Copyright (c) 2026 Lucid Softworks limited. GetTemplateObject interpreter-owned registry/cache adapted from crates/lumen/src/eval.rs @ f6c9860d44fa52249cfd647122edea184d32c863 in crates/amazing-js/src/interp_stmt.rs; stable parse-site tokens, realm identity and UTF-16 values use Amazing types.
- boa-dev/boa @ 39cd11214ecb366200157dab96b3919e61e9a391 — MIT; Copyright (c) 2019 Jason Williams. Whole tagged-template callee/receiver capture, site lookup and ordered substitutions from core/engine/src/bytecompiler/expression/mod.rs adapted in crates/amazing-js/src/vm/compiler_templates.rs.

- DioxusLabs/taffy, MIT, commit 2c81b8e09d172c2e4d7747be2b1e5a03e0f30bf0; Copyright (c) 2018 Visly Inc.; Copyright (c) 2026 Taffy Authors. Flex auto-margin/axis-alignment adaptation in crates/amazing-layout/src/flex.rs; license SHA256 f97daf1a0124413dccf399a4e6626b4b74acd05282f80b6d64ac82225650b77a.
- rust-lang/stacker @ d3ac2df79a409c681329006e9b31c96ab9c5fcae — MIT, Copyright (c) 2014 Alex Crichton. OS-thread stack-bound discovery and volatile current-frame measurement adapted in `crates/amazing-js/src/call_stack.rs`; no crate dependency or stack switching.
- rust-lang/libc @ b739c733e06a5f184bae7e296919c15d6ccd8ec8 — MIT, Copyright (c) The Rust Project Developers. GNU x86_64 `pthread_attr_t` layout adapted in `crates/amazing-js/src/call_stack.rs`; no crate dependency.
- lucid-softworks/lumen, MIT, Copyright (c) lucid-softworks/lumen contributors, commit f6c9860d44fa52249cfd647122edea184d32c863: function body lexical reservation and hoist phase adaptation in crates/amazing-js/src/{interp.rs,interp_stmt.rs,vm/compiler.rs,vm/compiler_declarations.rs,vm/mod.rs}.

| lucid-softworks/lumen | MIT | Copyright (c) 2026 Lucid Softworks limited | `crates/lumen/src/{eval,interpreter,bytecode}.rs`, `interpreter/call_entry.rs` @ f6c9860d44fa52249cfd647122edea184d32c863; proper-tail expression/completion/redispatch adaptation in Amazing `interp_tail_calls.rs`, `interp{,_stmt,_realms,_bound}.rs`, `vm/{mod,compiler,compiler_chains,compiler_resources,compiler_tail_calls}.rs` |
