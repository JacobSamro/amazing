# HTML75: current iframe navigation load dependencies

Fixed source base: `6350231606e466f08d84d73cd60989bfb8c3e925`. The independent History74 sources are not dependencies.

The HTML Standard iframe attribute-processing algorithm captures “current navigation was lazy loaded”; only false navigations potentially delay the owner Document load. The will-lazy-load algorithm tests the owner Document scripting-enabled flag and the actual whole ASCII-case-insensitive loading keyword. Initial about:blank and Location-triggered navigations do not pass through the attribute lazy step. Changing loading after a navigation has resumed does not retroactively change this flag.

The driver queues native owner/frame/child Document identities and generations, snapshots the flag, and revalidates the current frame navigation before completion. Parent load drains only its actual eager children, including their nested eager dependencies, without detaching unrelated/lazy work from the queue. Normal task turns later complete lazy child Window and nonbubbling container events. Superseded, removed, adopted and recycled associations do not dispatch stale loads.

Scope boundary: fetching remains immediate; actual layout-driven intersection eligibility/fetch resumption is not implemented. This ticket implements the normative nonblocking load dependency and does not infer visibility or fabricate rectangles.

Primary specifications reviewed 2026-10-04:
- https://html.spec.whatwg.org/multipage/iframe-embed-object.html#process-the-iframe-attributes
- https://html.spec.whatwg.org/multipage/iframe-embed-object.html#current-navigation-was-lazy-loaded
- https://html.spec.whatwg.org/multipage/urls-and-fetching.html#will-lazy-load-element-steps

Proof pending: whole workspace/docs and before/after iframe150, Window, syntax, modules, encoding and DOM. Root17's accepted hidden lazy assertion is a mandatory retained named pass, not a timeout or duplication waiver.
