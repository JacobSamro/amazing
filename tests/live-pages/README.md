# tests/live-pages — IA-LIVE-PAGES live corpus

A ~40-page selection of **public, no-login** everyday sites (news, docs, wiki,
code hosting, shop product pages, heavy SPA landing pages). Pages are fetched
live through Amazing's own network stack (`amazing-net` with TLS); nothing is
saved or redistributed, so there is no licensing question. A page that blocks
bots or times out is recorded as a fetch/HTTP classification, not a crash.

- `urls.txt` — the corpus, one URL per line (`#` comments allowed).
- `probe.js` — runs in each page's interpreter after its scripts and prints one
  `API|<path>|<typeof>` line per common web API. The harness turns `undefined`
  into the top-missing-APIs report.

Run it explicitly on a network-enabled host (the Boat sandbox). It is `#[ignore]`
so the normal `--workspace` gate never needs the network:

```
scripts/linux-run.sh integrator-assist -- \
  'AMAZING_LIVE_PAGES_EVIDENCE=$HOME/live-evidence \
   cargo nextest run -p amazing --locked --run-ignored -E "test(live_pages)" --no-capture'
```

Evidence (`run.tsv`, `top-missing-apis.md`, `errors.md`, `warnings.md`) is written
under `crates/amazing/tests/live_pages/evidence/` by default, or to
`$AMAZING_LIVE_PAGES_EVIDENCE` when set. This corpus is a robustness net, not
conformance credit.

`warnings.md` clusters the renderer's per-resource warnings (`UnsupportedSvg`,
unsupported image/font formats) by kind; `errors.md` clusters page-script and
fetch failures; `top-missing-apis.md` counts probed APIs whose `typeof` is
`undefined` per page.
