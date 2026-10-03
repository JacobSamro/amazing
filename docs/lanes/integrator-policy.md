# Native Document policy snapshot seam

Source `b5bf4da787f948281fb0e62347da9939adc661e6`, fixed dependency
`deb2cefc`. This additive driver API stores `ClientContext` by native Document
index and generation. Reads return an owned snapshot after all state and DOM
borrows end; writes validate a native Document and replace earlier generations.
HTML owns creator inheritance and response-header policy construction.

Boat native `/tmp` gate: 4,013 workspace tests passed, 17 skipped; four doctests
passed. The grouped regression covers actual CSP snapshots, stale/wrong native
identity, creator snapshots surviving replacement, explicit blank inheritance,
and immediate mutable-borrow availability after reads. No callback, realm entry,
transport, facade or parser behavior changes are included.

This is a pure internal seam. HTML56 must adopt the exact source into both sides
of its pinned corpus comparison; that ticket supplies behavioral policy proof.
