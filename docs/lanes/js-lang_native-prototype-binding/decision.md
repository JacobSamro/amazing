# Native SetPrototypeOf embedding adapter

Recorded published base ba82c97259f84c89dd1ce8a6906a3ee0a9ee38bd. Independent of the unqualified compiled module-source family. HTML owns the CustomElement consumer.

`Interpreter::binding_set_prototype_of(&JsValue,&JsValue)->Result<bool,Throw>` validates an object target and object-or-null prototype, then delegates the existing `default_set_prototype` unchanged. No mutable Reflect/Object lookup, direct prototype assignment, creation-realm replacement, false-to-throw conversion, or Proxy exception wrapping is introduced. Existing ordinary/Proxy/WindowProxy/module-namespace/immutable/nonextensible/cycle algorithms stay authoritative.

Five authored native groups cover poisoned Reflection, null/same/foreign prototypes, rejected cycles/extensibility/immutable targets, actual Proxy operands/false/throw/revocation/invariant behavior, and primitive validation without coercion. They are new API controls; no fake baseline adapter is provided. Whole existing Object/Reflect/Proxy prototype corpus and crate/doctests run on Linux with recorded source, fixed published base, native capacity and compact hashes. No local build or new dependency; this is a delegation adapter, not a new external port.
