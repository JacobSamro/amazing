# CSSOM driver adoption

CSSOM-DRIVER installs Window.getComputedStyle and fresh live, branded,
read-only CSSStyleProperties objects derived from CSSStyleDeclaration.
The lower bindings crate has no CSS engine dependency: generation-tagged
element/pseudo targets cross a CssomHost callback boundary after WebIDL
argument conversion. Property reads and enumeration consult the driver's
current DOM mutation epoch and viewport; wrappers retain targets rather
than declaration snapshots. Reentrant coercions release native borrows.

The driver collects author sheets from the actual owning document, retains
scope roots and stylesheet URLs, and installs actual registered font metrics
before inheritance. CSS owns supported longhand/custom enumeration and
canonical shorthand serialization. The driver resolves colors, stylesheet
URLs, typed numeric values from the installed font context, and principal
box dimensions from the existing real layout. Legacy eight-bit alpha follows
[CSS Color 4 serialization](https://drafts.csswg.org/css-color-4/#serializing-alpha-values).
Active
frame-document lookup rejects inert documents and nonrendered iframe
containers. Pseudo parsing uses the existing selector parser.

Primary specification: [CSSOM declaration and computed-style algorithms](https://drafts.csswg.org/cssom/).
The API preserves exact UTF-16 custom names, ASCII normalization of ordinary
names, unsigned-long item conversion, readonly DOMException errors after
argument conversion, fresh wrapper identity, and live post-mutation reads.
Invalid/disallowed colon-prefixed pseudos produce empty declarations;
non-colon arguments request the principal element. Indexed iteration uses
the realm's Array.prototype.values.

## Gate

The corpus runner measures every pinned CSSOM
HTML/XHTML testharness page, with occurrence-aware named-subtest zero-loss
comparison as well as file classifications. Final paired Boat runs froze
before 961bdcc1b94d96ead412181e0efdbbff936e684b and feature source
4564e6e3c7be86a9444809f4e2993d2359b8a51a. Both contain original CSS42–45,
the pure live-frame lookup helper and root ArrayIterator LengthOfArrayLike
dependency 57d219d23a3279971a12e0a6716825f17df5864a, with byte-identical
dependency and runner files. The general iterator fix allows the realm's
Array.prototype.values to observe inherited live declaration length.

Before: 3709 native tests, 17 skipped, and 4 doctests. After: 3714 native
tests, 17 skipped, and 4 doctests, including all five grouped CSSOM tests.
Pinned WPT c2a2fce69e5c2509d9dc28440ec8b42532e5c293, whole 191-page CSSOM
selection, 5-second deadlines and eight jobs: 6 → 20 passing pages and
586 → 1169 passing named subtests, **+14 pages/+583 subtests, zero losses**.
The gate rechecks prior passes lost to deadlines at 60 seconds; none required
rechecking. Full reports and the reproducible bulk recipe are retained in
`crates/amazing/evidence/cssom-driver/`. The clean ready source preserves the
gated executable source bytes; only final documentation/evidence are added.

## Remaining boundaries

Only existing supported CSS properties are exposed. CSS-owned serialization
returns empty for unrepresentable shorthands rather than a stale specified
winner. Stylesheet/CSSRule mutation interfaces and mutable element.style
declarations are separate areas. Public layout currently lacks generated and
inline principal box identity and iframe document layout roots; those reads
retain computed values rather than invented sizes. The callback is not yet
attached to a retained document animation sample or media intrinsic resource
provider. Asynchronous font-set completion and provider epochs remain part
of the browser resource lifecycle follow-on.
The existing CSS get_pseudo map contains rendering-applicable generated
styles; it omits computed styles for nonrendered/nonexistent and some
non-generating pseudos. CSS48 owns an additive native-document/font-context
computed-pseudo query without changing this rendering map. Unsupported
functional pseudo kinds remain outside the lower CSS selector enum.
