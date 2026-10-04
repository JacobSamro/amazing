# Symbol-aware binding Get

Recorded base: `dcee3e6202fa31b8543c2ef83a28cafe73f13dc1`.

`binding_get_property_key(&mut self, object: &JsValue, key: PropertyKey)` forwards to the existing semantic Get gateway. It preserves the original receiver, Symbol identity, exact UTF-16 string keys, inherited accessors, Proxy traps and arbitrary abrupt completions. The existing string binding API remains unchanged. The gateway also retains its existing nullish receiver behavior (Undefined); this adapter does not widen receiver validation or claim a complete primitive GetValue protocol. The driver can obtain MessagePort sequence iterators through the actual well-known iterator Symbol without scanning native properties.

Three grouped native controls exercise both genuine interpreted and native callable factories: keys and primitive receivers; observable Get order and receiver identity; arbitrary throws, revoked proxies and foreign getter homes. This is an additive adapter, without a new property algorithm or source port.

Fresh paired Linux qualification measured workspace 4,517 → 4,520 passes (17 skips), four doctests on each side, and 24 complete corpus controllers across whole Object/Proxy/Reflect/Symbol/String/language selections in both modes. Each mode retained 29,768 passing files and 55,813 passing variants across 30,166 selected files, with zero lost files or variants. Both whole commands exited 0. Actual pool profile was default 4 CPU/8 GB baremetal. Compact source/provenance/counts/hashes are in evidence/binding-property-key.summary.json; full raw output remains on Boat. Tested candidate was 915c751b83883e72c0478c4b51ef8b36e3f824aa; subsequent changes only add this qualification record.
