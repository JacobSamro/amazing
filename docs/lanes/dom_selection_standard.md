# Programmatic Selection and traversal

Fixed baseline: e1037e5db3187b0521b1cffe2742c828e35c9424, closed Node creation realm source. Fresh Linux Boat gate session32105 measures the whole579 selection, dom/traversal and selection directories, plus72 syntax pages. No provisional feature or zero-loss claim before paired proof.

## Lower model and granted seams

Selection has an append-only SelectionId slot, native associated Document plus generation, retained live Range reference and Direction (forwards/backwards/directionless). Node reuse cannot retarget an old Selection. A document/gen owns its unique SelectionId. Fresh exposed Range allocations record their explicit realm source immediately: constructor actual callee, Document factory immutable receiver realm, clone saved source Range realm, Selection saved associated realm; lookup never derives it from boundaries, ownerDocument or the caller. Passed-in Range realm registration is immutable. A Selection strongly roots the actual associated Range wrapper, releases it on replacement/clear, and exports the canonical original object across realms. Constructor NewTarget prototype handling remains the existing runtime policy. RealmRegistry is still permanently owned and the general GC lifecycle is not solved by this change. Wrapper allocation validates arena/id/document generation, then enters the actual creation-document realm using the ready Node association; collection key contracts stay unchanged.

Document.getSelection requires an actual associated browsing context. Window.getSelection resolves the genuine receiver Window/WindowProxy document, including foreign same-agent receivers, and cannot substitute the caller document. HTML owner supplies precise pure receiver/association accessors; fully-active context is a different predicate.

A document/gen owns the scheduled selectionchange flag. Typed SelectionChangeTask carries actual document/gen and a token validating the pending task, never a Selection slot epoch. UserInteraction dispatch clears the flag before calling listeners, fires one trusted nonbubbling/noncancelable Document event, and admits reentrant changes as a subsequent task. Range identity and direction survive live-boundary adjustments. DOM mutations and script Range setters must schedule actual changed selection boundaries. Input/textarea event scheduling remains a separate existing HTML path.

## Document.open conflict resolved

The Selection editor draft definition retains an obsolete sentence saying document.open replaces the Selection singleton. Current HTML document-open steps erase listeners and replace document children without replacing Selection. Pinned WPT selection/Document-open.html deliberately changed to require the same Selection object and rangeCount1 in upstream commit0ea8f9e9da1dadf3fd23e20621f880d6acc6c0ac, 2019-03-13 (Selection and document.open()). The previous fixture explicitly quoted the now-removed HTML singleton replacement step. Preserve actual Selection/Range identity across open, letting ordinary live Range mutation adjust boundaries. No fixture-specific behavior or reset hook. The parent revoked its tentative reset grant after this audit. Raw fixture/history stays in MAIN scratch traversal-selection-inventory.

Primary sources: https://w3c.github.io/selection-api/ and https://html.spec.whatwg.org/multipage/dynamic-markup-insertion.html#dom-document-open .

## Port and boundaries

MIT jsdom/jsdom Selection algorithm audited at9547fbf5b60af45d54c9e34d0891c7808f6ee2ce, lib/jsdom/living/selection/Selection-impl.js; copyright2010 Elijah Insua. Any copied/adapted algorithm receives the exact source header and appended notice. That version lacks current direction/getComposedRanges/shadow changes and event coalescing; port onto our live Range, namespace/native node, typed task and actual realm architecture against current algorithms.

Programmatic scope includes aliases, direction/type/anchor/focus, live retained Range methods, ToUint32 conversions, collapse/extend/setBaseAndExtent/deletion and actual composed StaticRanges. modify/keyboard/geometric user actions remain outside this ticket. Rendered stringification and visual contains equivalence need a genuine layout contract; no textContent or fabricated geometry is presented as conforming. Existing unrendered stringifier compatibility must be labeled honestly.

Pinned webidl/current-realm.html directly verifies borrowed Document.createRange and Range.cloneRange return receiver/source-realm objects, citing the agreed bug24652 outcome not yet reflected in WebIDL wording. It is incorrect to infer factory/clone realm from NewObject or current callee alone. The explicit RangeCreationContext retains that distinction and never derives existing Range realm from its mutable boundaries. Raw primary fixture is in MAIN scratch traversal-selection-inventory/webidl__current-realm.html.

Composed StaticRange wrappers use the validated Selection-associated immutable Document realm via a bounded host callback. The JavaScript result array remains in the actual method callee realm; borrowed root/child methods and closed shadow-root exposure are tested separately.
