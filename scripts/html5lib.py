#!/usr/bin/env python3
"""Measure Amazing against named upstream html5lib tree-construction files.

The html5lib tree tests now live in WPT under
html/syntax/parsing/resources/*.dat. This runner exercises the public `parse`
artifact. Fragment/script-mode cases are reported as skipped because that CLI
accepts only complete documents; every other discovered case is executed.
"""

from __future__ import annotations

import argparse
import subprocess
from dataclasses import dataclass
from pathlib import Path


@dataclass
class Case:
    source_file: str
    index: int
    data: str
    expected: list[tuple[int, str]]
    skip_reason: str | None


def canonical_expected(document: str) -> list[tuple[int, str]]:
    result: list[tuple[int, str]] = []
    for line in document.splitlines():
        if not line.startswith("| "):
            if result and result[-1][1].startswith("<!--") and not result[-1][1].endswith("-->"):
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
        if "=" in content and not content.startswith(("<", '"')):
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
    return result


def canonical_actual(output: str) -> list[tuple[int, str]]:
    lines = output.splitlines()
    if not lines or lines[0] != "#document":
        return [(-1, output.strip())]
    result: list[tuple[int, str]] = []
    for line in lines[1:]:
        if result and result[-1][1].startswith("<!--") and not result[-1][1].endswith("-->"):
            depth, content = result[-1]
            result[-1] = (depth, content + "\n" + line)
            continue
        spaces = len(line) - len(line.lstrip(" "))
        result.append((max(0, spaces // 2 - 1), line.lstrip(" ")))
    return result


def load_cases(path: Path) -> list[Case]:
    text = path.read_text(encoding="utf-8-sig")
    cases: list[Case] = []
    for index, block in enumerate(text.split("#data\n")[1:], start=1):
        if "#errors\n" not in block or "#document\n" not in block:
            continue
        data, remainder = block.split("#errors\n", 1)
        data = data.removesuffix("\n")
        skip_reason = None
        before_document, document = remainder.split("#document\n", 1)
        if "#document-fragment" in before_document:
            skip_reason = "fragment context is not exposed by the parse CLI"
        elif "#script-on" in before_document or "#script-off" in before_document:
            skip_reason = "script-mode variant is not exposed by the parse CLI"
        cases.append(
            Case(
                source_file=path.name,
                index=index,
                data=data,
                expected=canonical_expected(document),
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
        process = subprocess.run(
            [str(args.binary), "parse"],
            input=case.data,
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
