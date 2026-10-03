# Actual pending-I/O clock boundary

Fixed before `376dada37ad07042e7f97c5f6208b4cfcf347ee9`, after `592e94cc51a4867d646eb90d9e4ce2f41e182a1c` (algorithm539a3d41 plus boundary coverage592e94cc). Pure Dispatcher/NetworkService getters identify active unpaused HTTP requests. Runtime query excludes aborted Fetch, stale-generation/unsent/done XHR, data/cache/completed responses, and idle WebSockets. No realm entry or JS callbacks.

When genuine request I/O is pending the harness advances using monotonic actual elapsed time and a bounded1ms yield. Idle turns preserve deterministic16.667ms virtual progression; rendering opportunities stay60Hz. Existing real overall/process/harness deadline is unchanged. There are no fixture-dependent branches or timeout exceptions. Two native groups cover clock/render progression plus paused/cancelled/aborted/stale/data/WebSocket state boundaries on actual loopback transport.

Boat native/tmp:638 owned tests/six skips,3887 workspace/17 skips,four doctests,html5lib1894 and expanded1930/six unsupported retained. Whole13 WPT directories(1112pages),111module pages and whole192css/cssom pages retain every exact passing-name multiplicity and passing file: zero gains, zero losses. Known accepted XHR XMLcontentType assertion is retained on both matched sources. CSSOM1769 passing assertions and modules132 are unchanged. No parser/engine corpus gains are credited to this runner correction.

Compact fixture content pins/name hashes and rawSHA256 comparisons are committed; raw files MAIN .agents/html/html-io-clock-before/ and html-io-clock-after/. Native hook ownership was explicitly granted by root for lower Dispatcher/Service, scheduler forwarding, runtime-net query and harness clock.
