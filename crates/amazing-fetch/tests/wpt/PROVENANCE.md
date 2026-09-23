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

## NET-20 cookie HTTP cases

At the same pinned revision, the former `cookies/http-state/` parser corpus is
no longer present. The standalone HTTP-header vectors are embedded in HTML.
`cookies/source/` contains these unedited upstream files; `cookies/generate.mjs`
extracts their `httpCookieTest` and `httpRedirectCookieTest` calls to
`cookies/cases.rs` (Node is needed only when regenerating fixtures). Run them
with `cargo run -p amazing-fetch --example cookies_wpt --locked` on win-001.
The runner gives each case a fresh jar and uses the paths and redirect hosts
from WPT's cookie helper, with a fixed 2026-09-23 clock.

| Selection under WPT `cookies/` | Raw SHA-256 | HTTP cases |
|---|---|---:|
| `attributes/max-age.html` | `9a6fe4fd1edb9a8cd094a99b4a6fc191a31fbb698fcc25a324ceec99aa30` | 10 |
| `attributes/expires.html` | `0c237f0247b1c1c59b74e832ece358b77c70173f3594dd4faa83351dda95a451` | 11 |
| `attributes/path.html` | `68902e72051ee0e165af2c7ab31d3b989be373d6cc6ef2e08887b86923cb4c85` | 21 |
| `attributes/invalid.html` | `d0f32bb78e706b51fc2e6fb7a3ddce2e8f1add43631c6aff25a324ceec99aa30` | 26 |
| `name/name.html` | `d3fca366a5390de24634890d07f5dc74649c3cdb1c5e053352168b5cae87a2af` | 45 |
| `size/name-and-value.html` | `6ee5feac2958578f871bf7762adeac0e4431a4456ec13d8f28e5f3b7c3d42262` | 11 |
| `size/attributes.www.sub.html` | `618ab9bbfb1ea28db70f41de63b0f2c853cffb3d96180de775d84192d9ae6b4f` | 14 |
| `attributes/resources/domain-child.sub.html` | `a83028d4c41bc6d1b81213d33208e651351ab62c425a320939e1b474648d4673` | 54 |

Total: 192 HTTP-header cases. The `expires.html` document.cookie duplicates
and tests needing browser navigation, scripting, or multiple origins remain
outside this native selection. SameSite and cookie-prefix behavior are covered
by the crate's focused storage tests; WPT's corresponding files require the
browser harness and are not counted here.
