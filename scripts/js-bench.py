#!/usr/bin/env python3
"""Run the amazing-js benchmark programs and report median time, peak RSS and
VM-vs-fallback coverage. Run on a build box (see AGENTS.md), not on the Mac.

  scripts/js-bench.py [--runs 7] [--out result.json] [--no-build]
                      [--bin PATH] [--programs DIR] [--only a,b] [--no-node]

Modes (all use the same release `js_bench_host` example):
  default  what shipped hosts do today: top-level code on the tree walker,
           functions/arrows/methods compiled to bytecode when possible.
  vm       AMAZING_JS_VM=1: eligible top-level programs also run as bytecode.
  ast      AMAZING_JS_AST_FUNCTIONS=1: ordinary functions stay on the tree walker.
  wrapped  default mode, but the whole program is wrapped in an IIFE so its loops
           run inside a function (bytecode) instead of at tree-walker top level.
Time is the host's own in-process BENCH_MS (script run only). Peak RSS comes
from wait4 ru_maxrss of the child process. Checksums (stdout) must agree across
modes, runs and, when present, Node.js (`node --jitless`, default `node`).
"""
import argparse, collections, json, os, pathlib, shutil, statistics, subprocess, sys, tempfile, time

ROOT = pathlib.Path(__file__).resolve().parent.parent
MODES = {"default": {}, "tree": {"AMAZING_JS_VM": "0"}, "vm": {"AMAZING_JS_VM": "1"}, "ast": {"AMAZING_JS_AST_FUNCTIONS": "1"}, "wrapped": {}}
NODE_PRELOAD = """globalThis.print = (s) => console.log(String(s));
const t0 = process.hrtime.bigint();
process.on('exit', () => { process.stderr.write('BENCH_MS ' + (Number(process.hrtime.bigint() - t0) / 1e6).toFixed(3) + '\\n'); });
"""


def run_measured(cmd, env, timeout=300):
    """Run with wait4 so ru_maxrss belongs to exactly this child."""
    r_out, w_out = os.pipe(); r_err, w_err = os.pipe()
    start = time.perf_counter()
    pid = os.posix_spawn(cmd[0], cmd, env, file_actions=[
        (os.POSIX_SPAWN_DUP2, w_out, 1), (os.POSIX_SPAWN_DUP2, w_err, 2),
        (os.POSIX_SPAWN_CLOSE, r_out), (os.POSIX_SPAWN_CLOSE, r_err)])
    os.close(w_out); os.close(w_err)
    import threading
    chunks = {}
    def drain(name, fd):
        with os.fdopen(fd, "rb") as handle: chunks[name] = handle.read()
    threads = [threading.Thread(target=drain, args=("o", r_out)), threading.Thread(target=drain, args=("e", r_err))]
    for t in threads: t.start()
    _, status, usage = os.wait4(pid, 0)
    for t in threads: t.join()
    wall = time.perf_counter() - start
    code = os.waitstatus_to_exitcode(status)
    return code, chunks["o"].decode(errors="replace"), chunks["e"].decode(errors="replace"), wall, usage.ru_maxrss


def bench_ms(stderr):
    for line in stderr.splitlines():
        if line.startswith("BENCH_MS "):
            return float(line.split()[1])
    return None


def summarize_stats(path):
    prog = collections.Counter(); funcs = collections.Counter(); reasons = collections.Counter(); calls = [0, 0]
    if path.exists():
        for line in path.read_text().splitlines():
            f = line.split("\t")
            if len(f) < 3: continue
            if f[1] == "P": prog[f[2] + (":" + f[3] if len(f) > 3 and f[3] else "")] += 1
            elif f[1] == "F":
                funcs[f[2]] += 1
                if f[2] == "fail": reasons[f[3] if len(f) > 3 else ""] += 1
            elif f[1] == "C": calls[0] += int(f[2]); calls[1] += int(f[3])
    return {"programs": dict(prog), "function_compiles": dict(funcs), "function_fail_reasons": dict(reasons),
            "calls_vm": calls[0], "calls_tree": calls[1]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=7)
    ap.add_argument("--out")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--bin")
    ap.add_argument("--programs", default=str(ROOT / "crates/amazing-js/benches/programs"))
    ap.add_argument("--only")
    ap.add_argument("--no-node", action="store_true")
    ap.add_argument("--modes", default="default,tree,wrapped,vm,ast")
    args = ap.parse_args()

    binary = pathlib.Path(args.bin) if args.bin else None
    if binary is None:
        target = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
        if not args.no_build:
            subprocess.run(["cargo", "build", "-p", "amazing-js", "--example", "js_bench_host", "--release", "--locked"], cwd=ROOT, check=True)
        binary = target / "release/examples/js_bench_host"
    programs = sorted(pathlib.Path(args.programs).glob("*.js"))
    if args.only:
        wanted = set(args.only.split(","))
        programs = [p for p in programs if p.stem in wanted]
    node = None if args.no_node else shutil.which("node")
    node_version = subprocess.run([node, "--version"], capture_output=True, text=True).stdout.strip() if node else None
    scratch = pathlib.Path(tempfile.mkdtemp(prefix="js-bench-"))
    preload = scratch / "preload.js"; preload.write_text(NODE_PRELOAD)
    base_env = {k: v for k, v in os.environ.items() if not k.startswith("AMAZING_JS_")}
    results = {"binary": str(binary), "runs": args.runs, "node_version": node_version, "programs": {}}
    modes = args.modes.split(",")
    for program in programs:
        entry = {}
        checksums = set()
        wrapped = scratch / (program.stem + ".wrapped.js")
        wrapped.write_text("(function () {\n" + program.read_text() + "\n})();\n")
        targets = [(m, [str(binary), str(wrapped if m == "wrapped" else program)], MODES[m]) for m in modes]
        if node:
            targets += [("node-jitless", [node, "--jitless", "--require", str(preload), str(program)], {}),
                        ("node", [node, "--require", str(preload), str(program)], {})]
        for name, cmd, extra in targets:
            env = dict(base_env); env.update(extra)
            times, walls, rss = [], [], []
            for _ in range(args.runs):
                code, out, err, wall, maxrss = run_measured(cmd, env)
                if code != 0:
                    entry[name] = {"error": (err or out)[-300:]}; break
                checksums.add(out.strip()); times.append(bench_ms(err)); walls.append(wall * 1000); rss.append(maxrss)
            else:
                entry[name] = {"median_ms": round(statistics.median(times), 2), "min_ms": round(min(times), 2),
                               "median_wall_ms": round(statistics.median(walls), 1), "peak_rss_mb": round(max(rss) / 1024, 1)}
        for m in ("default", "vm"):
            if m in modes and "error" not in entry.get(m, {}):
                stats = scratch / f"{program.stem}.{m}.tsv"
                env = dict(base_env); env.update(MODES[m]); env["AMAZING_JS_VM_STATS"] = str(stats)
                run_measured([str(binary), str(program)], env)
                entry[m]["coverage"] = summarize_stats(stats)
        entry["checksum"] = sorted(checksums)[0] if len(checksums) == 1 else None
        entry["checksum_agrees"] = len(checksums) == 1
        results["programs"][program.stem] = entry
        d = entry.get("default", {}); v = entry.get("vm", {}); a = entry.get("ast", {})
        print(f"{program.stem:16} default {d.get('median_ms','-'):>9} ms  vm {v.get('median_ms','-'):>9}  ast {a.get('median_ms','-'):>9}  wrapped {entry.get('wrapped',{}).get('median_ms','-'):>9}"
              f"  rss {d.get('peak_rss_mb','-')} MB  node-jitless {entry.get('node-jitless',{}).get('median_ms','-')}  node {entry.get('node',{}).get('median_ms','-')}"
              f"  checksum_ok={entry['checksum_agrees']}", flush=True)
    if args.out:
        pathlib.Path(args.out).write_text(json.dumps(results, indent=1, sort_keys=True) + "\n")
    shutil.rmtree(scratch, ignore_errors=True)
    return 0 if all(e["checksum_agrees"] for e in results["programs"].values()) else 1


if __name__ == "__main__":
    sys.exit(main())
