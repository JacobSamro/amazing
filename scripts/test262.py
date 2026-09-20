#!/usr/bin/env python3
"""Phase-aware Test262 runner for amazing-js/examples/test262_host.

Requires Python >=3.10 and PyYAML==6.0.3 (runner tooling only; engine is std-only).
Build the host on Windows: cargo build -p amazing-js --example test262_host --locked
Run: python scripts/test262.py TEST262_ROOT --binary HOST.exe --output results.json

Contract: https://github.com/tc39/test262/blob/3655e7464de3d52643ecddd4b5f9f4f3e7f62398/INTERPRETING.md
Each variant gets a fresh process/realm and a wall-clock deadline. Helpers are
separate scripts, byte-preserved and unmodified; strictness applies to the test
only. Raw sources are unchanged and receive no helpers. Modules/async and other
unavailable host capabilities are explicit unsupported results, not successes.
Ordinary missing language/builtin features are failures, never guessed skips.

Host stdin: little-endian u32 byte length + UTF-8 bytes for each field: protocol
magic, test source, then pairs of helper name/source. Stdout ends with a
TEST262_RESULT JSON record; only successful host exit and protocol 1 are accepted.
Negative matching uses actual phase AND thrown constructor name, not error.name.
The host reports missing constructor wiring as unknown (an engine failure).

Output includes every discovered file, metadata, each variant, exclusions with
reasons, file/variant totals, directories, reason clusters, input/host hashes and
a reproducible classification hash. File status precedence: fail, timeout,
unsupported, pass; fixtures/user exclusions are skipped and have no variants.
The classification hash excludes diagnostics and timing, includes every path,
variant/status/reason, and must match on repeat runs. --compare enforces this.
"""
from __future__ import annotations

import argparse
import collections
import concurrent.futures
import hashlib
import json
import pathlib
import re
import struct
import subprocess
import sys
import time

try:
    import yaml
except ImportError:
    raise SystemExit("Install runner dependency: python -m pip install PyYAML==6.0.3")

PROTOCOL = "AMAZING_TEST262_1"
STATUSES = ("pass", "fail", "timeout", "unsupported", "skipped")
FLAGS = {"onlyStrict", "noStrict", "raw", "module", "async", "generated",
         "CanBlockIsFalse", "CanBlockIsTrue", "non-deterministic"}
FRONTMATTER = re.compile(r"/\*---(.*?)(?:---|\.\.\.)\*/", re.S)


class MetadataError(ValueError):
    pass


class MetadataLoader(yaml.SafeLoader):
    pass


def unique_mapping(loader, node, deep=False):
    # Check explicit duplicates before flattening: YAML merge overrides are valid.
    explicit = set()
    for key_node, _ in node.value:
        if key_node.tag == "tag:yaml.org,2002:merge":
            continue
        key = loader.construct_object(key_node, deep=deep)
        if key in explicit:
            raise MetadataError(f"duplicate metadata key: {key}")
        explicit.add(key)
    loader.flatten_mapping(node)
    result = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        result[key] = loader.construct_object(value_node, deep=deep)
    return result


MetadataLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, unique_mapping)


def metadata(source: str) -> dict:
    match = FRONTMATTER.search(source)
    if not match:
        raise MetadataError("missing YAML frontmatter")
    try:
        data = yaml.load(match.group(1), Loader=MetadataLoader)
    except (yaml.YAMLError, TypeError) as error:
        raise MetadataError(str(error)) from error
    if not isinstance(data, dict):
        raise MetadataError("frontmatter must be a mapping")
    for key in ("includes", "flags", "features", "locale"):
        values = data.get(key, [])
        if not isinstance(values, list) or any(not isinstance(x, str) for x in values):
            raise MetadataError(f"{key} must be a list of strings")
    flags = set(data.get("flags", []))
    if "onlyStrict" in flags and flags & {"noStrict", "raw"}:
        raise MetadataError("conflicting strictness flags")
    negative = data.get("negative")
    if "negative" in data:
        if not isinstance(negative, dict) or negative.get("phase") not in {"parse", "resolution", "runtime"}:
            raise MetadataError("negative.phase must be parse, resolution, or runtime")
        if not isinstance(negative.get("type"), str) or not negative["type"]:
            raise MetadataError("negative.type must be a constructor name")
    return data


def variants(meta: dict) -> list[str]:
    flags = set(meta.get("flags", []))
    if "module" in flags:
        return ["module"]
    if "raw" in flags:
        return ["raw"]
    if "onlyStrict" in flags:
        return ["strict"]
    if "noStrict" in flags:
        return ["non-strict"]
    return ["non-strict", "strict"]


def unavailable(path: str, meta: dict, caps: dict) -> str | None:
    flags = set(meta.get("flags", []))
    unknown = flags - FLAGS
    if unknown:
        return "flag.unknown:" + ",".join(sorted(unknown))
    for flag in ("module", "async"):
        if flag in flags and not caps.get(flag):
            return "host." + flag
    if meta.get("negative", {}).get("phase") == "resolution" and not caps.get("module"):
        return "host.module-resolution"
    if "CanBlockIsTrue" in flags and caps.get("can_block") is not True:
        return "host.CanBlockIsTrue"
    if "CanBlockIsFalse" in flags and caps.get("can_block") is not False:
        return "host.CanBlockIsFalse"
    if "IsHTMLDDA" in meta.get("features", []) and not caps.get("IsHTMLDDA"):
        return "host.IsHTMLDDA"
    if (path.startswith("intl402/") or path.startswith("staging/intl402/")) and not caps.get("intl402"):
        return "host.intl402"
    return None


def framed(fields: list[str]) -> bytes:
    encoded = [field.encode("utf-8") for field in fields]
    return b"".join(struct.pack("<I", len(field)) + field for field in encoded)


def request(source: str, variant: str, helpers: list[tuple[str, str]]) -> bytes:
    if variant == "strict":
        source = '"use strict";\n' + source
    return framed([PROTOCOL, source] + [item for pair in helpers for item in pair])


def result(status: str, reason: str, **details) -> dict:
    return {"status": status, "reason": reason, **details}


def classify(meta: dict, outcome: dict) -> dict:
    if outcome.get("unsupported"):
        return result("unsupported", outcome["unsupported"], observed=outcome)
    phase = outcome["phase"]
    negative = meta.get("negative")
    if phase.startswith("harness-"):
        return result("fail", f"{phase}:{outcome['error_type']}", observed=outcome)
    if negative:
        if phase == "complete":
            return result("fail", "negative.missing-error", observed=outcome)
        if phase != negative["phase"]:
            return result("fail", f"negative.phase:{negative['phase']}!={phase}", observed=outcome)
        if outcome["error_type"] != negative["type"]:
            return result("fail", f"negative.type:{negative['type']}!={outcome['error_type']}", observed=outcome)
        return result("pass", "negative.matched", observed=outcome)
    if phase == "complete":
        return result("pass", "completed")
    return result("fail", f"{phase}:{outcome['error_type']}", observed=outcome)


def execute(binary: pathlib.Path, payload: bytes, timeout: float) -> dict:
    try:
        child = subprocess.run([str(binary)], input=payload, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return result("timeout", "execution.deadline")
    except OSError as error:
        return result("fail", "runner.launch", detail=str(error))
    if child.returncode:
        return result("fail", "host.abnormal-exit", exit_code=child.returncode,
                      detail=child.stderr.decode("utf-8", errors="replace")[-4096:])
    # Only LF delimits the wire protocol. str.splitlines() also splits valid
    # JSON string characters U+0085/U+2028/U+2029 and corrupts diagnostics.
    lines = child.stdout.decode("utf-8", errors="replace").rstrip("\r\n").split("\n")
    try:
        final = lines[-1]
        if not final.startswith("TEST262_RESULT "):
            raise ValueError("missing result frame")
        outcome = json.loads(final[len("TEST262_RESULT "):])
        if outcome.get("protocol") != 1 or outcome.get("phase") not in {
            "parse", "runtime", "harness-parse", "harness-runtime", "complete"
        } or not isinstance(outcome.get("error_type"), str):
            raise ValueError("invalid host result")
        return {"outcome": outcome}
    except (ValueError, IndexError, AttributeError) as error:
        return result("fail", "runner.protocol", detail=str(error))


def helper_sources(root: pathlib.Path, meta: dict, cache: dict) -> list[tuple[str, str]]:
    if "raw" in meta.get("flags", []):
        return []
    names = ["assert.js", "sta.js"]
    if "async" in meta.get("flags", []):
        names.append("doneprintHandle.js")
    names += meta.get("includes", [])
    helpers = []
    for name in names:
        if name not in cache:
            base = (root / "harness").resolve()
            path = (base / name).resolve()
            if not path.is_relative_to(base):
                raise MetadataError("include escapes harness directory: " + name)
            cache[name] = path.read_bytes().decode("utf-8")
        helpers.append((name, cache[name]))
    return helpers


def aggregate(records: list[dict]) -> str:
    for status in ("fail", "timeout", "unsupported", "pass", "skipped"):
        if any(r["status"] == status for r in records):
            return status
    raise ValueError("empty variant list")


def run_file(path: pathlib.Path, root: pathlib.Path, binary: pathlib.Path,
             caps: dict, timeout: float, selected: bool, sources: dict, harness: dict) -> dict:
    name = path.relative_to(root / "test").as_posix()
    record = {"path": name, "variants": []}
    if "_FIXTURE" in path.name:
        return {**record, **result("skipped", "fixture")}
    if not selected:
        return {**record, **result("skipped", "selection.excluded")}
    try:
        source = sources[path].decode("utf-8")
        meta = metadata(source)
        record["metadata"] = {k: meta[k] for k in ("includes", "flags", "features", "negative", "esid", "locale") if k in meta}
        helpers = helper_sources(root, meta, harness)
    except (OSError, UnicodeError, MetadataError) as error:
        return {**record, **result("fail", "runner.input", detail=str(error))}
    reason = unavailable(name, meta, caps)
    for variant in variants(meta):
        if reason:
            verdict = result("unsupported", reason)
        else:
            response = execute(binary, request(source, variant, helpers), timeout)
            verdict = classify(meta, response["outcome"]) if "outcome" in response else response
        record["variants"].append({"variant": variant, **verdict})
    record["status"] = aggregate(record["variants"])
    record["reason"] = ";".join(sorted({v["reason"] for v in record["variants"] if v["status"] == record["status"]}))
    return record


def counts(records: list[dict]) -> dict:
    counter = collections.Counter(r["status"] for r in records)
    return {s: counter[s] for s in STATUSES}


def summarize(records: list[dict]) -> dict:
    all_variants = [v for r in records for v in r["variants"]]
    directories = collections.defaultdict(list)
    by_variant = collections.defaultdict(list)
    reasons = collections.defaultdict(collections.Counter)
    clusters = collections.Counter()
    for r in records:
        parts = r["path"].split("/")
        directories["/".join(parts[:-1][:2]) or "."].append(r)
        for v in r["variants"]:
            by_variant[v["variant"]].append(v)
            reasons[v["status"]][v["reason"]] += 1
            if v["status"] == "fail":
                detail = v.get("observed", {}).get("detail", "")
                detail = re.sub(r"^\d+:\d+:\s*", "", detail)
                # Preserve actual diagnostics, remove locations only. No guessed causes.
                clusters[v["reason"] + (" | " + detail if detail else "")] += 1
        if not r["variants"]:
            reasons[r["status"]][r["reason"]] += 1
    return {"discovered": len(records), "files": counts(records),
            "variant_count": len(all_variants), "variants": counts(all_variants),
            "by_variant": {k: counts(v) for k, v in sorted(by_variant.items())},
            "directories": {k: {"files": counts(v), "variants": counts([x for r in v for x in r["variants"]])}
                            for k, v in sorted(directories.items())},
            "failure_diagnostics": dict(clusters.most_common()),
            "reason_counts": {k: dict(v.most_common()) for k, v in sorted(reasons.items())}}


def sha(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def classification(records: list[dict]) -> list:
    return [[r["path"], r["status"], r["reason"],
             [[v["variant"], v["status"], v["reason"]] for v in r["variants"]]] for r in records]


def self_test(root: pathlib.Path, binary: pathlib.Path, caps: dict) -> None:
    # Runner contracts, independent of how many language tests the engine passes.
    m = metadata('/*---\nincludes:\n - compareArray.js\nflags: [onlyStrict]\nfeatures: [Symbol]\nnegative: {phase: runtime, type: TypeError}\nesid: sec-x\nlocale: [en-US]\ninfo: |\n  multiline\n  YAML\n---*/')
    assert m["includes"] == ["compareArray.js"] and m["locale"] == ["en-US"]
    assert variants(m) == ["strict"]
    summary = summarize([
        {"path": "harness/a.js", "status": "pass", "reason": "completed", "variants": [{"variant": "raw", "status": "pass", "reason": "completed"}]},
        {"path": "harness/b.js", "status": "skipped", "reason": "fixture", "variants": []},
        {"path": "built-ins/Array/x.js", "status": "fail", "reason": "parse:SyntaxError", "variants": [{"variant": "strict", "status": "fail", "reason": "parse:SyntaxError"}]},
    ])
    assert set(summary["directories"]) == {"harness", "built-ins/Array"}
    assert summary["directories"]["harness"]["files"]["skipped"] == 1
    assert summary["variant_count"] == 2 and summary["discovered"] == 3
    assert metadata('/*---\nbase: &x {esid: sec-old}\n<<: *x\nesid: sec-new\n---*/')["esid"] == "sec-new"
    for flags, expected in [([], ["non-strict", "strict"]), (["noStrict"], ["non-strict"]),
                            (["raw"], ["raw"]), (["module"], ["module"])]:
        assert variants({"flags": flags}) == expected
    for text in ['/*---\nflags: raw\n---*/', '/*---\nflags: []\nflags: []\n---*/',
                 '/*---\nnegative: {phase: bogus, type: Error}\n---*/']:
        try:
            metadata(text)
        except MetadataError:
            pass
        else:
            raise AssertionError("invalid metadata accepted")
    raw = '\ufeff/* exact CRLF */\r\n1;\r\n'
    assert request(raw, "raw", []) == framed([PROTOCOL, raw])
    assert request(raw, "strict", []) == framed([PROTOCOL, '"use strict";\n' + raw])
    helpers = helper_sources(root, {"includes": ["compareArray.js", "propertyHelper.js", "deepEqual.js"]}, {})
    assert [n for n, _ in helpers] == ["assert.js", "sta.js", "compareArray.js", "propertyHelper.js", "deepEqual.js"]
    assert all(s.encode() == (root / "harness" / n).read_bytes() for n, s in helpers)
    assert helper_sources(root, {"flags": ["raw"], "includes": ["missing.js"]}, {}) == []
    neg = {"negative": {"phase": "parse", "type": "SyntaxError"}}
    def observed(phase, kind):
        return {"phase": phase, "error_type": kind, "unsupported": None}
    assert classify(neg, observed("parse", "SyntaxError"))["status"] == "pass"
    assert classify(neg, observed("runtime", "SyntaxError"))["status"] == "fail"
    assert classify(neg, observed("parse", "TypeError"))["status"] == "fail"
    assert classify(neg, observed("complete", ""))["status"] == "fail"
    assert classify(neg, observed("harness-parse", "SyntaxError"))["status"] == "fail"
    assert unavailable("x", {"flags": ["async"]}, caps) == "host.async"
    assert unavailable("x", {"flags": ["module"]}, caps) == "host.module"
    assert unavailable("x", {"flags": ["CanBlockIsTrue"]}, caps) == "host.CanBlockIsTrue"
    assert unavailable("x", {"flags": ["CanBlockIsFalse"]}, caps) is None
    def probe(source, meta=None, helpers=None):
        meta = meta or {}
        response = execute(binary, request(source, "non-strict", helpers or []), 5)
        assert "outcome" in response, response
        return classify(meta, response["outcome"])
    assert probe("1+1;")["status"] == "pass"
    for separator in ("\u0085", "\u2028", "\u2029"):
        verdict = probe("throw " + json.dumps(separator) + ";")
        assert verdict["observed"]["phase"] == "runtime"
        assert separator in verdict["observed"]["detail"]
    assert probe("var previousRealmMarker=1;")["status"] == "pass"
    assert probe("if(typeof previousRealmMarker!=='undefined') throw 1;")["status"] == "pass"
    assert probe("let = ;", neg)["status"] == "pass"
    assert probe("throw new SyntaxError('runtime');", neg)["status"] == "fail"
    runtime = {"negative": {"phase": "runtime", "type": "TypeError"}}
    assert probe("throw {name:'TypeError'};", runtime)["status"] == "fail"
    assert probe("throw new RangeError('wrong');", runtime)["status"] == "fail"
    assert probe("TypeError.prototype.constructor=TypeError; throw new TypeError('right');", runtime)["status"] == "pass"
    assert probe("try {$262.createRealm();} catch(e) {};")["reason"] == "host.createRealm"
    assert probe("try {$262.detachArrayBuffer({});} catch(e) {};")["reason"] == "host.detachArrayBuffer"
    assert probe("if($262.evalScript('1+2')!==3) throw 1;")["status"] == "pass"
    assert probe("if($262.global !== globalThis) throw 1;")["status"] == "pass"
    base = helper_sources(root, {}, {})
    assert probe("assert.sameValue(NaN,NaN);", helpers=base)["status"] == "pass"
    assert probe("assert.sameValue(0,-0);", helpers=base)["status"] == "fail"
    assert probe("assert.throws(TypeError,function(){throw new RangeError('wrong');});", helpers=base)["status"] == "fail"
    assert probe("TypeError.prototype.constructor=TypeError; assert.throws(TypeError,function(){throw new TypeError('right');});", helpers=base)["status"] == "pass"
    assert probe("if(order!=='ab') throw 1;", helpers=[("a", "var order='a';"), ("b", "order+='b';")])["status"] == "pass"
    assert probe("1", neg, [("broken", "let = ;")])["reason"].startswith("harness-parse")
    assert execute(binary, request("while(true) {}", "raw", []), 0.2)["status"] == "timeout"
    print("Runner metadata, variants, byte preservation, includes, negative phase/type, host capability, real assertions, isolation/deadline contracts passed.", flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("test262_root", type=pathlib.Path)
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--compare", type=pathlib.Path)
    parser.add_argument("--only", default="")
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--timeout", type=float, default=10)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.jobs < 1 or args.timeout <= 0 or args.limit < 0:
        parser.error("jobs and timeout must be positive; limit must be nonnegative")
    if yaml.__version__ != "6.0.3":
        parser.error("reproducible runner requires PyYAML==6.0.3")
    root = args.test262_root.resolve()
    binary = args.binary.resolve()
    cap_result = subprocess.run([str(binary), "--capabilities"], capture_output=True, check=True, timeout=10)
    caps = json.loads(cap_result.stdout)
    if caps.get("protocol") != 1:
        parser.error("use amazing-js example test262_host, protocol 1")
    if caps.get("module") or caps.get("async"):
        parser.error("protocol 1 supports synchronous classic scripts only")
    if args.self_test:
        self_test(root, binary, caps)
        return 0
    if not args.output:
        parser.error("--output is required to preserve every result")
    files = sorted((root / "test").rglob("*.js"))
    if not files:
        parser.error("no tests discovered")
    selected = [p for p in files if args.only in p.relative_to(root / "test").as_posix()]
    if args.limit:
        selected = selected[:args.limit]
    selected = set(selected)
    # Snapshot the input once. Avoid reopening implicit helpers for every variant,
    # and hash the exact bytes subsequently sent to child processes.
    harness_files = sorted((root / "harness").rglob("*.js"))
    inputs = files + harness_files
    print(f"Snapshotting {len(files)} tests and {len(harness_files)} helpers", flush=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        sources = dict(zip(inputs, pool.map(pathlib.Path.read_bytes, inputs)))
    harness = {p.relative_to(root / "harness").as_posix(): sources[p].decode("utf-8") for p in harness_files}
    manifest = hashlib.sha256()
    for path in inputs:
        manifest.update(path.relative_to(root).as_posix().encode() + b"\0" + hashlib.sha256(sources[path]).digest())
    revision = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, check=True).stdout.decode().strip()
    started = time.monotonic()
    records = []
    args.output.parent.mkdir(parents=True, exist_ok=True)
    journal = args.output.with_suffix(".jsonl")
    with journal.open("w", encoding="utf-8", newline="\n") as stream, concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        pending = {pool.submit(run_file, p, root, binary, caps, args.timeout, p in selected, sources, harness): p for p in files}
        for future in concurrent.futures.as_completed(pending):
            record = future.result()
            records.append(record)
            stream.write(json.dumps(record, ensure_ascii=True, default=str) + "\n")
            if len(records) % 1000 == 0:
                stream.flush()
                print(json.dumps({"completed_files": len(records), "discovered": len(files), "counts": counts(records),
                                  "seconds": round(time.monotonic() - started, 1)}), flush=True)
    records.sort(key=lambda r: r["path"])
    stable = classification(records)
    digest = hashlib.sha256(json.dumps(stable, separators=(",", ":"), ensure_ascii=True).encode()).hexdigest()
    report = {"schema": 1, "test262_revision": revision, "input_sha256": manifest.hexdigest(),
              "host_sha256": sha(binary), "runner_sha256": sha(pathlib.Path(__file__)),
              "pyyaml": yaml.__version__, "python": sys.version, "capabilities": caps,
              "timeout_seconds": args.timeout, "jobs": args.jobs, "filter": args.only, "limit": args.limit,
              "elapsed_seconds": round(time.monotonic() - started, 3), "classification_sha256": digest,
              "summary": summarize(records), "records": records}
    args.output.write_text(json.dumps(report, separators=(",", ":"), ensure_ascii=True, default=str) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in report.items() if k not in {"records", "summary"}}, indent=2), flush=True)
    print(json.dumps({k: v for k, v in report["summary"].items() if k not in {"directories", "failure_diagnostics"}}, indent=2), flush=True)
    if args.compare:
        before = json.loads(args.compare.read_text(encoding="utf-8"))
        fields = ("test262_revision", "input_sha256", "host_sha256", "runner_sha256", "timeout_seconds", "jobs", "capabilities", "pyyaml", "python", "filter", "limit", "classification_sha256")
        differences = [k for k in fields if before[k] != report[k]]
        if differences:
            print("Repeat gate differs: " + ", ".join(differences), file=sys.stderr)
            return 2
        print("Repeat gate: identical inputs and every file/variant classification.", flush=True)
    # Failed conformance is a measured result, not a runner crash. Still return 1 for CI.
    return int(bool(report["summary"]["files"]["fail"] or report["summary"]["files"]["timeout"]))


if __name__ == "__main__":
    sys.exit(main())
