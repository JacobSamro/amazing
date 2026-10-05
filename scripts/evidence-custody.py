#!/usr/bin/env python3
"""Explicit orchestrator custody holds; never allocates or contacts a sandbox."""
import argparse
import hashlib
import importlib.util
import json
import math
import os
import re
from pathlib import Path
import stat
import tempfile
import time

MAX_HOLD_SECONDS = 7200
EXTENSION_SECONDS = 900
DEFAULT_DIRECTORY = Path.home() / '.amazing-evidence-custody'


def endpoint(row):
    if not isinstance(row, dict): raise ValueError('endpoint must be an object')
    spec = importlib.util.spec_from_file_location('completed_job', Path(__file__).with_name('completed-job.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.valid(row)


def private(path, directory=False):
    info = path.lstat()
    if info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise ValueError('custody path is not owned/private')
    if not (stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode)):
        raise ValueError('custody path must be a real directory/file')


def validate(row, path):
    if not isinstance(row, dict) or type(row.get('schema')) is not int or row.get('schema') != 1 or type(row.get('active')) is not bool:
        raise ValueError('invalid custody record')
    endpoint(row['endpoint'])
    raw = bytes.fromhex(row['receipt_hex'])
    if hashlib.sha256(raw).hexdigest() != row['receipt_sha256'] or json.loads(raw) != row['endpoint']:
        raise ValueError('endpoint authentication mismatch')
    expected = row['endpoint']['box_id'] + '-' + row['endpoint']['job_id'] + '.json'
    if path.name != expected:
        raise ValueError('custody identity/path mismatch')
    start, end = row['created_epoch'], row['custody_deadline_epoch']
    if any(type(v) not in (int, float) or not math.isfinite(v) for v in (start, end)) or not 0 < end-start <= MAX_HOLD_SECONDS:
        raise ValueError('invalid finite custody deadline')
    if row.get('reason') not in ('terminal-unconsumed', 'unknown-exit'):
        raise ValueError('invalid custody reason')
    if not row['active'] and not re.fullmatch(r'[0-9a-f]{64}', str(row.get('consumption_sha256', ''))):
        raise ValueError('cleared hold lacks consumption receipt')
    return row


def decision(directory, box_id, now):
    """Ambiguity inhibits managed stops, but never authorizes an extension."""
    if not directory.exists() and not directory.is_symlink():
        return {'held': False, 'extension_seconds': 0, 'records': [], 'errors': []}
    records, errors = [], []
    try:
        private(directory, True)
        paths = list(directory.iterdir())
    except (OSError, ValueError) as error:
        return {'held': True, 'extension_seconds': 0, 'records': [], 'errors': [str(error)]}
    for path in paths:
        try:
            private(path)
            row = validate(json.loads(path.read_bytes()), path)
            if row['active'] and row['endpoint']['box_id'] == box_id:
                records.append(row)
        except (OSError, ValueError, KeyError, TypeError) as error:
            errors.append(path.name + ': ' + str(error))
    # An expired hold remains a stop veto until explicitly consumed. It does
    # not renew TTL: provider expiry remains the finite cost boundary.
    remaining = max((r['custody_deadline_epoch']-now for r in records), default=0)
    return {'held': bool(records or errors),
            'extension_seconds': 0 if errors else max(0, min(EXTENSION_SECONDS, int(remaining))),
            'records': [r['endpoint']['job_id'] for r in records], 'errors': errors}


def publish(path, row, replace=False):
    fd, temporary = tempfile.mkstemp(prefix='.custody-', dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(row, stream, sort_keys=True); stream.write('\n'); stream.flush(); os.fsync(stream.fileno())
        if replace:
            os.replace(temporary, path)
        else:
            os.link(temporary, path)  # fail closed on concurrent registration
        descriptor = os.open(path.parent, os.O_RDONLY)
        try: os.fsync(descriptor)
        finally: os.close(descriptor)
    finally:
        if os.path.exists(temporary): os.unlink(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, default=DEFAULT_DIRECTORY)
    sub = parser.add_subparsers(dest='action', required=True)
    hold = sub.add_parser('hold')
    hold.add_argument('--receipt', type=Path, required=True)
    hold.add_argument('--receipt-sha256', required=True)
    hold.add_argument('--seconds', type=int, required=True)
    hold.add_argument('--reason', choices=['terminal-unconsumed', 'unknown-exit'], required=True)
    clear = sub.add_parser('consume')
    clear.add_argument('--receipt', type=Path, required=True)
    clear.add_argument('--receipt-sha256', required=True)
    clear.add_argument('--consumption-receipt', type=Path, required=True)
    args = parser.parse_args()
    raw = args.receipt.read_bytes()
    if hashlib.sha256(raw).hexdigest() != args.receipt_sha256: raise ValueError('receipt SHA mismatch')
    receipt = endpoint(json.loads(raw))
    lanes = json.loads((Path(__file__).resolve().parent.parent / '.agents/.boat-lanes.json').read_text())
    if receipt['box_id'] not in lanes.values(): raise ValueError('not an Amazing managed box')
    args.directory.mkdir(mode=0o700, exist_ok=True)
    private(args.directory, True)
    path = args.directory / (receipt['box_id'] + '-' + receipt['job_id'] + '.json')
    if args.action == 'hold':
        if not 0 < args.seconds <= MAX_HOLD_SECONDS: raise ValueError('custody duration outside finite bound')
        if path.exists(): raise ValueError('custody record already exists; no implicit renewal')
        now = time.time()
        row = dict(schema=1, active=True, endpoint=receipt, receipt_hex=raw.hex(), receipt_sha256=args.receipt_sha256,
                   created_epoch=now, custody_deadline_epoch=now+args.seconds, reason=args.reason)
    else:
        private(path)
        row = validate(json.loads(path.read_bytes()), path)
        if row['receipt_sha256'] != args.receipt_sha256: raise ValueError('different endpoint receipt')
        row.update(active=False, consumed_epoch=time.time(),
                   consumption_sha256=hashlib.sha256(args.consumption_receipt.read_bytes()).hexdigest())
    publish(path, row, replace=args.action == 'consume')
    print(json.dumps({'path': str(path), 'active': row['active'], 'endpoint': receipt}))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
