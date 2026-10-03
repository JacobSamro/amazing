# JS27 reference capture and assignment evaluation

Fixed integrated dependency: `0d5963fe7abda88ec6ea8100efd6e853f278bf4b`.
The lane retains JS26's original history on `lane/js-lang-js11`; work proceeds
on `lane/js-lang-js27` so VM17's ordinary callable policy and broad class
fallback guards are included without reapplying sibling changes.

Reference records capture the declaring EnvHandle, an object environment with its
WithBaseObject distinction, or an unresolvable reference's realm global. A read of
an unresolvable reference always throws. A strict unresolvable write throws after
evaluating its RHS. Object environment reads/writes recheck HasProperty.
Capturing an EnvHandle clones its source-context sidecar unchanged.

Computed assignment target references retain the raw key until GetValue or
PutValue. GetValue updates the reference with its converted key, so compound
assignment and updates convert once. Plain assignment evaluates RHS before key
conversion and nullish-base rejection. Public primitive writes use a boxed target
and the original primitive receiver, observing inherited setters and strict versus
sloppy failures. A computed super key evaluates before capturing the current
HomeObject prototype; the actual receiver is obtained before the expression.

Destructuring prepares each non-pattern target before source GetValue, default
evaluation or iterator stepping/draining. Defaults and rest copying then use that
captured target. Source keys retain UTF-16 code units. Nested patterns recurse
when their source value becomes available. Abrupt target capture closes the
iterator using the existing completion precedence and done tracking.

ToNumeric finishes each numeric operand conversion in source order, and postfix
updates return the converted numeric value, including strings and booleans.

Port source: MIT `lucid-softworks/lumen`, `crates/lumen/src/eval.rs`, commit
`f6c9860d44fa52249cfd647122edea184d32c863`; copyright 2026 Lucid Softworks
limited. Reference and assignment-pattern algorithms are adapted to Amazing's
existing EnvHandle, property gateways, private brands and PatternIter. Object rest
captures its target before copying, following the spec rather than the upstream
implementation's later capture. Headers and an appended notices row identify the
ported files. No dependency is introduced.

Parent and VM owner granted the exact shared Reference consumers, raw member
write instruction, write_member_reference helper in plain Assign and prepared
pattern targets, and Update's ToNumeric helper. Runtime35 eval contexts and private
metadata, Runtime36 WindowProxy hooks, VM invocation/factory/FDI fields and
statement scope/hoist work remain separately owned.

Gate evidence is pending. The baseline's initial corpus invocation rejected an
unsupported CLI flag; its preserved log is incomplete corpus evidence. The fixed
runner invocation uses `--only language/`, including Annex B language directories.
