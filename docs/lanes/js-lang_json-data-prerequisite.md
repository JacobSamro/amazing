# Genuine JSON data prerequisite

Base: `68b1e6363be41f3867c684040326d54bf07af3af`.

`JsonDocument::parse_utf16` validates the entire exact UTF16 input into data-only
nodes and checked source ranges before any realm allocation. It retains JSON
member order and duplicate occurrences, exact decoded string units, number
values and primitive source spelling. It accepts only JSON whitespace, numbers,
literals, arrays, objects and escapes. Existing 512 nesting limit is preserved.

`Interpreter::binding_materialize_json` allocates using actual active intrinsic
Object/Array prototypes. Fresh own data properties bypass inherited setters;
`__proto__` is data, duplicate names replace in-place, array length retains its
standard descriptor. `binding_parse_json_utf16` normalizes invalid grammar in the
actual active realm. No private Interpreter, evaluation, synthetic script,
pretty printing or fabricated export is used.

Installed `std_json_parse` shares these operations, retaining unchanged semantic
reviver traversal (postorder, original holder, proxy internal methods, omitted
source for replaced values, source only for unchanged primitives, root key,
ignored unsuccessful delete/create-data results, abrupt callbacks). Raw JSON
validation uses the pure parser without pointless object allocation. The older
native entry delegates this same complete implementation. Stringify is unchanged.

Extraction adapts the existing attributed Boa JSON port at
`39cd11214ecb366200157dab96b3919e61e9a391`, MIT, Copyright 2019 Jason Williams;
exact upstream LICENSE-MIT checked. New files carry attribution, notices append
one row, no Cargo dependency or unsafe ownership change.

All original three JSON native groups remain unchanged. Four shared new observable
groups run on both genuine AST factories and native-body backend settings:
coercion/no evaluation; own descriptors/poisoned prototypes; reviver source/order;
mutation/proxy/abrupt controls. Three after-only genuine API groups cover the full
grammar, raw/escaped UTF16 units and independent actual realm materialization.
These are source prepared; no native result is claimed.

Qualification: fixed tests-only BEFORE; complete AFTER; workspace nextest,
doctests, release host; whole built-ins/JSON, language/import,
language/expressions/dynamic-import and language/module-code in default and VM
modes, pinned 3655e746, 60 seconds/jobs8. Authentic BEFORE failures are measured,
never masked. Module-family reports are retention controls at this prerequisite
stage, not a claim that the unchanged typed loader is implemented.

Actual typed ModuleSource/realm-qualified host composition remains a following
whole ticket. Preserve c24/eb0 originals and adopt genuine ceb4 source-entry APIs
only through independently published prerequisites; no ancestor transplantation.
