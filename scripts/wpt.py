#!/usr/bin/env python3
"""Run a declared subset of actual WPT CSS reftests through Amazing."""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
from pathlib import Path


LINK_RE = re.compile(r"<link\b[^>]*>", re.IGNORECASE)
ATTR_RE = re.compile(r"([:\w-]+)\s*=\s*(['\"])(.*?)\2", re.DOTALL)
DEFAULT_TESTS = [
    "css/CSS2/box-display/display-001.xht",
    "css/CSS2/box-display/box-generation-001.xht",
    "css/CSS2/box-display/anonymous-box-generation-001.xht",
    "css/CSS2/box-display/root-box-002.xht",
    "css/css-flexbox/css-flexbox-row.html",
    "css/css-flexbox/flexbox_justifycontent-center.html",
    "css/css-flexbox/flexbox_margin-auto.html",
    "css/css-flexbox/gap-001-ltr.html",
    "css/css-grid/grid-flexible-track-fractional-flex-factor.html",
    "css/css-grid/grid-flexible-track-free-space-distribution.html",
]


def match_reference(test: Path) -> Path:
    source = test.read_text(encoding="utf-8-sig", errors="replace")
    for tag in LINK_RE.findall(source):
        attrs = {name.lower(): value for name, _, value in ATTR_RE.findall(tag)}
        if attrs.get("rel", "").lower() == "match" and "href" in attrs:
            return (test.parent / attrs["href"]).resolve()
    raise ValueError("no rel=match reference")


def render(binary: Path, source: Path, output: Path, width: int) -> tuple[bool, str]:
    process = subprocess.run(
        [str(binary), "render", str(source), str(output), str(width)],
        text=True,
        encoding="utf-8",
        capture_output=True,
        check=False,
    )
    return process.returncode == 0, (process.stderr or process.stdout).strip()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("wpt", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--width", type=int, default=800)
    parser.add_argument("--tests", nargs="+", default=DEFAULT_TESTS)
    parser.add_argument("--work-dir", type=Path, default=Path("target/wpt-audit"))
    args = parser.parse_args()

    target = (Path.cwd() / "target").resolve()
    work = args.work_dir.resolve()
    try:
        relative_work = work.relative_to(target)
    except ValueError:
        parser.error(f"--work-dir must be inside {target}")
    if not relative_work.parts:
        parser.error("--work-dir must not be the target directory itself")
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)

    passed = 0
    failures: list[str] = []
    for index, relative in enumerate(args.tests, start=1):
        test = (args.wpt / relative).resolve()
        try:
            reference = match_reference(test)
        except (OSError, ValueError) as error:
            failures.append(f"{relative}: {error}")
            continue
        if not reference.is_file():
            failures.append(f"{relative}: missing reference {reference}")
            continue
        test_bmp = work / f"{index:03}-test.bmp"
        reference_bmp = work / f"{index:03}-ref.bmp"
        test_ok, test_log = render(args.binary, test, test_bmp, args.width)
        ref_ok, ref_log = render(args.binary, reference, reference_bmp, args.width)
        if not test_ok or not ref_ok:
            failures.append(
                f"{relative}: render failed; test={test_log!r}; reference={ref_log!r}"
            )
            continue
        test_bytes = test_bmp.read_bytes()
        reference_bytes = reference_bmp.read_bytes()
        if test_bytes == reference_bytes:
            passed += 1
        else:
            failures.append(
                f"{relative}: BMP mismatch ({len(test_bytes)} vs {len(reference_bytes)} bytes); "
                f"reference={reference.relative_to(args.wpt.resolve())}"
            )

    discovered = len(args.tests)
    failed = len(failures)
    print(f"WPT CSS reftest subset: {discovered} discovered")
    print(f"  passed:  {passed}")
    print(f"  failed:  {failed}")
    print(f"  skipped: 0")
    print(f"  measured pass rate: {passed}/{discovered} ({passed / discovered * 100:.2f}%)")
    for failure in failures:
        print(f"  FAIL {failure}")
    return 0 if not failures else 1


if __name__ == "__main__":
    raise SystemExit(main())
