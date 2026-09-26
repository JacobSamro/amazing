# DOM-B DOMStringMap native area ready

Implementation `07a885b5`, fixed base `c7b0c3c2`. Live dataset reads/writes/deletes,
ASCII name conversion, UTF-16 values, attribute-order names and SameObject wrappers
now use native DOMStringMap. HTML, SVG and MathML expose the getter; arbitrary
namespaces no longer receive the old legacy dataset extension.

Linux isolated-target gate: **2733 passed / 10 skipped**, docs **4/4**;
baseline owned/driver **537 passed / 4 skipped**. The full 55-source collection
and global-attribute testharness selection improves **140→168 passing subtests**,
**135→107 failing**, **28 gains / zero lost passes**. Files **15→22 pass**,
**39→32 fail**, one harness error unchanged. All native dataset files pass except
the two prototype-accessor setter assertions in dataset-binding.window.js.
Those and numeric named-only set/define/delete discrimination are routed to JS.

Evidence: `crates/amazing-bindings/evidence/dataset/` (includes excluded reftest
inventory). The interrupted namespace gate was discarded; identical source was
rerun through linux-run recovery and completed. Windows remains deferred.
Next: inherited collection property surfaces; generic intrinsic array iteration
needs a routed JS LengthOfArrayLike fix. Iframe Range remains deferred.

Reproduce with `run-corpus.py before <base>` / `after <source>` after the build
and tests, using the pinned WPT revision in the JSON files. The final run copied
the binary immediately after build and used --reuse-binary after tests. It ran
on the source subsequently committed as the implementation SHA above.

Specs: https://html.spec.whatwg.org/multipage/dom.html#domstringmap and
https://w3c.github.io/mathml-core/#dom-and-javascript .
