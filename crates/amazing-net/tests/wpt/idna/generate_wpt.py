#!/usr/bin/env python3
"""Generate native fixtures from the two WPT JSON files in PROVENANCE.md."""
import hashlib
import json
import pathlib
import sys

HASHES = {
    "toascii": "644eba9d5b593df8095cfa307222f3014542ff9cc02d555f8e5660059d80470f",
    "IdnaTestV2": "338192b9815dbdace6c035cb1acd50cd737070cd67d6e3f620d2543f63eb0cbb",
}


def rust_string(value):
    if value is None:
        return "None"
    chars = []
    for char in value:
        cp = ord(char)
        if 0xD800 <= cp <= 0xDFFF:
            chars.append("\ufffd")  # URL scalar value string conversion
        elif char == "\\":
            chars.append("\\\\")
        elif char == '"':
            chars.append('\\"')
        elif char == "\n":
            chars.append("\\n")
        elif char == "\r":
            chars.append("\\r")
        elif char == "\t":
            chars.append("\\t")
        elif cp < 32 or cp == 127 or 0x202a <= cp <= 0x202e or 0x2066 <= cp <= 0x2069:
            chars.append(f"\\u{{{cp:x}}}")
        else:
            chars.append(char)
    return 'Some("' + "".join(chars) + '")'


def main(root):
    for name, expected in HASHES.items():
        raw = (root / (name + ".json")).read_bytes()
        assert hashlib.sha256(raw).hexdigest() == expected, name
        cases = [item for item in json.loads(raw) if isinstance(item, dict)]
        with (pathlib.Path(__file__).parent / (name + "_cases.rs")).open("w") as out:
            out.write("[\n")
            for index, case in enumerate(cases):
                out.write(f'  Case {{ index: {index}, input: {rust_string(case["input"])}, output: {rust_string(case["output"])}, url_only: {str(case.get("urlStandardOnly", False)).lower()} }},\n')
            out.write("]\n")
        print(name, len(cases))


if __name__ == "__main__":
    main(pathlib.Path(sys.argv[1]))
