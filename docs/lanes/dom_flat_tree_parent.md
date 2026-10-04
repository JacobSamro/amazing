# Native actual flat-tree parent

Fixed dependency is closed Window source/evidence e7841de2. New API `Dom::flat_tree_parent(NodeIdx) -> Result<Option<NodeIdx>, DomError>` is native only, not a public closed-root exposure. CSS Shadow4.1 defines actual slots as parents, shadow-root children as host children, and omitted unassigned light/fallback branches as absent. The full chain is validated so descendants of omitted branches cannot leak a container or style-query ancestor.

Existing `find_slot(node,false)` supplies named/manual/closed assignment. Assigned fallback suppression uses the actual native assigned list. Document/ordinary fragment roots stop traversal; ShadowRoot itself is not a flattened node. Detached ordinary element trees retain internal ancestry, but consumer CSS must separately validate actual connection and active context. Saved handles validate their generation before this immutable native query. No callback/current realm/owner-document connectivity guess occurs.

Native groups cover open/closed named and manual assignments, active/inactive fallback including nested shadow descendants, slotted slots, live reassignment, detached/removed hosts, real Document/template/fragment boundaries, invalid/retired nodes and reused generations. Lower semantics do not install a CSS facade or claim WPT gains before the CSS consumer adopts them. Primary specification: https://drafts.csswg.org/css-shadow-1/#flat-tree . This is an original adaptation to Amazing native types, no source port or added dependency.

Source provisional until release+workspace/docs+whole shadow-dom and stronger888/72 evidence closes on fixed before/after.

Fresh fixed-base Linux gate: workspace 4325→4329, 17 skipped each, doctests 4 each. Core888 92820→92820, syntax72 2706→2706, and the one clean paired shadow277 run 5676→5676 retain all names/multiplicities/passing files. Shadow rerun matches ordinary25s, actual META-long600s, heavy150s; original33 unfinished-page diagnostics remain recorded. Raw stays on Boat.
