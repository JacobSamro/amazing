#!/usr/bin/env python3
"""Bind terminal baseline compact bytes into a later exact command file.

Use only after supported status/fetch authenticated the baseline. No raw
outputs, executable blobs, SSH or lifecycle actions are handled here.
"""
import argparse
import base64
import hashlib
import json
import pathlib
import shlex
import zlib


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--baseline-dir', required=True)
    p.add_argument('--terminal-summary', required=True)
    p.add_argument('--baseline-source', required=True)
    p.add_argument('--after-command-template', required=True)
    p.add_argument('--template-sha256', required=True)
    p.add_argument('--native-baseline-dir', required=True)
    p.add_argument('--output-command', required=True)
    p.add_argument('--output-manifest', required=True)
    a = p.parse_args()
    terminal_path = pathlib.Path(a.terminal_summary)
    terminal_bytes = terminal_path.read_bytes()
    terminal = json.loads(terminal_bytes)
    if (terminal.get('state'), terminal.get('actual_exit'), terminal.get('runner_exit')) != ('TERMINAL', 0, 0):
        p.error('baseline needs authenticated actual terminal0/runner0')
    if terminal['job']['source_commit'] != a.baseline_source:
        p.error('baseline terminal source mismatch')
    root = pathlib.Path(a.baseline_dir)
    gate_bytes = (root / 'gate.summary.json').read_bytes()
    gate = json.loads(gate_bytes)
    if (gate['source_commit'], gate['corpus_role'], gate['job_exit']) != (a.baseline_source, 'before', 0):
        p.error('baseline requires exact successful full corpus before')
    files = {'gate.summary.json': gate_bytes}
    embedded = {}
    for name, receipt in gate['compact_files'].items():
        if pathlib.PurePosixPath(name).name != name or not name.endswith('.json'):
            p.error('baseline compact filename escapes owner')
        if name.endswith('.phase.json'):
            phase = next(r for r in gate['phases'] if r['phase'] + '.phase.json' == name)
            data = (json.dumps(phase, sort_keys=True, separators=(',', ':')) + '\n').encode()
            if digest(data) != receipt['sha256'] or len(data) != receipt['bytes']:
                p.error('authentic embedded phase receipt hash differs')
            embedded[name] = {'sha256': digest(data), 'bytes': len(data)}
            continue
        data = (root / name).read_bytes()
        if digest(data) != receipt['sha256'] or len(data) != receipt['bytes']:
            p.error('baseline compact bytes changed')
        files[name] = data
    template = pathlib.Path(a.after_command_template).read_bytes()
    if digest(template) != a.template_sha256:
        p.error('after command template changed')
    packet = {'source_commit': a.baseline_source,
              'terminal': terminal, 'terminal_sha256': digest(terminal_bytes),
              'files': {name: {'sha256': digest(data), 'data': base64.b64encode(data).decode()}
                        for name, data in files.items()}}
    encoded = base64.b64encode(zlib.compress(json.dumps(packet, separators=(',', ':')).encode(), 9)).decode()
    # Large input travels in command-file/stdin source, never as an exec argv.
    prelude = "set -eu\npython3 - " + shlex.quote(a.native_baseline_dir) + " <<'AUTHENTIC_BASELINE_COMPACT'\n"
    prelude += "import base64,hashlib,json,pathlib,sys,zlib\npacket=json.loads(zlib.decompress(base64.b64decode(" + repr(encoded) + ")))\n"
    prelude += "root=pathlib.Path(sys.argv[1]);root.mkdir(parents=True,exist_ok=False)\n"
    prelude += "for name,row in packet['files'].items():\n data=base64.b64decode(row['data'])\n if pathlib.PurePosixPath(name).name!=name or hashlib.sha256(data).hexdigest()!=row['sha256']: raise ValueError('compact ferry changed')\n (root/name).write_bytes(data)\n"
    prelude += "(root/'baseline-terminal.summary.json').write_text(json.dumps(packet['terminal'],separators=(',',':'))+'\\n')\nAUTHENTIC_BASELINE_COMPACT\n"
    command = prelude.encode() + template
    with pathlib.Path(a.output_command).open('xb') as stream:
        stream.write(command)
    manifest = {'schema': 1, 'baseline_source': a.baseline_source,
                'baseline_job': terminal['job'], 'terminal_summary_sha256': digest(terminal_bytes),
                'files': {name: {'sha256': digest(data), 'bytes': len(data)} for name, data in files.items()},
                'embedded_phase_receipts': embedded,
                'template_sha256': digest(template), 'command_sha256': digest(command),
                'command_bytes': len(command), 'native_baseline_dir': a.native_baseline_dir,
                'qualification': 'Authentic compact ferry only; no cross-box ELF baseline validation'}
    with pathlib.Path(a.output_manifest).open('x') as stream:
        stream.write(json.dumps(manifest, sort_keys=True, separators=(',', ':')) + '\n')
    print(json.dumps({'command_sha256': digest(command), 'compact_files': len(files), 'baseline_job': terminal['job']['job_id']}))


if __name__ == '__main__':
    main()
