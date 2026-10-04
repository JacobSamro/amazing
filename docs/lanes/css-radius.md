# Physical border-radius grammar and computed declarations

Qualified on recorded origin/dev `6eec0d0015cbcfcea60882e1502f2f96497787e6`.
Linux default 4 CPU / 8 GiB paired gate: workspace 4459 → 4466 passed,
17 skipped, and four doctests passed on both sides. All 20 render directories
retained all classifications (1327 passes, zero losses). The 17-directory literal
parsing selection gained 11 assertions (4312 → 4323 / 5126), with no lost
assertions. These gains reject invalid radius declarations; they are not renderer
gains. Compact maps and raw-path/hash receipts are in
`crates/amazing-css/evidence/radius/`. Tested source is `017dff9717`; this ready
commit adds only documentation and compact evidence to that tested production.

The whole CSS Backgrounds3 §4.1 physical-radius declaration area shares its
length-percentage validator between cascade expansion and parsing conformance.
All four corner pairs and both independent shorthand quads compute genuine
absolute/em/rem lengths, range-clamp resolved calculations, preserve percentages
for the existing border-box consumer, and serialize shortest equivalent CSSOM
quads. Variables and CSS-wide inheritance use existing atomic winner expansion;
computed inherited lengths keep their originating font context. Public style
mutation shares the same normalization.

Primary source, read 2026-10-05:
https://www.w3.org/TR/2024/CRD-css-backgrounds-3-20240311/#border-radius
Own implementation, std-only; no copied upstream code.

Unknown font metrics and mixed percentage/length calculations retain their
existing context-dependent tokens. No box basis, font metric, geometric radius,
Paint corner formula or overlap reduction is fabricated here. Logical corner
mapping is a separate CSS Logical area; this ticket owns the four physical
longhands defined in Backgrounds3. Seven native matrices cover grammar/range,
quad/slash serialization, genuine font/root context, calculations, variables and
inheritance, rule cascade, and clone/set behavior.

Held joint Canvas/table/gradient source `f5330950` and unbuilt image-context
`06712471` are independent and unchanged. Their losses are not waived by this
parser ticket. Literal WPT adapter counts must be reported separately from
actual renderer and installed CSSOM results.
