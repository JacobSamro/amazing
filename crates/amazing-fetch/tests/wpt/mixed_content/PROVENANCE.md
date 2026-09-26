# Mixed-content native selection

WPT revision `c48d58747e1f211527fb695fd60548a997fae617`.
All four files in `mixed-content/tentative/autoupgrades/` are preserved
unaltered under `source/`. The `wpt_mixed_content` test checks their nine
media and CORS request-policy expectations. It does not run the HTML harness,
load media bytes, or cover other `mixed-content/` directories.

| File | SHA-256 | Native cases |
|---|---|---:|
| `audio-upgrade.https.sub.html` | `867bdb43b326ca0d98c52832ed3c8ed4f9ba4c1b449ff92e011a5c4825aa93a5` | 2 |
| `image-upgrade.https.sub.html` | `aad5bc7637a3b26357a81b679a587e6d38789af35685080060a416be16d372d4` | 2 |
| `mixed-content-cors.https.sub.html` | `df1db32d430ffc9294227af0e640809afe98cac1b8368710c288522e03805c12` | 3 |
| `video-upgrade.https.sub.html` | `c4859c90d19d87e57a6b18a8ad6ab3ac30d87225efeb36a8b569fecf556b3e66` | 2 |
