# JS26: decorators and class element initialization

Fixed base: `bd3bff1a43a456ffcabd551b779fd0058120841a`.

Port source: lucid-softworks/lumen, MIT, commit
`f6c9860d44fa52249cfd647122edea184d32c863`,
`crates/lumen/src/parser.rs` and `crates/lumen/src/eval.rs`.
The license was verified at this commit. Source headers and a new notices row
identify the adapted files. No engine dependency was added.

The AST keeps source-ordered decorator expressions on classes, methods and
fields. The parser accepts member expressions, one member call, or a parenthesized
expression; it checks the enclosing Yield/Await/private-name context and both
allowed positions around export. Constructors and static blocks reject decorators.

The evaluator captures expression values and receivers once, applies decorator
functions in reverse order, validates all replacement values, and gives each
addInitializer closure its own finished state. Access contexts use the existing
property gateways and exact private brands. Auto-accessors capture private backing
storage. Initializer lists retain callable values strongly through the existing Rc
value model; native access closures retain their hidden state and class environment.

Member decoration proceeds through static methods/accessors, instance
methods/accessors, static fields, then instance fields. Private methods precede
method extra initializers and field initialization. Static fields/blocks and class
extra initializers run on the final replacement class value. The class-local
binding remains in its temporal dead zone until class decoration completes.

Decorated classes fall back atomically from the VM compiler rather than losing
annotations. Suspension discovery includes every decorator expression and rejects
partial class lowering. This series does not introduce a new VM suspension opcode.
Runtime owns contextual eval and VM owns invocation-frame tokens and constructor
call-time initialization; these patches do not change those factories or callers.

Validation: exact source `19052d3c`, fixed base `bd3bff1a`; Linux workspace
3708 passed /17 skipped, four doctests. Whole language/ selection 24572 files:
23803 → 23843 passing files, 44234 → 44307 passing variants, +40/+73,
zero losses/timeouts. All 20 decorator paths pass. Exact hashes and gained paths
are in [js-lang_JS26-evidence.json](js-lang_JS26-evidence.json).
