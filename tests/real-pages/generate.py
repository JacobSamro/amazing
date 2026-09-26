#!/usr/bin/env python3
"""Generate the IA-REAL-PAGES corpus.

The corpus is synthesized (real-world pages cannot be redistributed) but mirrors
the structure and resource graphs of everyday sites: news, docs, wiki, shop and
web-app shells. Each page links shared and page-specific subresources (CSS, JS,
JSON, SVG images) through relative paths, so the driver exercises its real
resource loader. Output is deterministic; rerun to regenerate in place.

Usage: python3 tests/real-pages/generate.py
"""
from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parent
ASSETS = ROOT / "assets"
PAGES = ROOT / "pages"


def svg(width: int, height: int, fill: str, label: str) -> str:
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}"><rect width="{width}" height="{height}" fill="{fill}"/>'
        f'<text x="8" y="{height // 2 + 4}" font-family="sans-serif" font-size="12" '
        f'fill="#ffffff">{label}</text></svg>\n'
    )


ASSET_FILES: dict[str, str] = {
    "base.css": """\
:root{--fg:#1a1a1a;--muted:#667;--line:#dde;--accent:#0a58ca;--bg:#fff}
*{box-sizing:border-box}
body{margin:0;font:15px/1.5 -apple-system,Segoe UI,Roboto,sans-serif;color:var(--fg);background:var(--bg)}
header.site{display:flex;align-items:center;gap:16px;padding:10px 20px;border-bottom:1px solid var(--line);position:sticky;top:0;background:#fff}
header.site img{height:28px}
nav.main a{margin-right:12px;color:var(--accent);text-decoration:none}
main{max-width:960px;margin:0 auto;padding:20px}
h1{font-size:28px;margin:0 0 12px}
h2{font-size:20px;margin:22px 0 8px}
a{color:var(--accent)}
.muted{color:var(--muted)}
.card{border:1px solid var(--line);border-radius:8px;padding:14px;margin:10px 0}
.grid{display:grid;grid-template-columns:repeat(3,1fr);gap:14px}
.flex{display:flex;gap:14px;flex-wrap:wrap}
table{border-collapse:collapse;width:100%}
th,td{border:1px solid var(--line);padding:6px 8px;text-align:left}
aside.sidebar{float:right;width:240px;margin-left:16px}
footer.site{border-top:1px solid var(--line);padding:16px 20px;color:var(--muted)}
form label{display:block;margin:8px 0 2px}
input,select,textarea{padding:6px;border:1px solid var(--line);border-radius:4px;width:100%;max-width:420px}
button{padding:8px 14px;border:0;border-radius:4px;background:var(--accent);color:#fff;cursor:pointer}
.badge{display:inline-block;padding:1px 7px;border-radius:10px;background:#eef;font-size:12px}
.price{font-weight:600}
""",
    "theme-dark.css": """\
body.dark{--fg:#eee;--muted:#99a;--line:#334;--accent:#7ab7ff;--bg:#12141a}
body.dark header.site{background:#12141a}
""",
    "site.js": """\
(function () {
  'use strict';
  document.documentElement.classList.add('js');
  var toggle = document.getElementById('nav-toggle');
  if (toggle) {
    toggle.addEventListener('click', function () {
      document.body.classList.toggle('nav-open');
    });
  }
  var links = document.querySelectorAll('nav.main a');
  for (var i = 0; i < links.length; i++) {
    if (links[i].getAttribute('href') === document.location.pathname) {
      links[i].setAttribute('aria-current', 'page');
    }
  }
  window.__siteReady = true;
})();
""",
    "analytics.js": """\
var analyticsQueue = analyticsQueue || [];
function track(name, props) {
  var t = (typeof Date.now === 'function') ? Date.now() : 0;
  analyticsQueue.push({ name: name, props: props || {}, t: t });
}
track('pageview', { title: document.title });
""",
    "clock.js": """\
(function () {
  var now = Date.now();
  var host = document.getElementById('clock');
  if (host) { host.textContent = String(now); }
})();
""",
    "widgets.js": """\
function renderCard(title, body) {
  var card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = '<h3>' + title + '</h3><p>' + body + '</p>';
  return card;
}
document.addEventListener('DOMContentLoaded', function () {
  var host = document.getElementById('widgets');
  if (host) {
    host.appendChild(renderCard('Related', 'Rendered by widgets.js'));
    host.appendChild(renderCard('Popular', 'Second generated card'));
  }
});
""",
    "app.js": """\
(function () {
  'use strict';
  var state = { count: 0 };
  function render() {
    var out = document.getElementById('counter');
    if (out) { out.textContent = String(state.count); }
  }
  var button = document.getElementById('increment');
  if (button) {
    button.addEventListener('click', function () { state.count += 1; render(); });
  }
  render();
  requestAnimationFrame(function () { document.body.dataset.frame = '1'; });
})();
""",
    "fetch-data.js": """\
fetch('../assets/data.json').then(function (response) {
  return response.json();
}).then(function (data) {
  var host = document.getElementById('remote');
  if (host) { host.textContent = data.title; }
}).catch(function (error) {
  var host = document.getElementById('remote');
  if (host) { host.textContent = 'unavailable'; }
});
""",
    "storage.js": """\
try {
  localStorage.setItem('last-visit', String(Date.now()));
  var seen = localStorage.getItem('last-visit');
  var host = document.getElementById('seen');
  if (host) { host.textContent = seen ? 'returning' : 'new'; }
} catch (error) {
  var host = document.getElementById('seen');
  if (host) { host.textContent = 'unknown'; }
}
""",
    "observer.js": """\
if (typeof IntersectionObserver === 'function') {
  var observer = new IntersectionObserver(function (entries) {
    entries.forEach(function (entry) {
      if (entry.isIntersecting) { entry.target.classList.add('visible'); }
    });
  });
  document.querySelectorAll('.lazy').forEach(function (node) { observer.observe(node); });
} else {
  document.querySelectorAll('.lazy').forEach(function (node) { node.classList.add('visible'); });
}
""",
    "broken.js": """\
(function () {
  var value = window.__missing_config.deep.value;
  console.log('unreachable ' + value);
})();
""",
    "data.json": """\
{"title":"Remote data loaded","items":[1,2,3],"ok":true}
""",
}

# One unguarded call per file, so a missing global or interface is attributed to
# a single feature instead of aborting a larger bundle.
ASSET_FILES.update({
    "feature-matchmedia.js": "matchMedia('(min-width: 600px)').matches;\n",
    "feature-websocket.js": "new WebSocket('ws://127.0.0.1:9/');\n",
    "feature-canvas.js": "document.createElement('canvas').getContext('2d').fillRect(0,0,1,1);\n",
    "feature-clipboard.js": "navigator.clipboard.writeText('x').then(function(){}, function(){});\n",
    "feature-mutationobserver.js": "new MutationObserver(function(){}).observe(document.body, {attributes:true});\n",
    "feature-element-animate.js": "document.body.animate([{opacity:1},{opacity:0}], 100);\n",
    "feature-urlparams.js": "new URLSearchParams('a=1&b=2').get('a');\n",
    "feature-history.js": "history.pushState({x:1}, '', '/next');\n",
    "feature-json.js": "JSON.stringify(JSON.parse('{\\\"a\\\":1}'));\n",
    "feature-structuredclone.js": "structuredClone({a:[1,2,3]});\n",
})

for name, fill in [
    ("logo.svg", (120, 32, "#0a58ca", "amazing")),
    ("hero.svg", (800, 240, "#134e8a", "hero")),
    ("thumb-1.svg", (160, 120, "#2f6f4e", "1")),
    ("thumb-2.svg", (160, 120, "#8a5a13", "2")),
    ("thumb-3.svg", (160, 120, "#7a1f3d", "3")),
    ("thumb-4.svg", (160, 120, "#3b3f8a", "4")),
    ("thumb-5.svg", (160, 120, "#5a5a5a", "5")),
    ("thumb-6.svg", (160, 120, "#0f6d7a", "6")),
]:
    ASSET_FILES[name] = svg(*fill)


def head(title: str, extra_css: list[str] | None = None, head_scripts: list[str] | None = None) -> str:
    extra_css = extra_css or []
    head_scripts = head_scripts or []
    css = "".join(
        f'<link rel="stylesheet" href="../assets/{sheet}">' for sheet in ["base.css", *extra_css]
    )
    scripts = "".join(f'<script src="../assets/{src}"></script>' for src in head_scripts)
    return (
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n"
        f"<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n"
        f"<title>{title}</title>\n{css}\n{scripts}\n</head>\n"
    )


def header(site: str) -> str:
    return (
        '<header class="site">'
        '<img src="../assets/logo.svg" alt="logo">'
        f'<strong>{site}</strong>'
        '<nav class="main"><a href="index.html">Home</a>'
        '<a href="section.html">Section</a><a href="search.html">Search</a></nav>'
        '<button id="nav-toggle" type="button">Menu</button>'
        "</header>"
    )


def footer(site: str) -> str:
    return f'<footer class="site">© {site} — synthesized corpus page</footer>\n'


def body_grid() -> str:
    return (
        '<div id="widgets"></div>'
        '<div class="grid">'
        + "".join(
            f'<div class="card"><img src="../assets/thumb-{i}.svg" alt="thumb {i}">'
            f'<h3>Story {i}</h3><p class="muted">Summary line for story {i}.</p></div>'
            for i in range(1, 7)
        )
        + "</div>"
    )


def body_table(rows: int = 6) -> str:
    cells = "".join(
        f"<tr><td>row {i}</td><td>{i * 7}</td><td>ok</td></tr>" for i in range(1, rows + 1)
    )
    return (
        "<table><thead><tr><th>Name</th><th>Value</th><th>Status</th></tr></thead>"
        f"<tbody>{cells}</tbody></table>"
    )


def body_form() -> str:
    return (
        '<form onsubmit="track(\'submit\'); return false;">'
        '<label for="q">Query</label><input id="q" name="q" value="browser engine">'
        '<label for="kind">Kind</label><select id="kind"><option>news</option><option>docs</option></select>'
        '<label for="note">Note</label><textarea id="note" rows="3">hello</textarea>'
        '<p><button type="submit">Send</button></p></form>'
    )


def docs_sidebar() -> str:
    return (
        '<aside class="sidebar"><div class="card"><h3>On this page</h3>'
        '<ul><li><a href="#intro">Intro</a></li><li><a href="#api">API</a></li>'
        '<li><a href="#examples">Examples</a></li></ul></div></aside>'
    )


# (category, slug, title, body, extra_css, head_scripts, deferred_scripts)
def pages() -> list[dict]:
    out: list[dict] = []

    def add(category, slug, title, body, extra_css=None, head_scripts=None, scripts=None):
        out.append({
            "category": category,
            "slug": slug,
            "title": title,
            "body": body,
            "extra_css": extra_css or [],
            "head_scripts": head_scripts or ["analytics.js"],
            "scripts": scripts or ["site.js"],
        })

    # news
    add("news", "home", "Daily Ledger — Home",
        f'<main><h1>Top stories</h1>{body_grid()}<h2>Editor picks</h2>{body_table()}</main>',
        scripts=["site.js", "widgets.js"])
    add("news", "article", "City council approves transit plan",
        '<main><article><h1>City council approves transit plan</h1>'
        '<p class="muted">By A. Reporter · 4 min read</p>'
        '<img src="../assets/hero.svg" alt="transit">'
        '<p>The council voted 7-2 to fund the plan after a long debate.</p>'
        '<h2>What changes</h2><p>Three new lines open by 2030.</p>'
        f'{body_form()}</article></main>',
        scripts=["site.js", "observer.js"])
    add("news", "section", "World — Daily Ledger",
        f'<main><h1>World</h1>{body_grid()}</main>', scripts=["site.js", "widgets.js"])
    add("news", "live", "Live updates: storm coverage",
        '<main><h1>Live updates</h1><p class="muted">clock <span id="clock"></span></p><div id="live">'
        '<div class="card"><strong>09:12</strong> Power restored downtown.</div>'
        '<div class="card"><strong>08:40</strong> Schools closed.</div></div></main>',
        scripts=["site.js", "app.js", "clock.js"])
    add("news", "gallery", "Photos of the week",
        f'<main><h1>Photos of the week</h1>{body_grid()}</main>',
        scripts=["site.js", "observer.js"])
    add("news", "opinion", "Opinion: The case for slower streets",
        '<main><article><h1>The case for slower streets</h1>'
        '<p>Lower speed limits save lives at modest cost.</p></article></main>',
        scripts=["site.js"])
    add("news", "search", "Search — Daily Ledger",
        f'<main><h1>Search</h1>{body_form()}<h2>Results</h2>{body_table()}</main>',
        head_scripts=["analytics.js"], scripts=["site.js", "fetch-data.js"])
    add("news", "tag", "Tag: transit",
        f'<main><h1>#transit</h1>{body_grid()}</main>', scripts=["site.js", "widgets.js"])
    add("news", "author", "A. Reporter — archive",
        f'<main><h1>A. Reporter</h1><p class="muted">312 articles</p>{body_table()}</main>',
        scripts=["site.js"])
    add("news", "subscribe", "Subscribe",
        f'<main><h1>Subscribe</h1>{body_form()}</main>',
        scripts=["site.js", "storage.js"])

    # docs
    add("docs", "index", "Amazing Docs",
        f'<main><h1>Amazing docs</h1><p class="muted">Guides and reference.</p>{body_grid()}</main>',
        scripts=["site.js", "widgets.js"])
    add("docs", "guide", "Getting started",
        f'<main>{docs_sidebar()}<h1>Getting started</h1><p>Install and run a page.</p>'
        f'<pre><code>amazing render page.html out.bmp</code></pre></main>', scripts=["site.js"])
    add("docs", "api", "API reference",
        f'<main>{docs_sidebar()}<h1>API reference</h1>{body_table()}</main>', scripts=["site.js"])
    add("docs", "reference", "Reference: layout",
        f'<main>{docs_sidebar()}<h1>Layout</h1><h2 id="intro">Intro</h2>'
        f'<p>Block and inline formatting contexts.</p><h2 id="api">API</h2>{body_table()}</main>',
        scripts=["site.js"])
    add("docs", "changelog", "Changelog",
        f'<main><h1>Changelog</h1>{body_table()}</main>', scripts=["site.js"])
    add("docs", "faq", "FAQ",
        '<main><h1>FAQ</h1><details><summary>Does it work offline?</summary>'
        '<p>Yes for saved pages.</p></details></main>', scripts=["site.js"])
    add("docs", "search", "Docs search",
        f'<main><h1>Search docs</h1>{body_form()}</main>',
        scripts=["site.js", "fetch-data.js"])
    add("docs", "tutorial", "Tutorial: your first page",
        f'<main>{docs_sidebar()}<h1>Your first page</h1><p>Write HTML, then render it.</p>'
        f'{body_form()}</main>', scripts=["site.js"])
    add("docs", "migration", "Migration guide",
        f'<main>{docs_sidebar()}<h1>Migration</h1><p>Move off the old API.</p>{body_table()}</main>',
        scripts=["site.js"])
    add("docs", "cli", "CLI",
        f'<main>{docs_sidebar()}<h1>CLI</h1>{body_table()}</main>', scripts=["site.js"])

    # wiki
    add("wiki", "article", "Browser engine — Wiki",
        f'<main>{docs_sidebar()}<h1>Browser engine</h1>'
        '<div class="card"><strong>Infobox</strong><p class="muted">Written in Rust.</p></div>'
        f'<h2>History</h2><p>Started as an exercise.</p></main>', scripts=["site.js"])
    add("wiki", "talk", "Talk: Browser engine",
        f'<main><h1>Talk</h1>{body_table()}</main>', scripts=["site.js"])
    add("wiki", "category", "Category: Engines",
        f'<main><h1>Category: Engines</h1>{body_grid()}</main>', scripts=["site.js", "widgets.js"])
    add("wiki", "diff", "Diff: revision 42",
        '<main><h1>Diff</h1><pre class="muted">- old line\n+ new line</pre></main>', scripts=["site.js"])
    add("wiki", "edit", "Editing Browser engine",
        f'<main><h1>Editing</h1>{body_form()}</main>', scripts=["site.js", "storage.js"])
    add("wiki", "history", "Revision history",
        f'<main><h1>History</h1>{body_table()}</main>', scripts=["site.js"])
    add("wiki", "user", "User: Editor",
        f'<main><h1>Editor</h1><p class="muted">Joined 2019</p>{body_table()}</main>', scripts=["site.js"])
    add("wiki", "portal", "Community portal",
        f'<main><h1>Portal</h1>{body_grid()}</main>', scripts=["site.js", "widgets.js"])
    add("wiki", "file", "File: Diagram.svg",
        '<main><h1>File</h1><img src="../assets/hero.svg" alt="diagram"></main>', scripts=["site.js"])
    add("wiki", "template", "Template: Infobox",
        f'<main><h1>Template</h1>{body_table()}</main>', scripts=["site.js"])

    # shop
    add("shop", "home", "Shop — Home",
        f'<main><h1>Featured</h1>{body_grid()}</main>', scripts=["site.js", "widgets.js"])
    add("shop", "category", "Category: Mugs",
        f'<main><h1>Mugs</h1><div class="flex">{body_grid()}</div></main>',
        scripts=["site.js", "widgets.js"])
    add("shop", "product", "Amazing Mug",
        '<main><h1>Amazing Mug</h1><img src="../assets/thumb-1.svg" alt="mug">'
        '<p class="price">$14.00</p><p class="badge">In stock</p>'
        '<button id="increment" type="button">Add to cart</button>'
        '<div class="card"><h2>Reviews</h2><p>“Sturdy.”</p></div></main>',
        scripts=["site.js", "app.js"])
    add("shop", "cart", "Cart",
        f'<main><h1>Cart</h1>{body_table()}<p class="price">Total: $42.00</p></main>',
        scripts=["site.js", "storage.js"])
    add("shop", "checkout", "Checkout",
        f'<main><h1>Checkout</h1>{body_form()}</main>', scripts=["site.js", "storage.js"])
    add("shop", "search", "Product search",
        f'<main><h1>Search</h1>{body_form()}<div id="remote"></div></main>',
        scripts=["site.js", "fetch-data.js"])
    add("shop", "reviews", "All reviews",
        f'<main><h1>Reviews</h1>{body_table()}</main>', scripts=["site.js"])
    add("shop", "account", "Account",
        f'<main><h1>Account</h1>{body_form()}<p id="seen"></p></main>',
        scripts=["site.js", "storage.js"])
    add("shop", "wishlist", "Wishlist",
        f'<main><h1>Wishlist</h1>{body_grid()}</main>', scripts=["site.js", "widgets.js"])
    add("shop", "order", "Order #1007",
        f'<main><h1>Order #1007</h1>{body_table()}</main>', scripts=["site.js"])

    # app shells
    add("app", "dashboard", "Dashboard",
        '<main><h1>Dashboard</h1><div class="grid">'
        '<div class="card"><h3>Users</h3><p id="counter">0</p>'
        '<button id="increment" type="button">+1</button></div>'
        '<div class="card"><h3>Revenue</h3><p>$12k</p></div>'
        '<div class="card"><h3>Churn</h3><p>2.1%</p></div></div></main>',
        scripts=["site.js", "app.js"])
    add("app", "todos", "Todos",
        f'<main><h1>Todos</h1>{body_form()}<div id="widgets"></div></main>',
        scripts=["site.js", "widgets.js"])
    add("app", "chat", "Chat",
        f'<main><h1>Chat</h1>{body_table()}</main>', scripts=["site.js", "app.js"])
    add("app", "settings", "Settings",
        f'<main><h1>Settings</h1>{body_form()}</main>',
        scripts=["site.js", "storage.js"])
    add("app", "analytics", "Analytics",
        f'<main><h1>Analytics</h1>{body_table(8)}</main>', scripts=["site.js", "fetch-data.js"])
    add("app", "mail", "Mail",
        f'<main><h1>Mail</h1>{body_table()}</main>', scripts=["site.js"])
    add("app", "calendar", "Calendar",
        f'<main><h1>Calendar</h1><p class="muted">now <span id="clock"></span></p>{body_table()}</main>',
        scripts=["site.js", "app.js", "clock.js"])
    add("app", "notes", "Notes",
        f'<main><h1>Notes</h1>{body_form()}<div id="widgets"></div></main>',
        scripts=["site.js", "widgets.js", "storage.js"])
    add("app", "kanban", "Kanban",
        f'<main><h1>Kanban</h1><div class="flex">'
        '<div class="card"><h3>To do</h3><p>3</p></div>'
        '<div class="card"><h3>Doing</h3><p>1</p></div>'
        '<div class="card"><h3>Done</h3><p>9</p></div></div></main>',
        scripts=["site.js", "app.js"])
    add("app", "profile", "Profile",
        f'<main><h1>Profile</h1>{body_form()}<p id="seen"></p></main>',
        scripts=["site.js", "storage.js"])

    # Two deliberately brittle pages: one throws in a script, one probes an
    # unshipped API. They exercise the lenient path and the fallback classifier.
    add("news", "broken-script", "Sponsored — Daily Ledger",
        f'<main><h1>Sponsored</h1><p>Advertorial content.</p>{body_table()}</main>',
        scripts=["site.js", "broken.js"])
    add("app", "lazy-list", "Lazy list",
        f'<main><h1>Lazy list</h1>{body_grid()}</main>',
        scripts=["site.js", "observer.js", "widgets.js"])

    # Attach one unguarded feature probe to each app shell (and a couple of shop
    # pages) so missing globals/interfaces surface as per-page fallbacks.
    feature_by_app = {
        "dashboard": ["feature-matchmedia.js", "feature-mutationobserver.js"],
        "chat": ["feature-websocket.js", "feature-structuredclone.js"],
        "analytics": ["feature-canvas.js"],
        "mail": ["feature-clipboard.js"],
        "calendar": ["feature-structuredclone.js"],
        "notes": ["feature-mutationobserver.js"],
        "kanban": ["feature-element-animate.js"],
        "profile": ["feature-urlparams.js"],
        "todos": ["feature-history.js"],
        "settings": ["feature-json.js"],
    }
    for entry in out:
        if entry["category"] == "app" and entry["slug"] in feature_by_app:
            entry["scripts"].extend(feature_by_app[entry["slug"]])
        if entry["category"] == "shop" and entry["slug"] == "checkout":
            entry["scripts"].extend(["feature-history.js", "feature-urlparams.js"])

    return out


def build_page(entry: dict) -> str:
    category = entry["category"]
    site = {"news": "Daily Ledger", "docs": "Amazing Docs", "wiki": "Wiki",
            "shop": "Shop", "app": "Console"}[category]
    deferred = "".join(f'<script defer src="../assets/{src}"></script>' for src in entry["scripts"])
    return (
        head(entry["title"], entry["extra_css"], entry["head_scripts"])
        + "<body>\n"
        + header(site)
        + entry["body"]
        + "\n"
        + footer(site)
        + deferred
        + "\n</body>\n</html>\n"
    )


def main() -> None:
    entries = pages()
    assert len(entries) == 52, f"expected 52 pages, got {len(entries)}"
    slugs = {(e["category"], e["slug"]) for e in entries}
    assert len(slugs) == len(entries), "duplicate page slug"
    if ASSETS.exists():
        shutil.rmtree(ASSETS)
    if PAGES.exists():
        shutil.rmtree(PAGES)
    ASSETS.mkdir(parents=True)
    PAGES.mkdir(parents=True)
    for name, content in ASSET_FILES.items():
        (ASSETS / name).write_text(content, encoding="utf-8", newline="\n")
    manifest = []
    for entry in entries:
        html = build_page(entry)
        path = PAGES / f'{entry["category"]}-{entry["slug"]}.html'
        path.write_text(html, encoding="utf-8", newline="\n")
        manifest.append({
            "page": f'pages/{path.name}',
            "category": entry["category"],
            "title": entry["title"],
        })
    (ROOT / "manifest.json").write_text(
        json.dumps({"schema": 1, "pages": manifest}, indent=2) + "\n",
        encoding="utf-8", newline="\n",
    )
    print(f"wrote {len(manifest)} pages and {len(ASSET_FILES)} assets to {ROOT}")


if __name__ == "__main__":
    main()
