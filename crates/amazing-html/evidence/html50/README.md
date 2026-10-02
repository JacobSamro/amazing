# HTML50 durable parser and browsing-context scripting state

The fixed dependency baseline is ba5ec2f5: HTML49 plus the DOM scripting-state
API6fcb63ce, ready charset8a816d91, and table5c26640a. Serializer811ce5ea is
adopted as475fcb26. No DOM implementation is modified by this ticket.

Every complete, incremental, resumable, byte and fragment HTML parser stores
its configured ScriptingMode on the document. Legacy parser entry points keep
Enabled. Explicit disabled and inert XML parsers remain disabled. The iframe
adapter stores the actual browsing-context sandbox setting before navigation
callbacks and after each incremental graft, including HTML, XML, XHTML, and
initial about:blank documents. MIME does not determine scripting state.

document.write selects fragment parsing from the target document's actual
state and executes inserted scripts only when scripting is enabled. Grouped
native regressions cover parser chunk boundaries, normal/shadow fragments,
sixteen iframe source/sandbox combinations, and inert document.write.

The dependency-aware baseline encoding194 is12268pass354fail, syntax72 is
2694pass14fail, and DOM314 is12363pass281fail. Relative to the earlier API-only
baseline, table/charset adoption adds44 syntax and654 DOM assertions. These
are dependency gains, not HTML50 parser gains.

The full native Boat gate passes586owned tests/6skipped,3567workspace tests/
17skipped, and4doctests. Tree CLI retains1894/1894; expanded tree selection
retains1930pass6unsupported. Encoding194 and syntax72 preserve all exact
subtest-name/status multiplicities. Matched25s whole DOM314 improves12363/280 to12369/274, six adopted
serializer gains and zero lost passing assertions, including against the
original10s baseline. Both runs retain16timeouts6errors69unsupported. The
eight-file serialization selection improves352/23 to372/3, twenty adopted
serializer gains and zero losses. HTML50 context glue has no separate WPT
gain on these selections; its configured-mode/sandbox/inert native tests pass.

context_sweep.py inventories the entire requested pinned WPT directory,
rejects empty selections, preserves unsupported/errors/timeouts, and checks
driver provenance and complete subtest counts. compare.py compares exact
subtest-name byte hashes with multiplicity and fails on any lost passing
assertion. The corrected serialization directory is
html/syntax/serializing-html-fragments (eight files). The initial empty
selection is excluded from evidence. The first10s DOM run timed out on one
previously passing XHR assertion; matched whole runs use the standard25s
deadline without repeating the successful native test gate.

Remaining escaping.html failures are synchronous document.write visibility,
template contents owner-document separation, and missing
Range.createContextualFragment. The first is the next HTML parser integration
area; the latter two remain DOM-owned. Matched binaries retain identical
hashes at start and end. Full name-byte hashes/status multiplicities and
comparison records are durable; orchestration scratch retains raw reports.
No new port or engine dependency is introduced.
