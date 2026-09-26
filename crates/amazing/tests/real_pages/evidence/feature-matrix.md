# IA-REAL-PAGES feature matrix (2026-09-26)

Probed with `amazing run` on the Linux Boat sandbox at lane HEAD. These are the
globals/APIs the corpus pages touch; missing ones are the routed tickets.

| API | Result | Corpus page / script |
| --- | --- | --- |
| `Date.now` | **undefined** | `app-calendar`, `news-live` (`assets/clock.js`) |
| `undefined.x` / `null.x` | **returns undefined, no throw** | `news-broken-script` (`assets/broken.js`) |
| `navigator` | **ReferenceError** | `app-mail` (`feature-clipboard.js`) |
| `matchMedia` | **ReferenceError** | `app-dashboard` |
| `WebSocket` | **ReferenceError** | `app-chat` |
| `IntersectionObserver` | ReferenceError (page falls back) | `app-lazy-list`, `news-article` (`observer.js`) |
| `getComputedStyle` | **ReferenceError** | probe only |
| `requestIdleCallback` | **ReferenceError** | probe only |
| `Element.prototype.animate` | **undefined** | `app-kanban` (`feature-element-animate.js`) |
| `MutationObserver` | function | `app-notes` |
| `URLSearchParams` | function | `app-profile`, `shop-checkout` |
| `structuredClone` | function | `app-chat`, `app-calendar` |
| `JSON` / `history.pushState` / canvas `getContext` | present | `app-settings`, `app-todos`, `app-analytics` |

The corpus renders every page (`status=ok`, non-blank) with zero crashes; these
gaps are script errors or silent no-ops, not driver crashes.
