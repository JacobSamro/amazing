# HTML61 pure actual Document realm lookup

FrameRuntime::document_realm(&self, &Dom, NodeIdx) returns only the actual live generation-matching native Document realm. It reuses document_context to validate the ancestor chain, then resolves the actual context. It performs no JS callbacks, realm entry or caller/current-document fallback. Native grouped root/child/stale/retired/inert/element/invalid-index assertions pass.

Isolated code38461181 plus native-width correction5cc520ee on fixed coherent92191ddb (READY57/58/59). Boat673 owned/6skip4110 workspace/17skip and four docs, all1894CLI/1930expanded trees (six unsupported). Whole13 contexts,111modules,220HTMLsyntax,192CSSOM and123Selection pages preserve every passing named assertion multiplicity and passing file: zero loss and zero gains; fixture pins match. Lower handler compiler/session adoption is the facade owner's separate gate. No MIME/HTML60 source included.
