# Coroutine class strictness companion

The class matrix on fixed dependency `35217ac3` found a separate actor-context
defect. Native VM class evaluation retained strictness after Await, but the
forced-AST class key resumed with the enclosing sloppy function's initial
strictness. Diagnostic source `32eb2370` produced
`[true,"number","number",["tdz","constructor","prototype"]]`: the unbound
assignment created `notDeclared`, and the expected ReferenceError was absent.
The original diagnostic remains in the lane's iteration-3 workspace log.

`leave_segment` now saves the body's current strictness before restoring the
driver's strictness. `enter_segment` reinstalls that saved value. The existing
class wrapper still restores the enclosing strictness after evaluation, and
segment completion take/restore is unchanged. This is an additive context
capture, independent of class lowering and the separate sync-generator realm
companion. No new generator state or TLS is introduced.

The grouped proof covers Await and Yield class keys in sloppy bodies, required
ReferenceError before creating an unbound property, subsequent successful
sloppy assignment, nested With/try/disposal, and native versus forced-AST
factories. Readiness requires the paired whole language/class/coroutine corpus
and workspace/doctest gate; this document records the diagnosis, not readiness.
