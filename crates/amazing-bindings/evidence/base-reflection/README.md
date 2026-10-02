# HTMLBaseElement reflection and URL resolution

Fixed base 811ce5ea (gated HTML serializer/context API); pinned C2 WPT whole
html/semantics/document-metadata/the-base-element directory (11 files) and 314 DOM node files.
Baseline base 3 passed, 13 failed; the baseline node sweep missed the known XHR deadline
assertion, so the previous accepted serializer report also guards zero loss.

HTMLBaseElement.href is USVString with reflected setter and a dedicated getter:
absent attribute is parsed as empty against owner-document fallback URL metadata,
ignoring itself and every other base element. Invalid URL returns original scalar
string. target reflects DOMString, preserving non-scalar UTF-16. Existing general
URL resolution now rejects data/javascript first-base URLs per the frozen-base
selection algorithm, returning to the document fallback instead of resolving
relative URLs against an opaque prohibited base.

Native tests cover missing/relative/invalid href, two-base independence, element
URL inheritance, invalid/prohibited first bases, USV/DOMString conversion,
conversion callbacks, receiver checks and a detached base's cross-document
Shift_JIS query encoding. No engine code copied or dependencies added.

Existing runtime limitations: no captured about-base URL state for about:blank
or srcdoc, no CSP base-uri policy hook and no durable frozen-base URL history.
The HTML owner has the iframe/context state needed for a separate coordinated
fallback API ticket; inert about:blank documents must not be assumed to inherit
root context. Pinned about_blank/srcdoc cases currently stop earlier at missing
Window.frames. Navigation/product and Window state failures remain in evidence.

Algorithm: https://html.spec.whatwg.org/multipage/semantics.html#the-base-element
Native Boat workspace: 3,642 passed, 17 skipped; four doctests passed. Whole base directory: 3 passed/13 failed → 13 passed/3 failed (+10), zero lost. Full DOM nodes retain the accepted 12,692 passing assertions, 288 failures and six errors, with zero gains/losses credited. The initial full run missed the existing XHR deadline assertion; the complete 325-file unchanged-source recheck restored it. Reports preserve unsupported/error/timeout classes and native source hashes.
