#!/usr/bin/env python3
"""Measure Amazing against named upstream html5lib tree-construction files.

The html5lib tree tests now live in WPT under
html/syntax/parsing/resources/*.dat. This runner exercises the public `parse`
artifact. Fragment/script-mode cases are reported as skipped because that CLI
accepts only complete documents; every other discovered case is executed.
"""

from __future__ import annotations

import argparse
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path

_ATTR_RE = re.compile(r' ((?:(?:xml|xlink|xmlns) )?[^\s="]+)="((?:[^"])*)"')
_ATTRIBUTE_LINE_RE = re.compile(r'(?:(?:xml|xlink|xmlns) )?[^\s="]+=".*"')


def _normalize_tag_attrs(content: str) -> str:
    """html5lib's own reference dump serializes an element's attributes in
    alphabetical order by name, not source/insertion order -- Amazing's CLI
    dump (`amazing-dom`'s `dump_rec`) intentionally keeps insertion order
    (existing tree.rs unit tests pin that), so a tag with 2+ attributes
    whose source order isn't already alphabetical looks like a mismatch even
    when the parsed tree is identical (html5lib tests19.dat cases 37/38:
    `<html c=d>`+`<html a=b>` attribute merge expected `a="b" c="d"`, got the
    insertion order `c="d" a="b"`; case 86: `<meta http-equiv=... content=
    ...>` expected `content` before `http-equiv`). Reordering only for this
    comparison -- not the CLI's actual dump format -- keeps that unrelated
    coverage intact while no longer conflating a cosmetic order difference
    with a real tree-construction mismatch."""
    if not (content.startswith("<") and content.endswith(">")):
        return content
    if content.startswith(("<!", "<?")):
        return content
    attrs = _ATTR_RE.findall(content)
    if not attrs:
        return content
    head = content[: _ATTR_RE.search(content).start()]
    ordered = "".join(f' {name}="{value}"' for name, value in sorted(attrs))
    return f"{head}{ordered}>"


@dataclass
class Case:
    source_file: str
    index: int
    data: str
    expected: list[tuple[int, str]]
    context: str | None
    skip_reason: str | None


def _is_unterminated_continuation(content: str) -> bool:
    """True when `content` is a dumped comment or text node whose own value
    contains a literal newline, so the `.dat`/dump format spills its
    remainder onto one or more raw, unindented follow-up lines instead of a
    new `| `-prefixed entry. Both comparators below need this: a comment
    (`<!-- ... -->`) missing its closing `-->`, or a quoted text node
    (`"..."`) missing its closing `"`. Previously only the comment case was
    handled, so any text node with an embedded newline (e.g. CDATA content
    `foo\nbar`, a `<script>` body, or plain text split by a bare `\n` in the
    source) silently truncated in `canonical_expected` (dropping everything
    after the first line) and, worse, fed its bare, unindented remainder(s)
    through `canonical_actual`'s normal per-line path, producing bogus
    zero-depth entries (html5lib domjs-unsafe.dat cases 1-3:
    `<svg><![CDATA[foo\\nbar]]>`; webkit01.dat cases 5/7/27/28/35 and others:
    any text node that is, or contains, a bare newline).

    A text node's dumped value is exactly one `"`, its data verbatim, then a
    closing `"` -- so a *one-character* `content` of just `"` can only mean
    the opening quote was immediately followed by the data's own newline (a
    real empty text node dumps as `""`, two characters, on one line, never
    split): that single-character case is always still open, which a plain
    `endswith('"')` check alone would miss."""
    return (
        (content.startswith("<!--") and not content.endswith("-->"))
        or (content.startswith('"') and not (len(content) >= 2 and content.endswith('"')))
    )


def canonical_expected(document: str) -> list[tuple[int, str]]:
    result: list[tuple[int, str]] = []
    for line in document.splitlines():
        if not line.startswith("| "):
            if result and _is_unterminated_continuation(result[-1][1]):
                depth, content = result[-1]
                result[-1] = (depth, content + "\n" + line)
            continue
        body = line[2:]
        spaces = len(body) - len(body.lstrip(" "))
        content = body.lstrip(" ")
        # Amazing stores the doctype separately from the arena tree and the
        # Phase 1 CLI intentionally dumps only the tree rooted at #document.
        if content.startswith("<!DOCTYPE"):
            continue
        if _ATTRIBUTE_LINE_RE.fullmatch(content):
            # html5lib serializes attributes as child-indented lines while
            # Amazing's CLI renders them inline in the element start tag.
            for position in range(len(result) - 1, -1, -1):
                parent_depth, parent = result[position]
                if parent_depth == spaces // 2 - 1 and parent.startswith("<"):
                    result[position] = (
                        parent_depth,
                        parent[:-1] + " " + content + ">",
                    )
                    break
            continue
        result.append((spaces // 2, content))
    return [(depth, _normalize_tag_attrs(content)) for depth, content in result]


def canonical_actual(output: str) -> list[tuple[int, str]]:
    lines = output.splitlines()
    if not lines or lines[0] != "#document":
        return [(-1, output.strip())]
    result: list[tuple[int, str]] = []
    for line in lines[1:]:
        # HTML-33: `canonical_expected` (above) has always skipped a
        # `<!DOCTYPE` line because Amazing's dump CLI didn't emit one at the
        # time this comparison was written ("Amazing stores the doctype
        # separately from the arena tree"). Some later ticket's dump-fidelity
        # work started emitting a `<!DOCTYPE html>` line as the tree's first
        # child, but this side of the comparison was never updated to match
        # -- every one of the corpus's 69 "failures" was this same single
        # spurious leading line, not a real tree-construction mismatch
        # (confirmed: every other line in every failing case's actual output
        # matched its expected output exactly).
        stripped = line.lstrip(" ")
        if stripped.startswith("<!DOCTYPE"):
            continue
        if result and _is_unterminated_continuation(result[-1][1]):
            depth, content = result[-1]
            result[-1] = (depth, content + "\n" + line)
            continue
        spaces = len(line) - len(line.lstrip(" "))
        result.append((max(0, spaces // 2 - 1), line.lstrip(" ")))
    return [(depth, _normalize_tag_attrs(content)) for depth, content in result]


def load_cases(path: Path) -> list[Case]:
    text = path.read_text(encoding="utf-8-sig")
    cases: list[Case] = []
    for index, block in enumerate(text.split("#data\n")[1:], start=1):
        if "#errors\n" not in block or "#document\n" not in block:
            continue
        data, remainder = block.split("#errors\n", 1)
        data = data.removesuffix("\n")
        skip_reason = None
        context = None
        before_document, document = remainder.split("#document\n", 1)
        if "#document-fragment" in before_document:
            fragment = before_document.split("#document-fragment\n", 1)[1]
            context = fragment.splitlines()[0].strip()
        if "#script-on" in before_document or "#script-off" in before_document:
            skip_reason = "script-mode variant is not exposed by the parse CLI"
        cases.append(
            Case(
                source_file=path.name,
                index=index,
                data=data,
                expected=canonical_expected(document),
                context=context,
                skip_reason=skip_reason,
            )
        )
    return cases


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument(
        "--files",
        nargs="+",
        default=[
            "blocks.dat",
            "comments01.dat",
            "tables01.dat",
            "adoption01.dat",
            "tests1.dat",
            "svg.dat",
            "math.dat",
            "namespace-sensitivity.dat",
            "tests11.dat",
        ],
        help="Named .dat files under html/syntax/parsing/resources",
    )
    parser.add_argument("--show-failures", type=int, default=5)
    args = parser.parse_args()

    resources = args.corpus / "html" / "syntax" / "parsing" / "resources"
    cases: list[Case] = []
    for name in args.files:
        path = resources / name
        if not path.is_file():
            raise SystemExit(f"missing html5lib/WPT file: {path}")
        cases.extend(load_cases(path))

    passed = 0
    failed: list[tuple[Case, list[tuple[int, str]], str]] = []
    skipped = 0
    for case in cases:
        if case.skip_reason:
            skipped += 1
            continue
        command = [str(args.binary), "parse"]
        # html5lib/WPT .dat fixtures are bare snippets with no <meta charset>
        # or transport encoding -- `amazing parse` takes raw bytes and runs
        # the real encoding-sniffing algorithm on them (§13.2.3.2), whose
        # spec-correct default when nothing else signals an encoding is
        # windows-1252, not UTF-8 (`amazing-html/src/encoding.rs`). Every
        # non-ASCII fixture was silently mojibake'd by that default. A
        # leading U+FEFF BOM is a real, spec-defined UTF-8 signal that any
        # conforming sniffer consumes and strips (§13.2.3.2's BOM step,
        # highest priority) -- exactly what a saved-from-the-real-web page
        # would carry to get the same unambiguous result, so this forces
        # correct decoding without changing the parsed content. Only for
        # `parse`, which sniffs; `parse-fragment` never sniffs (`amazing-
        # html::parse_fragment*` decodes its `&str` argument directly), so a
        # BOM there would wrongly become a literal U+FEFF character in the
        # fragment.
        data = case.data
        if case.context:
            command = [str(args.binary), "parse-fragment", case.context]
        else:
            data = "﻿" + data
        process = subprocess.run(
            command,
            input=data,
            text=True,
            encoding="utf-8",
            capture_output=True,
            check=False,
        )
        actual = canonical_actual(process.stdout)
        if process.returncode == 0 and actual == case.expected:
            passed += 1
        else:
            failed.append((case, actual, process.stderr.strip()))

    eligible = passed + len(failed)
    print(f"html5lib/WPT tree subset: {len(cases)} discovered")
    print(f"  files:   {', '.join(args.files)}")
    print(f"  passed:  {passed}")
    print(f"  failed:  {len(failed)}")
    print(f"  skipped: {skipped}")
    rate = (passed / eligible * 100.0) if eligible else 0.0
    print(f"  measured pass rate: {passed}/{eligible} ({rate:.2f}%)")

    for case, actual, stderr in failed[: args.show_failures]:
        print(f"\nFAIL {case.source_file} case {case.index}")
        print(f"  data:     {case.data!r}")
        print(f"  expected: {case.expected!r}")
        print(f"  actual:   {actual!r}")
        if stderr:
            print(f"  stderr:   {stderr}")
    return 0 if not failed else 1


if __name__ == "__main__":
    raise SystemExit(main())
