# Native user-agent event payload dispatch

Code: 79c1fa53 and 0f2307d6 on fixed c85dccf489fa3a92f56c906b91b16dccbfa39192 (root eb7fa04a plus standalone function-realm API).

`dispatch_user_agent_event_with_payload` accepts a populated native Event, dispatches it once through the existing trusted event/activation pipeline, and returns whether default action was allowed. The native group verifies MouseEvent payload and trusted state, checkbox preactivation, cancellation rollback, and exactly one input/change pair for successful activation. It does not invoke the legacy dispatcher.

Linux Boat final: 3,868 workspace tests passed, 17 skipped; four doctests passed. Fixed baseline: 3,867 tests and four doctests. Whole pinned dom/nodes + dom/events: 482 pages, 13,095 passing assertions before and after, zero lost passing assertions (including duplicate names) and zero lost passing files. Raw failures were 551 -> 553, errors six -> six. Three zero-pass cases changed failure/deadline classifications; comparison.json records them without crediting gains. Facade adoption is separate work.

Both release binaries were built from explicit frozen refs before the corpus run. Shared pinned WPT was read-only and fixtures were unchanged. Compact files hash exact UTF-16LE names and retain multiplicities; provenance.json records binary hashes, refs, pin and runner. Full logs/reports are retained in MAIN .agents/dom-bindings/ua-event-before and ua-event-after.
