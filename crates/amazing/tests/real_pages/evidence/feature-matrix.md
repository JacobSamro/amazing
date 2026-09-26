# IA-REAL-PAGES feature matrix (2026-09-26)

Probed with `amazing run` on the merged head `5d484278` (Linux Boat). These are
the globals/APIs the corpus pages touch; missing ones are the routed tickets.

| API | Result | Corpus page / script |
| --- | --- | --- |
| `Date.now` / `Date.parse` | **undefined** | `app-calendar`, `news-live` (`assets/clock.js`) |
| `undefined.x` / `null.x` | **returns undefined, no throw** | `news-broken-script` (`assets/broken.js`) |
| `obj?.x` on nullish | undefined (correct) | probe |
| `navigator` | object | `app-mail` |
| `navigator.clipboard.writeText` | **undefined** | `app-mail` (`feature-clipboard.js`) |
| `matchMedia` | **undefined** | `app-dashboard` |
| `getComputedStyle` | **undefined** | probe only |
| `requestIdleCallback` | **undefined** | probe only |
| `WebSocket` | **undefined** | `app-chat` (`feature-websocket.js`) |
| `IntersectionObserver` | undefined (page falls back) | `app-lazy-list`, `news-article` (`observer.js`) |
| `Element.prototype.animate` | **undefined** | `app-kanban` (`feature-element-animate.js`) |
| `MutationObserver` | function | `app-notes` |
| `URLSearchParams` | function | `app-profile`, `shop-checkout` |
| `structuredClone` | function | `app-chat`, `app-calendar` |
| `JSON` / `history.pushState` / canvas `getContext` | present | `app-settings`, `app-todos`, `app-analytics` |

The corpus renders every page (`status=ok`, non-blank) with zero crashes; these
gaps are script errors or silent no-ops, not driver crashes. Two consecutive
harness runs were byte-identical in status, error count, fallback count and
nonwhite pixels at this head.
