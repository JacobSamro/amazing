# HTML48 template forms, doctype identifiers, and meta quotes

Fixed base 319a363612a1f0fe402f3bf5f1d54c9818eaed7d; pinned WPT
c2a2fce69e5c2509d9dc28440ec8b42532e5c293. The complete 72-page
html/syntax/parsing testharness selection is recorded by syntax_sweep.py,
before.jsonl, after.jsonl, and syntax-comparison.json.

Ported the complete in-body and in-table form rules from MIT html5ever
6db7bbf1c4dfaecd28731950d427e40c8b911bef, adapting template-local forms,
pointer identity scope checks, and removal of non-current form stack entries
to Amazing's tree builder. Attribution and a new notice row accompany the port.
Current HTML additionally treats empty legacy HTML 4.01 system identifiers as
quirks, and meta content charset extraction requires a matching closing quote.

The final native Boat gate passed 575 owned tests (6 skipped), 3550 workspace
tests (17 skipped), and four doctests. All 62 tree files retained 1894/1894 CLI
passes; the expanded tree probe was identical to HTML47: 1930 passes and six
unsupported script-execution cases. The full 194-file encoding selection was
unchanged at 12216 passes/406 failures, with no former pass lost.

The syntax selection improved from 2646 passes/62 failures to 2650/58 with
zero lost passes. Files improved from 36 pass/14 fail to 39/11; two timeouts,
one error, and 19 unsupported files remain. Four actual parser gains: two
empty-system-ID doctype assertions, misnested-form-in-template, and quotes-in-meta.
Comparison uses exact subtest names, preserving multiplicity and asynchronous
ordering. Meaningful native regressions also cover every input split, nested
template and outer-form pointers, non-current form end tags, and table forms.

The remaining 43 template table assertions depend on DOM table interfaces;
five dynamic adoption assertions depend on mapped-node reparenting during DOM
import. These are routed to the DOM lane and are not credited as parser gains.

Sources: https://html.spec.whatwg.org/multipage/parsing.html,
https://github.com/servo/html5ever/tree/6db7bbf1c4dfaecd28731950d427e40c8b911bef.
Reproduce in one native-source linux-run.sh html --host boat command: release
amazing and html5lib_host builds; owned/workspace nextest; workspace doctests;
ensure-wpt.sh; scripts/html5lib.py with all 62 resource filenames; HTML45 run.py;
HTML46 encoding_sweep.py; this directory's syntax_sweep.py.
