# Complete source, FDI and EDI composition

The producer/consumer composition is qualified in [the joint gate](js-source-fdi-edi-joint.md); [compact evidence](js-source47-fdi-edi-evidence.json) records the actual native/corpus results and raw HOME custody.

The original input `eval('var x=3');Reflect.deleteProperty(globalThis,'x')`, followed by `var absent;let x=4`, now correctly succeeds: the configurable eval property was deleted and current-primary CreateGlobalVarBinding does not retain the historical Script-name veto. The first candidate's unchanged `.is_err()` oracle was its sole native failure (4,641 passed/1 failed/17 skipped; raw SHA prefix8734209c). That actual failure remains preserved. The explicitly approved test correction retains the input and checks both absent bindings, lexical x=4 and no own x.

The grouped eight-case controls additionally cover deleted/undeleted eval variables/functions, authored configurable and inherited non-configurable properties, hardened eval properties and genuine non-configurable Script var/function properties. All original EDI/FDI/source/resource controls remain. Wider historical Script name tracking is a separate GDI boundary, not a claim of this ticket.
