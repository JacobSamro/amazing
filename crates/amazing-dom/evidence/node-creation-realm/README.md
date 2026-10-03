# Node creation and relevant realm

Fixed dependency base: `deb2cefcdb110adced3c1fb32b89a38f51844ba6`.
Before: `4bb491cf57d5d66ca76b7b7dd02e2644885a90dc`; after: `30c995a7cda3e5e4929436470321c6b4dbbd46d7`.

Generation-tagged immutable creation-document anchors determine genuine Node and
DOMImplementation wrapper realms, independently of mutable ownerDocument.
Adoption preserves wrappers; clone uses the current node document; import uses
the target document. Lazy/attached Attr boundaries preserve identity and original
realms. Factory/Document clone origin snapshots and receiver-saved DOMParser
contexts use actual typed Origins. Known template-owner creation mints one stable
opaque default Origin and preserves Document interface identity. Retained realms
do not invent defaultView or active browsing contexts. Missing/stale allocation
contexts fail before native allocation. Existing permanent RealmRegistry ownership
is unchanged; this does not implement realm garbage collection.

Linux workspace: 4,016 → 4,027 passed, 17 skipped; four doctests both sides.
Whole 579 pinned pages: 57,039 → 57,208 passing assertions, 2,278 → 2,111 failures,
seven errors unchanged. Exact passing-name Counters (including multiplicities),
accepted file-pass classifications and all selected files have zero losses.
All 13 node-creation-realm assertions pass. Credit 168 assertions; the additional
XHR completion is uncredited timing. Unsupported top-level XHTML remains honest.
Syntax72 is byte-identical: 2,702 passes / six failures.

The initial stronger `afa9d348` diagnostic accepted 57,037 passes. Both corrected
refs retain every accepted name/multiplicity/file pass. Earlier accepted Range
(full579 and full66 matched120s), XMLSerializer and importer reports also have
zero lost passing names or accepted file passes. Initial failed compiler runs and
the 23-MutationObserver prerequisite regression remain in orchestration scratch.
The identical HTML context-only correction restored all 23. Its owner separately
gated full13 directories, whole220 syntax, 111 modules and tree corpora with zero
losses (3,876 workspace tests, 17 skipped, four docs).

Both sides include identical HTML origin APIs, child registration/Option hooks,
parser-created frame timing37, its test-only numeric correction, and d96 context-only
forwarding. These receive no Node-ticket credit. Before compatibility registration
records actual native generation/origin without the Node realm maps.
The wrapper builds release before workspace/docs/corpus, uses native /tmp and
starts WPT with ensure-wpt; shared corpora are read-only.

Compact rows hash exact passing names with UTF-16LE/surrogatepass. Full raw
reports, logs and fixture bytes are fetched to the lane's orchestration scratch;
provenance records immutable source/binary/report hashes.

The remaining 17 PI-attribute cases use invalid XML documents containing only a
PI; current DOMParser error steps require an empty error Document before
parsererror insertion. No XML well-formedness relaxation is claimed.
