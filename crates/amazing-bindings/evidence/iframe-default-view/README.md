# Iframe Document.defaultView and namespace dump fidelity

Fixed base `2eb1229a`; candidate binary/test source `20b74057` (includes the
already-gated NET-43 helper). Boat pool-dom `bx_na7t27mn`, native `/tmp` checkout
and Cargo target. Workspace nextest **3520 passed, 17 skipped**, doctests **4/4**.

Complete pinned WPT `dom/nodes`: **314 files**, assertions
**11385 -> 12028 pass**, **1593 -> 951 fail**, **643 gained, zero lost**.
Comparison counts `(file, subtest name)` multiplicities. Six assertion errors
remain unchanged. Both raw results include frozen binary hashes.

Iframe association fixes 642 assertions: createElementNS **376/220 -> 596/0**;
querySelectorAll **1601/374 -> 1885/90**; matches and webkitMatchesSelector each
**589/80 -> 658/11**. One additional XHR responseType=document assertion appears;
that recovery is not attributed to the iframe association fix.

The namespace dump change makes already-stored expanded attributes visible,
with canonical XML/XLink/XMLNS labels and preserved insertion order. Its native
test passes; two old dump-only expectations were corrected. HTML's html5lib
corpus measures diagnostic fidelity separately from parser behavior.

The recovered sandbox's HOME filesystem previously failed with missing rlibs,
SIGBUS and os107; failed attempts earn no gate credit. All successful final
builds/tests used native source and target directories. Shared WPT remained
read-only. No upstream code ported; no dependency added.

Reproduce through MAIN `scripts/linux-run.sh dom-bindings --host boat`: copy
the checkout to `/tmp`, link `corpora` to HOME, use a native Cargo target,
`cargo build -p amazing --release --locked`, workspace nextest and doctests,
then `dom-b-audit/run-corpus.py LABEL SOURCE --directory dom/nodes`.
