"""Standalone ordinary-callable timing probe; no conformance classification."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.path.insert(0, "scripts")
import test262

cases = {
    "numeric_loop": ("function f(n){let total=0;for(let k=0;k<n;k++){total+=k}return total}", 100000, 4999950000),
    "array_growth": ("function f(n){let a=[];for(let k=0;k<n;k++){a[k]=k}return a.length}", 30000, 30000),
    "array_truncate": ("function f(n){let a=[],total=0;for(let r=0;r<n;r++){for(let k=0;k<10000;k++){a[k]=k}total+=a.length;a.length=0}return total}", 3, 30000),
    "object_tail_delete": ("function f(n){let a={};for(let k=0;k<n;k++){a[k]=k}for(let k=n-1;k>=0;k--){delete a[k]}return Object.keys(a).length}", 10000, 0),
    "codepoint_apply": ("function f(n){let a=[],total=0;for(let k=0;k<10000;k++){a[k]=k}for(let r=0;r<n;r++){total+=String.fromCodePoint.apply(null,a).length}return total}", 20, 200000),
}
harness = (Path(os.environ["TEST262_ROOT"]) / "harness/regExpUtils.js").read_text()
builder = harness + "\nfunction f(n){return buildString({loneCodePoints:[],ranges:[[0,n-1]]}).length}"
cases["buildString_scaled"] = (builder, 30000, 30000)
cases["buildString_full"] = (builder, 1114112, 2162688)
binary = Path(os.environ["CARGO_TARGET_DIR"]) / "release/examples/test262_host"
records = []
for backend in ["ast_factories", "default", "opt_in_vm"]:
    for name, (body, n, expected) in cases.items():
        source = body + f"\nvar started=Date.now();var result=f({n});if(result!=={expected})throw 'BAD:'+result;throw 'elapsed='+String(Date.now()-started);"
        env = os.environ.copy()
        env.pop("AMAZING_JS_VM", None)
        env.pop("AMAZING_JS_AST_FUNCTIONS", None)
        env.pop("AMAZING_JS_VM_COMPILE_LOG", None)
        if backend == "ast_factories":
            env["AMAZING_JS_AST_FUNCTIONS"] = "1"
        elif backend == "opt_in_vm":
            env["AMAZING_JS_VM"] = "1"
        started = time.monotonic()
        try:
            proc = subprocess.run([str(binary)], input=test262.request(source, "non-strict", []),
                                  capture_output=True, env=env, timeout=120)
            record = dict(backend=backend, case=name, n=n, expected=expected,
                          wall_seconds=round(time.monotonic()-started, 6),
                          returncode=proc.returncode, response=proc.stdout.decode(),
                          stderr=proc.stderr.decode())
            assert "elapsed=" in record["response"], record
        except subprocess.TimeoutExpired:
            record = dict(backend=backend, case=name, n=n, timeout_seconds=120)
        records.append(record)
        print(json.dumps(record), flush=True)
report = dict(base=os.environ["PROFILE_REF"], host_sha256=test262.sha(binary),
              harness_sha256=test262.sha(Path(os.environ["TEST262_ROOT"]) / "harness/regExpUtils.js"),
              records=records)
(Path(os.environ["OUT"]) / "jsvm17-profile-after.json").write_text(json.dumps(report, indent=2)+"\n")
