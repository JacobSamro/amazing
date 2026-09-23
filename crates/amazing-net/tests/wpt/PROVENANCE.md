# WPT URL corpus

Revision: `c48d58747e1f211527fb695fd60548a997fae617`

Source: https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/url/resources/urltestdata.json

Upstream raw SHA-256: `81e85fd3c199c08ef9c34cf651b3580eeedd080316493bfaf277a6b5ff8cf652`

All object records selected; comment strings are not test cases. Vendored JSON is
whitespace-normalized (one record per line); values and order are unchanged.
License: upstream-LICENSE.md. No external runtime/test dependency.

## NET-21 setter vectors

Source: https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/url/resources/setters_tests.json

Revision: `c48d58747e1f211527fb695fd60548a997fae617`; upstream raw SHA-256: `62e4b14bdaddd66b0f7d99bf066aed1c4a588667f4aa97bf26da212134fc24bf`.
All 278 object records are vendored unchanged. `generate_setters.py` turns them into Rust constants for the native `setters_wpt` runner. Its `--baseline` mode records behavior without applying a setter.

URLSearchParams assertion ports in `../urlsearchparams_wpt.rs` cite pinned `url/urlsearchparams-{append,delete,get,getall,has,set,sort,size,stringifier}.any.js` from the same revision. These are focused native ports; WebIDL conversions remain with dom-bindings.
