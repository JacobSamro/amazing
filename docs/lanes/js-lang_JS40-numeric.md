# JS40: NumericLiteral grammar, value and property keys

Fixed dependency: JS39 proof bdb84488e76d611f02dd4ed635ae30fad2895e56, incorporating coherent root35217ac3. Tests-only baseline4622fd244bb29f6027e3661e3ba614be49d86650. No moving integration dependency during measurement.

Existing TokenStream scanner snapshots preserve the original spelling independently of normalized Numeric digits/radix. Crate-private prefix and span queries detect actual legacy leading zeros, including trivia/comments and speculative RegExp scanning. No public Token or Numeric schema, AST field, VM opcode or class adapter changes are required.

Legacy octal ends at its final octal digit; the following dot belongs to member access rather than a fraction. Decimal leading-zero literals containing8/9 retain fractional/exponent grammar. Integer separators, BigInt suffixes, immediate escaped identifiers and strict/module/class/property-name boundaries are checked. Late directive checks consult actual scanner snapshots, preserving explicit0o leading-zero digits as valid strict syntax.

Radix Number values reuse the existing accurate BinaryBigInt conversion instead of a u128 bound. Numeric property names use ToPropertyKey, preserving arbitrary-precision BigInt strings and Number key formatting in interpreter/class/destructuring paths. Existing VM Numeric/ToKey delegates the same value conversion and needs no change.

Port: lucid-softworks/lumen crates/lumen/src/lexer.rs @f6c9860d44fa52249cfd647122edea184d32c863, MIT, Copyright(c)2026 Lucid Softworks limited. Verified cached source/license. The adapter preserves raw digit payload and corrects legacy-octal dot/exponent boundaries and long-radix limits against primary grammar; upstream fixed-width and normalized-token assumptions are not copied. Source header and appended notice included.

Primary reference: [ECMA-262 NumericLiteral grammar and NumericValue](https://tc39.es/ecma262/2025/multipage/ecmascript-language-lexical-grammar.html#sec-literals-numeric-literals), especially12.9.3.1 strict early errors and12.9.3.3 value conversion. Original Boat Node reference probes are in tests/numeric_literal_reference.cjs.

Validation pending: eight grouped native/forced-AST and parser/scanner groups, whole language/Number/BigInt/Object/Function/Proxy in both modes, workspace nextest and doctests, release profile/60-second deadline. No readiness claim before complete matched zero-loss proof.
