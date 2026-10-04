#!/usr/bin/env python3
"""Generate only normalization DATA from pinned official Unicode 17 inputs.

No host unicodedata tables or third-party packages are used. The Rust algorithm
is separately MIT-licensed; generated Unicode data uses Unicode-3.0.
"""
import hashlib
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[1]
DATA = ROOT / "data" / "unicode17"
SOURCE = ROOT / "src"
PROVENANCE = json.loads((DATA / "provenance.json").read_text())
assert PROVENANCE["unicode_version"] == "17.0.0"
assert PROVENANCE["license"] == "Unicode-3.0"
for name, metadata in PROVENANCE["inputs"].items():
    content = (DATA / name).read_bytes()
    assert len(content) == metadata["bytes"], name
    assert hashlib.sha256(content).hexdigest() == metadata["sha256"], name


def ranges(text):
    endpoints = [int(point, 16) for point in text.strip().split("..")]
    return range(endpoints[0], endpoints[-1] + 1)


canonical = {}
compatibility = {}
combining = {}
previous = -1
for line in (DATA / "UnicodeData.txt").read_text().splitlines():
    fields = line.split(";")
    assert len(fields) == 15
    point = int(fields[0], 16)
    assert point > previous
    previous = point
    ccc = int(fields[3])
    if ccc:
        # UCD First/Last ranges all have class zero in these pinned inputs.
        assert not fields[1].endswith((", First>", ", Last>"))
        combining[point] = ccc
    decomposition = fields[5].split()
    if decomposition:
        is_compatibility = decomposition[0].startswith("<")
        if is_compatibility:
            decomposition.pop(0)
        parts = tuple(int(part, 16) for part in decomposition)
        assert parts and all(part <= 0x10FFFF for part in parts)
        if is_compatibility:
            compatibility[point] = parts
        else:
            assert len(parts) <= 2
            canonical[point] = parts

excluded = set()
for line in (DATA / "DerivedNormalizationProps.txt").read_text().splitlines():
    body = line.split("#", 1)[0].strip()
    if not body:
        continue
    fields = [field.strip() for field in body.split(";")]
    if fields[1] == "Full_Composition_Exclusion":
        excluded.update(ranges(fields[0]))
explicit = set()
for line in (DATA / "CompositionExclusions.txt").read_text().splitlines():
    body = line.split("#", 1)[0].strip()
    if body:
        explicit.update(ranges(body))
assert explicit <= excluded

composition = {}
for point, parts in canonical.items():
    if len(parts) == 2 and point not in excluded:
        assert combining.get(parts[0], 0) == 0
        assert parts not in composition
        composition[parts] = point


def expand(point, active=()):
    assert point not in active, "cyclic decomposition"
    if 0xAC00 <= point < 0xD7A4:
        syllable = point - 0xAC00
        result = [0x1100 + syllable // 588, 0x1161 + (syllable % 588) // 28]
        if syllable % 28:
            result.append(0x11A7 + syllable % 28)
        return result
    parts = canonical.get(point, compatibility.get(point))
    if parts is None:
        return [point]
    return [value for part in parts for value in expand(part, (*active, point))]


HEADER = """// Generated normalization DATA from official Unicode 17.0.0 UCD.
// Copyright © 1991-2026 Unicode, Inc. Distributed under Unicode-3.0.
// See ../data/unicode17/LICENSE.txt and provenance.json for original inputs.
// Regenerate with: python3 crates/amazing-js/tools/generate_unicode17_normalization.py
// Rust normalization algorithm is separately adapted under MIT; this file is data.
"""


def table(name, rust_type, rows):
    # Two entries per line keep each generated Rust data file below 4000 lines.
    entries = ["    " + ", ".join(rows[index:index + 2]) + ","
               for index in range(0, len(rows), 2)]
    return f"const {name}: {rust_type} = &[\n" + "\n".join(entries) + "\n];\n"


outputs = {
    "canonical": table("CANON_DECOMP", "&[(u32, u32, u32)]", [
        f"(0x{point:X}, 0x{parts[0]:X}, 0x{parts[1] if len(parts) == 2 else 0:X})"
        for point, parts in sorted(canonical.items())])
        + table("COMPOSE", "&[(u32, u32, u32)]", [
            f"(0x{first:X}, 0x{second:X}, 0x{point:X})"
            for (first, second), point in sorted(composition.items())]),
    "compatibility": table("COMPAT_DECOMP", "&[(u32, &[u32])]", [
        f"(0x{point:X}, &[" + ", ".join(f"0x{part:X}" for part in expand(point)) + "])"
        for point in sorted(compatibility)]),
    "combining": table("CCC", "&[(u32, u8)]", [
        f"(0x{point:X}, {ccc})" for point, ccc in sorted(combining.items())]),
}
generated = {}
for suffix, text in outputs.items():
    name = f"runtime_string_normalization_data_{suffix}.rs"
    content = (HEADER + text).encode()
    assert content.count(b"\n") < 4000
    (SOURCE / name).write_bytes(content)
    generated[name] = {"sha256": hashlib.sha256(content).hexdigest(),
                       "bytes": len(content), "lines": content.count(b"\n")}

test_rows = 0
part = ""
part1 = set()
for line in (DATA / "NormalizationTest.txt").read_text().splitlines():
    body = line.split("#", 1)[0].strip()
    if body.startswith("@"):
        part = body
        continue
    if not body:
        continue
    columns = body.split(";")
    assert len(columns) == 6 and not columns[-1].strip()
    assert all(column.strip() for column in columns[:5])
    if part == "@Part1":
        points = columns[0].split()
        assert len(points) == 1
        part1.add(int(points[0], 16))
    test_rows += 1

(DATA / "generated.json").write_text(json.dumps({
    "unicode_version": "17.0.0", "canonical_decompositions": len(canonical),
    "compatibility_decompositions": len(compatibility), "combining_classes": len(combining),
    "composition_pairs": len(composition), "full_composition_exclusions": len(excluded),
    "explicit_composition_exclusions": len(explicit), "normalization_test_rows": test_rows,
    "part1_single_code_points": len(part1), "unlisted_all_code_points": 0x110000 - len(part1),
    "public_conformance_row_checks_per_backend": test_rows * 20,
    "generated": generated,
}, indent=2) + "\n")
print(json.dumps({"tables": generated, "normalization_test_rows": test_rows}, indent=2))
