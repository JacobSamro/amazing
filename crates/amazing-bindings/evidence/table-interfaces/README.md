# HTML table DOM interfaces

Fixed base8a816d91. Pinned WPT c2a2fce, complete html/semantics/tabular-data
classic testharness selection (29files):10passing and144failing subtests before.
Whole314dom/nodes baseline:12,682passes,298failures,six errors.

HTMLTableElement parts and create/delete methods, ordered live rows and tBodies;
HTMLTableSectionElement rows and insert/deleteRow; HTMLTableRowElement indices,
cells and insert/deleteCell; HTMLTableCellElement cellIndex. Table-specific
live filters reuse existing HTMLCollection cache and wrapper identity. Ordered
rows place thead rows first, direct/tbody rows next, tfoot rows last; only direct
structural relationships and HTML namespace elements count. All mutations use
checked DOM operations and owning document creation. Bindings validate receiver
and nullable part brands, preserve conversion order and implement WebIDL long
conversion/defaults. Existing reflected table attributes remain installed.

Native regressions cover live collection identity, reordered/mutated sections,
nested tables and foreign namespace exclusion, part placement and invalid part
atomicity, empty/index edges and installed conversion behavior. No upstream
engine code is copied. Algorithms read from the
[HTML Standard tables section](https://html.spec.whatwg.org/multipage/tables.html).

Final Boat native workspace3,534pass17skip and four doctests. Table29files:
10→146passes (+136),144→9failures; **zero lost passes**. Complete314nodes:
12,682→12,686passes (+4),298→293failures,six errors unchanged, zero lost.
The four DOM gains are getElementsByClassName tests that use row.cells.
The first candidate had the known intermittent XML XHR timeout; a complete
343-file unchanged-source recheck with two workers restored it. Evidence records
both outcomes. after.json holds all table assertions; gate.json holds compact
whole-node classifications and the frozen binary hash.

Remaining caption cross-iframe adoption assertion is existing realm-local Node
wrapper identity: DOM adoption places the caption under the correct table, but
reading its parent through the child realm returns a distinct wrapper. The
65,534-row span-limits layout fixture timed out before and was SIGKILLed in both
candidate runs; none of its assertions passed before. These classifications
are retained, not excluded. Other table failures are geometry/span limits.

HTML lane separately measures its complete
72-page parsing sweep on its newer parser base; table API gains are DOM bindings
gains and are not credited as parser algorithm changes.
