# tests/real-pages — IA-REAL-PAGES corpus

A ~52-page synthesized corpus of everyday browsing: news, docs, wiki, shop and
web-app shells. Pages are **synthesized** rather than saved copies, because the
originals cannot be redistributed; each mirrors a real site's structure and
resource graph and links shared subresources through relative paths, so the
driver exercises its real loader, script execution and event loop.

- `generate.py` — deterministic generator. Rerun to regenerate `assets/`,
  `pages/` and `manifest.json` in place.
- `pages/` — 52 HTML pages (`<category>-<slug>.html`).
- `assets/` — shared CSS, classic/defer JS, JSON and SVG images, plus one
  unguarded probe script per feature (`feature-*.js`).
- `manifest.json` — page list with categories.

The corpus is a driver regression net, not conformance credit. It is run by
`crates/amazing/tests/real_pages.rs` through the lenient headless
`amazing render-page` command; the test fails on a crash, panic, timeout, blank
render or missing image, and prints a per-page table. The latest table and a
feature matrix are under `crates/amazing/tests/real_pages/evidence/`.

Run it explicitly:

```
cargo nextest run -p amazing --locked real_pages --no-capture
# persist the table:
AMAZING_REAL_PAGES_EVIDENCE=/tmp/run.tsv cargo nextest run -p amazing --locked real_pages --no-capture
# one page:
AMAZING_REAL_PAGES_ONLY=news-article cargo nextest run -p amazing --locked real_pages --no-capture
```
