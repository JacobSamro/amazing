# Chromium HSTS preload snapshot

Source: `net/http/transport_security_state_static.json` at Chromium commit
`d5e6fd51b430fec89732a3976e666011ecffa0a2`.
Source URL: https://raw.githubusercontent.com/chromium/chromium/d5e6fd51b430fec89732a3976e666011ecffa0a2/net/http/transport_security_state_static.json
Source SHA-256: `22a2a6e72ea571ba80ec478fda90c65fcfad54b1d2b3f8015008bc3c8b88c9d6`.

`chromium-preload.bin` contains 94778 `force-https` entries sorted by
ASCII hostname. Each record is one byte of name length, that many ASCII
name bytes, then one byte for `include_subdomains`. SHA-256:
`dd2a2860576801f7ce6cdc652fb4c312683b8517ad1375b6f5827e72588a74e7`. The source is governed by Chromium's
BSD-style license; see https://chromium.googlesource.com/chromium/src/+/main/LICENSE.
