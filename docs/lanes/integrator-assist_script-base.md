# Classic WPT fixture URL alignment

Fixed base: module adoption implementation `432ba911` and evidence `17899b96`.
Both blocking and deferred classic fixture fetches now use the same captured
canonical URL as the immutable dynamic-import source locator. A document
`base href` therefore chooses the executed classic file as well as its later
imports. Per-implementation testdriver vendor resources are identified by the
resolved root-relative resource URL. Query-driven WPT HTTP response pipelines
remain unsupported; fragments do not affect source bytes.

A broad adapter regression executes the alpha fixture under a document base,
mutates that base to beta, proves delayed imports still use alpha, then loads
the beta fixture under the new base. Final Boat workspace/docs and the complete
111-page module corpus are compared against the preceding module gate.

Boat final workspace: 3565 nextest passed / 17 skipped; 4 doctests passed.
The full 111-page module corpus initially varied on instantiation-error-4,
reporting shared root error identity where the preceding run had distinct
errors. Two complete corpus repeats on identical source both retained all
35 passing pages and 110 passing subtests with zero gains/losses. This is an
existing module traversal/error-choice instability, routed to js-lang along
with choice-of-error-1/2 and duplicated-imports evaluation ordering. The
captured fixture URL and executed source bytes of the affected page are
unchanged by this ticket. Repeat evidence is `module-driver/script-base-after.json`.
