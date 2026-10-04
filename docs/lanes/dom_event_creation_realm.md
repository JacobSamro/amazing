# Event allocation and redispatch realms

Recorded engine base: 70a76d75dd90009bd6948c0bae548ded5ab6a7e1. Linux qualification completed against this fixed published base.

Primary DOM sections 2.5, 2.9 and 2.10 (reviewed 2026-10-05): https://dom.spec.whatwg.org/#constructing-events and https://dom.spec.whatwg.org/#concept-event-fire. WebIDL platform objects keep their relevant realm: https://webidl.spec.whatwg.org/#js-platform-objects.

Every authored constructor and Event subclass records the actual native callee allocation realm before exposure. Browser firing and Document.createEvent record the real target/Document relevant realm. One EventId map covers Event, CustomEvent, UI subclasses, navigation/error subclasses and MessageEvent. Event wrapper cache reentry uses the saved realm before class/prototype selection. Existing dispatch of an Event preserves its allocation identity and typed/value sidecars; dispatch does not mint a new wrapper realm.

Target resolution uses genuine Node creation anchors, retained WindowContext slot associations, messaging/generic EventTarget allocation records and the qualified AbortSignal allocation map. There is no caller/root/current-proxy fallback. Numeric native target slots are a trusted same-Dom internal boundary; public receiver conversion validates private arena/generation brands.

No scheduler, timeout eligibility, event path/activation/cancellation, structured-clone/router algorithm or Window navigation changes. Only immutable Window slot-to-realm lookup is adopted from HTML. Original MessageEvent and Abort controls remain. Weak JS wrapper custody remains unchanged; scalar RealmHandles add no strong wrapper roots.

Whole native groups cover every installed constructor class, borrowed Document.createEvent in both directions, foreign function/object listeners, custom JS subclasses, reentrant rejection, retained old Event after child navigation, trusted existing-object redispatch, native MouseEvent/MessageEvent payload firing at actual foreign targets, actual Window/Node/MessagePort/AbortSignal firing and private key/map conflict validation. Whole workspace/docs and original DOM/Abort/messaging/shadow/syntax/XML corpora qualify the single ready squash. Raw stays Boat; only compact named-count/file/hash proof is committed.

Qualification: workspace 4,599 → 4,609 passed, zero failures, 17 skips; doctests 4 → 4. Whole Abort 36, messaging 111, core 92,842, shadow 5,681, syntax 2,708 and XML 112 passing assertions retained exactly, with zero passing-file losses. Shadow uses matched actual META-long 600-second budgets. Compact proof: `crates/amazing-bindings/evidence/event-creation-realm/summary.json`; raw remains Boat. The original 133 candidate passed the whole pair; final source was gated independently against the completed baseline after correcting one authored numeric test constructor compile error (actual 101 retained). No partial-progress feature credit.
