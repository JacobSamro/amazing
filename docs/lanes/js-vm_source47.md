# VM genuine source propagation

Qualified in the complete [joint gate](js-source-fdi-edi-joint.md). CompiledFunction carries SourceMetadata from genuine function, arrow and method AST carriers; class constructors override their method carrier with the whole class source. instantiate transfers that carrier to JsFunction. Synthetic body-only async/generator/native bridges remain absent and preserve the original callable's source.

All three original private VM groups remain, proving actual compiled carriers, eligible function controls without relevant TreeEval, exact source units/comments/line endings, buffer identity after input/AST drop, whole-class constructor source and synthetic absence. Public forced-AST/native consumer controls are retained. No new AST Send/Sync or cycle-reclamation claim is made.
