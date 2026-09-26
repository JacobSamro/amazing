# IPC-11 DOM-side MessageEvent contract

Implementation `2f5ca997` plus TypeError follow-up `aa2024f3`, fixed base
`536d0bdc`. Native `EventClass::Message` and `Interface::MessageEvent` preserve
payload identity using host-owned roots, validate dictionaries and nullable
sources, convert origin to USVString, cache a frozen ports array, implement
legacy initialization, and release replaced/discarded initialization roots.
The payload follows [HTML MessageEvent](https://html.spec.whatwg.org/multipage/comms.html#the-messageevent-interface).

Shared targets use optional `BindingHost::external_event_target_slot/value`.
The host must allocate from `Dom::create_event_target` and validate canonical
wrapper identity, realm, and liveness. `is_message_port`,
`is_message_event_source`, and `sequence_message_ports` supply IPC-owned brand
and iterable conversion. Absent iterable support reports NotSupportedError;
non-object sequences are rejected with TypeError first. The driver implements
`create_frozen_array` without calling script-replaceable Object.freeze.

`dispatch_message_event(&mut host, EventTarget, kind, MessageEventPayload<V>)`
delivers a trusted event through the shared registry. Payload fields are
`data: V`, `origin/last_event_id: Vec<u16>`, `source: Option<V>`, `ports: Vec<V>`.
Native tests cover defaults, conversion order, rooting, legacy reinitialization,
constructor/receiver errors, Window identity, frozen arrays, target/currentTarget,
capture, once, passive, removal, and trusted versus script dispatch.

## Gate

Linux Boat, exact `aa2024f3`: 2,531 workspace nextest passed, 10 skipped;
workspace doctests 2 passed. Owned/driver iteration: 514 passed, 4 skipped,
owned doctests 0. `gate.json` records the fixed source and base.

Pinned WPT c2a2fce69e5c2509d9dc28440ec8b42532e5c293, fresh storage per file,
800×600, 20-second deadline. Seven selected sources before/after:
2 pass / 3 fail / 2 unsupported; 234 passing / 206 failing / 6 error subtests,
unchanged, with zero lost passing files or subtests. Raw results include executable
hashes and exact test names. The two unsupported sources are .htm/.window.js,
which this fixed runner does not execute. These are measured classifications.

**IPC adapter adoption remains separate.** runtime_host_messaging.rs still
installs its temporary global MessageEvent and owns duplicate messaging listener
lists. IPC must adopt the new constructor/prototype, target hooks and trusted
dispatch helper; no installed messaging conformance gain is claimed here. The
real native constructor/frozen-array adapter is independently exercised by the
driver regression. The null ports regression discovered in the first corpus run
was corrected and the full final gate repeated before recording these results.
