# Pinned Public Suffix List

Upstream: https://github.com/publicsuffix/list
Commit: `645b36c9a51ff0b3718df0ba02f6570aa6921632`
Original `public_suffix_list.dat` SHA-256:
`d3fba7b2934b0e1b9c958baf42bef85107b53cd0115ff7e81a7e61c7119a36d0`.

`upstream-00.dat` through `upstream-05.dat` concatenate byte-for-byte into that
upstream file. They are split only to respect the repository's 4,000-line file
limit; comments, section markers and attribution are preserved. `LICENSE` is
upstream MPL-2.0, retained unmodified. `test_psl.txt` is upstream's CC0 fixture
at the same commit. `pin.json` records exact input hashes and part order.

Both ICANN and PRIVATE sections are mandatory. Generated `rules-*.txt` are
sorted ASCII rules derived from these inputs (MPL-2.0). `fixtures.tsv` derives
from active upstream fixture statements (CC0); mixed case and Unicode are
normalized before the provider boundary, and null/empty uses the `-` sentinel.
These fixtures test suffix lookup on canonical inputs, not runtime IDNA.

`../../tools/normalize_psl.py` performs offline data preparation with Python 3
standard-library RFC3492 punycode; it deliberately does not use IDNA2003, UTS46
mapping tables, or fetch anything. Unicode spelling is the trusted upstream
input; runtime host validation remains a separate default-deny seam. Generator,
output and fixture hashes are recorded in `generated.json`. Generate with
`python crates/amazing-fetch/tools/normalize_psl.py`; verify using `--check`
on win-001. Code builds/tests must also run there, never on the editing Mac.

Updates require a reviewed PR: explicitly choose a new upstream commit, replace
all input shards/licence/fixtures and input hashes, regenerate offline, review
rule additions/removals and all changed boundaries, then rerun provider/cookie
and URL gates twice. Add cases for each changed site boundary. Review persisted
cookie and cache keys before deployment, including newly public domains,
PRIVATE tenant splits/merges, SameSite changes and invalidation/migration of
old keys. The library never fetches data or rewrites user profiles. See
`docs/lanes/net_TODO.md` for the embedding migration responsibility.
