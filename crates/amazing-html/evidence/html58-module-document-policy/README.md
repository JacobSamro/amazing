# HTML58 actual module document referrer policy

Fixed READY56 source3f6595f3 → source82232376. Only module_script_options
and grouped native fixtures change. Source57b9d57d then test correction82232376.

Initial external module requests and inline graph dependencies use the actual
script owner Document's generation-guarded response referrer policy. A valid
whole ASCII-case-insensitive referrerpolicy attribute overrides it; absent,
empty, invalid, whitespace-padded and comma-list attributes fall back to that
actual Document. No root/current-Document or URL-derived policy is substituted.
Existing module options snapshots, dependency response-header handling and
resource resolver remain intact.

Two native groups cover duplicate actual root response fields before first
external/inline graph fetch, dependency response headers, actual blank/srcdoc
inheritance, all attribute states and old owner Documents after frame retirement.
The initial native compile failure used a private context field;822 replaces
only that fixture lookup with the public actual frame-to-document query.

Boat:669 owned passed/6 skipped,4106 workspace passed/17 skipped,four doctests;
1894 CLI and1930 expanded html5lib passes/6 unsupported retained. Whole13 WPT
selections,111 module pages,whole220 syntax and192 CSSOM retain every accepted
passing name multiplicity and passing file, with exact pins:zero losses and
zero WPT gains. Accepted application/xml XHR remains passing. This records
module policy correctness, not parser or comparator gains.

The isolated source predates HTML57's PageScript generation field. When combined
with generation source d11c2780, native helper22ef9960 and five direct-binding
call correction3e6e6bc1 are mechanical test dependencies (both already supplied
to the facade identically before/after); they add actual DOM generation snapshots
without changing the module policy algorithm. Integration proof remains owned
by the integrator/facade; this gate does not claim it tested that combined source.

Exact refs/log hashes in gate.json, compact records and comparison.json; raw
reports preserved MAIN .agents/html/html58-corrected-after.
