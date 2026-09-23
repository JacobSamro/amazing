# NET-17 pinned WPT JSON inputs

Revision: `c48d58747e1f211527fb695fd60548a997fae617` of
`web-platform-tests/wpt`. These are upstream JSON files, preserved without
editing. `generate.py` converts their assertions to std-only Rust constants;
`examples/corpora_wpt.rs` runs every case and prints each mismatch and a
per-corpus summary. The original WPT license is in `upstream-LICENSE.md`.

| Selection under WPT root | Raw SHA-256 | Cases |
|---|---|---:|
| `fetch/data-urls/resources/data-urls.json` | `b3be3d6e1d0eb7f1b5b20d7410d30cb76216d41c9fe3d8364446abecff88bba5` | 72 |
| `fetch/data-urls/resources/base64.json` | `0cdb75b4fcc46e4a6caa0e3b0005157a0d0be03e00bcf7b7054a045b1267568d` | 80 |
| `mimesniff/mime-types/resources/mime-types.json` | `016d3dd91b647e03af2a69c7ec241bf787df8bffe7953018fe566e8ec5d7b822` | 74 object cases; section labels excluded |
| `mimesniff/mime-types/resources/generated-mime-types.json` | `20924495060ac9633f10d57a326b95a5987863f27c5d4765b13b744304b33fed` | 881 |

The data URL selection tests URL parsing plus Fetch's data URL processor. The
base64 selection prepends `data:;base64,` and tests the same processor, including
Infra's forgiving-base64 decoder. The MIME
selections compare the MIME Sniffing §4 serialization, including failure.
They do not run WPT's JavaScript harness or all Fetch/MIME WPT tests.
