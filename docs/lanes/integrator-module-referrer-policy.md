# Module response referrer policy

Typed module transport processes all response Referrer-Policy fields in insertion order, and retains the last recognized nonempty token. Invalid trailing tokens cannot discard an earlier valid policy. Missing or invalid-only fields leave the inherited request option unchanged.

Normative source checked 2026-10-03: https://w3c.github.io/webappsec-referrer-policy/#parse-referrer-policy-from-header (section 8.1). Original implementation; no upstream code copied.

The existing installed-browser resource group uses three response fields: origin, garbage/no-referrer, and unknown/empty. The dependent redirected module request must receive no-referrer while retaining credentials and the real document context. Whole merged native/docs and corpus validation remains required before publication.
