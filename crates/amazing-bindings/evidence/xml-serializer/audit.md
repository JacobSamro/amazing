# XMLSerializer algorithm and pinned-oracle audit

The native serializer ports the complete node/namespace area from
[jsdom/w3c-xmlserializer](https://github.com/jsdom/w3c-xmlserializer/tree/265f1e1a192594708281809c8c7730b65acb36f3),
`lib/serialize.js`, `lib/attributes.js`, and `lib/constants.js`, at
`265f1e1a192594708281809c8c7730b65acb36f3` (MIT, Sebastian Mayr).
The source header, bundled full license, and appended third-party notice record
that provenance. No upstream dependencies were added.

`XMLSerializer.serializeToString` passes `require_well_formed = false`.
Standalone Attr serializes as the empty string; native UTF16 text/attribute units,
including lone surrogates, remain intact. The separate strict native helper checks
specified well-formedness constraints. Both paths validate tree topology before
following children, including malformed document sibling cycles.

The port follows the current [DOM Parsing serialization algorithm](https://w3c.github.io/DOM-Parsing/#xml-serializing-an-elements-attributes)
for attribute prefix selection and whitespace escaping. In particular, an
unmapped attribute namespace retains its original prefix if that prefix is absent
from the local prefix map; generated attribute declarations then update that map
before processing later attributes. This differs from the port source's older
unconditional prefix generation. Native coverage checks same-element collisions.

The preliminary pinned `c2a2fce69e5c2509d9dc28440ec8b42532e5c293` XMLSerializer
page measures 29 passes and four failures, with no fixture changes. Remaining cases:

- Two expect a generated prefix instead of the original prefix when no conflicting
  declaration exists on that element. The current draft's attribute algorithm
  retains the original prefix in these cases, including ancestor-only bindings.
- One expects ordinary null-namespace `xmlns` attributes to be omitted by local
  name. Namespace recording/omission currently examines XMLNS-namespace attributes;
  ordinary null-namespace attributes remain ordinary attributes.
- One expects a redundant empty default namespace declaration to survive alongside
  an empty prefixed declaration; the current algorithm drops that redundant default.

These classifications remain failures in the evidence. There is no XLink-specific
prefix rule and no expected-label substitution. The draft explicitly describes
namespace-prefix generation collisions in issue 44, including the pinned case that
expects duplicate generated declarations. The permissive API follows that algorithm
without imposing a general XML parser validity requirement.

Constructor coverage includes no-new rejection, subclasses, Reflect.construct with
custom prototypes, actual iframe-realm receivers/nodes, bound and Proxy new targets,
foreign intrinsic fallback, revoked proxies, and abrupt prototype getters. The
new-target prototype is read before native allocation; primitive fallback uses the
runtime's semantic function-realm lookup rather than the current script scope.
