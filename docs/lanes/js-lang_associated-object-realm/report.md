# Associated object creation realm

Expose `Interpreter::binding_object_creation_realm` from the corresponding allocated object's genuine native realm stamp. No Bound/Proxy target traversal, properties/traps, or current-realm fallback; revoked Proxy works. Missing provenance and foreign-agent objects reject. Existing GetFunctionRealm remains unchanged.

Recorded origin base: `5efe03ecd6ebf0cbeacca0a4dc24aa55a06f8f58`; tested source `40efa0461850af4bb672e433e2d667f1d3d0546a`. Six genuine AST/Bytecode, native/default, borrowed-Bind, revoked-Proxy, poisoned-prototype and provenance groups are after-only. No absent-API baseline adapter.

Linux workspace 5263 -> 5269 PASS, zero FAIL, 17 SKIP both; actual workspace doctests38 and separate host tests16 PASS both. Both jobs/fetches actual0. Six pinned directories default+VM preserve 26,311 passing files/48,729 passing variants each mode, zero losses/gains.

Native raw/ELFs UNARCHIVED on `bx_wyv9f4h4`: before job `5439e7c984a143f6803896672a1ce2af`, after `b8d4c907c07a485e9f7a8381373e6e36`. All631 ELFs per side retained/readback before target retirement. Compact includes raw paths/hashes/source/input guards. No full16 integration/publication claim.
