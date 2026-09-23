# IDNA data and WPT fixtures

Unicode version: **17.0.0**, matching the pinned WPT IDNA corpus. The generated `src/url/idna_data.rs` uses these
Unicode Consortium files (SHA-256 of the downloaded bytes):

| File | SHA-256 |
| --- | --- |
| [IdnaMappingTable.txt](https://www.unicode.org/Public/17.0.0/idna/IdnaMappingTable.txt) | `87f05505dc026fdb2bff16132bdc68a8014675836882a9a2b1844540ad3be382` |
| [DerivedBidiClass.txt](https://www.unicode.org/Public/17.0.0/ucd/extracted/DerivedBidiClass.txt) | `4867b4b7f0731ed1bfcd34cc6251211ff1542541fce0734b6fbda139ee80b3a4` |
| [DerivedJoiningType.txt](https://www.unicode.org/Public/17.0.0/ucd/extracted/DerivedJoiningType.txt) | `f39ebe974825d6736aee15582250307aa532b2cfab3caf3f86bd23fddc9c5c4d` |
| [UnicodeData.txt](https://www.unicode.org/Public/17.0.0/ucd/UnicodeData.txt) | `2e1efc1dcb59c575eedf5ccae60f95229f706ee6d031835247d843c11d96470c` |
| [DerivedCombiningClass.txt](https://www.unicode.org/Public/17.0.0/ucd/extracted/DerivedCombiningClass.txt) | `191463abfbd202703c6fd6776a92a23ac44ec65e0476a7f95aa91ca492cef29b` |
| [DerivedNormalizationProps.txt](https://www.unicode.org/Public/17.0.0/ucd/DerivedNormalizationProps.txt) | `71fd6a206a2c0cdd41feb6b7f656aa31091db45e9cedc926985d718397f9e488` |

To regenerate, download those six files to one directory and run
`python3 generate_unicode.py <directory>`. The generator checks all hashes and
writes a complete, binary searched IDNA mapping plus normalization, bidi,
joining, and mark tables. The generated Rust source is 400,566 bytes and 2,702
lines, including 9,262 mapping spans and 7,950 mapped code points. All lookup
arrays are sorted and binary searched; no full-codepoint allocation is stored.

WPT revision: `c48d58747e1f211527fb695fd60548a997fae617`.

| Selection | Source | SHA-256 | Objects |
| --- | --- | --- | ---: |
| toascii | [toascii.json](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/url/resources/toascii.json) | `644eba9d5b593df8095cfa307222f3014542ff9cc02d555f8e5660059d80470f` | 87 |
| IdnaTestV2 | [IdnaTestV2.json](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/url/resources/IdnaTestV2.json) | `338192b9815dbdace6c035cb1acd50cd737070cd67d6e3f620d2543f63eb0cbb` | 2,671 |

Download both WPT files to a directory and run `python3 generate_wpt.py
<directory>`. It checks their hashes, keeps every object in order, and maps
unpaired UTF-16 surrogates to U+FFFD as the URL Standard's scalar value string
conversion requires. Raw `IdnaTestV2.json` is not checked in because it exceeds
the repository's 4,000 line file limit. The native runner is `examples/idna_wpt.rs`.
