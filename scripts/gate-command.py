#!/usr/bin/env python3
"""Wait for a command; keep native output and publish a compact phase receipt."""
import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
import tempfile


def describe(path):
    path = pathlib.Path(path)
    with path.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    return {'path': str(path.resolve()), 'bytes': path.stat().st_size, 'sha256': digest}


def copy_output(source, destination):
    """A failed/missing/truncated output copy is independent of command exit."""
    source, destination = pathlib.Path(source), pathlib.Path(destination)
    result = {'source': str(source), 'destination': str(destination), 'copy_exit': 1}
    try:
        result['native'] = describe(source)
        shutil.copyfile(source, destination)
        result['durable'] = describe(destination)
        if any(result['native'][key] != result['durable'][key] for key in ('bytes', 'sha256')):
            raise OSError('output copy is truncated or changed')
        result['copy_exit'] = 0
    except OSError as error:
        result['error'] = {'type': type(error).__name__, 'errno': error.errno, 'message': str(error)[:800]}
    return result


def run(command, phase, source_commit, output, receipt, extra_copies=()):
    native = pathlib.Path(tempfile.mkdtemp(prefix='amazing-gate-command-'))
    log = native / 'output.log'
    result = {'schema': 1, 'phase': phase, 'source_commit': source_commit,
              'command': command, 'command_exit': None, 'engine_exit': None,
              'wrapper_exit': 3, 'native_receipt': str(native / 'phase.manifest.json')}
    try:
        # The command never writes its standard streams to snapshot-backed HOME.
        with log.open('wb', buffering=0) as stream:
            process = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT)
            result['command_exit'] = process.returncode
            result['engine_exit'] = process.returncode if process.returncode >= 0 else 128 - process.returncode
    except OSError as error:
        result['command_error'] = {'type': type(error).__name__, 'errno': error.errno, 'message': str(error)[:800]}
    try:
        if log.exists():
            result['raw_native'] = describe(log)
            with log.open('rb') as stream:
                stream.seek(max(0, log.stat().st_size - 8192))
                lines = stream.read().decode(errors='replace').splitlines()
            result['diagnostic_tail'] = [line[:400] for line in lines[-12:]]
    except OSError as error:
        result['native_output_error'] = {'errno': error.errno, 'message': str(error)[:800]}
    result['output_copies'] = [copy_output(log, output)]
    result['output_copies'].extend(copy_output(a, b) for a, b in extra_copies)
    copies_ok = all(copy['copy_exit'] == 0 for copy in result['output_copies'])
    result['output_copy_exit'] = 0 if copies_ok else 1
    result['wrapper_exit'] = result['engine_exit'] or (0 if copies_ok and result['engine_exit'] is not None and not result.get('command_error') and not result.get('native_output_error') else 3)
    # Native receipt is available even when durable publication fails.
    native_receipt = native / 'phase.manifest.json'
    try:
        native_receipt.write_text(json.dumps(result, separators=(',', ':')) + '\n')
    except OSError as error:
        result['receipt_native_error'] = {'errno': error.errno, 'message': str(error)[:800]}
        if result['wrapper_exit'] == 0: result['wrapper_exit'] = 3
    published = copy_output(native_receipt, receipt)
    if published['copy_exit']:
        result['receipt_copy_error'] = published
        if result['wrapper_exit'] == 0: result['wrapper_exit'] = 3
        try: native_receipt.write_text(json.dumps(result, separators=(',', ':')) + '\n')
        except OSError: pass  # The CLI still emits the compact failure receipt.
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--phase', required=True)
    parser.add_argument('--source-commit', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--receipt', required=True)
    parser.add_argument('--copy', action='append', default=[], metavar='SOURCE=DESTINATION')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if not command: parser.error('a command is required')
    copies = [pair.split('=', 1) for pair in args.copy]
    if any(len(pair) != 2 for pair in copies): parser.error('--copy requires SOURCE=DESTINATION')
    result = run(command, args.phase, args.source_commit, args.output, args.receipt, copies)
    print(json.dumps(result, separators=(',', ':')), flush=True)
    return result['wrapper_exit']


if __name__ == '__main__':
    raise SystemExit(main())
