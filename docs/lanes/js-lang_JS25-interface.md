# JS25 contextual eval parser

`parser::parse_eval(source, &EvalContext)` retains the Script goal and seeds
PerformEval's lexical early-error context. `EvalContext` is public and Default:
`strict`, `allow_new_target`, `allow_super_property`, `allow_super_call`, and
`class_initializer` booleans; `visible_private_names: Vec<String>` contains
private identifier text including `#`. Default describes indirect eval.

New-target permission does not enable Return, Yield or Await at program top
level. Super property and super call permissions are independent. Private names
remain visible through ordinary functions and nested classes, with normal inner
class shadowing; unresolved outer uses fail after the Script has parsed.
Initializer `arguments` restrictions include object shorthand and arrows, but
ordinary function/method parameters and bodies establish their own contexts.
Derived constructor parameter initializers can contain super calls. Computed
method keys still parse in their enclosing lexical context.

Runtime owns the consumer and obtains permissions from the lexical caller, not
native eval's callee frame. It also owns execution under that caller's home/this
context and contextual private-name resolution. VM17's invocation identity must
remain unchanged when its captured CallFrame is cloned. The parser API does not
change caller metadata, callee frames, environment schemas or eval execution.

Three grouped parser regressions cover independent permissions, Script-goal
restrictions, private scopes/shadowing/unknown names, arguments through arrow
and shorthand forms, and ordinary parameter/body boundaries. Owned Boat bulk:
529 passed / 1 skipped (JS502, regex27), 1 doctest. Workspace3602 passed /17 skipped,
4 doctests. Matching whole-language/ including AnnexB, 24572 selected files:
23438 passing files and43617 passing variants unchanged, zero losses/timeouts.
Exact before/after hashes and source equivalence are in
[js-lang_JS25-evidence.json](js-lang_JS25-evidence.json). Runtime integrates its
consumer after source locator34; the contextual eval cluster remains open until
that consumer gate.
