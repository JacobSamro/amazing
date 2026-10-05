#!/usr/bin/env python3
"""Own the D-Bus lifetime of an actual Linux capture command (stdlib only)."""
import argparse
import json
import os
import re
from pathlib import Path
import signal
import subprocess
import sys
import time

STALE_BUS_VARIABLES = (
    'DBUS_SESSION_BUS_ADDRESS', 'DBUS_SESSION_BUS_PID',
    'DBUS_SESSION_BUS_WINDOWID', 'DBUS_STARTER_BUS_TYPE',
    'DBUS_STARTER_ADDRESS', 'AT_SPI_BUS_ADDRESS',
)


def call(destination, path, method, *arguments):
    command = ['dbus-send', '--session', '--print-reply', '--reply-timeout=3000',
               '--dest=' + destination, path, method, *arguments]
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=5)
        return {'actual_exit': result.returncode, 'stdout': result.stdout[:4096],
                'stderr': result.stderr[:4096]}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {'actual_exit': None, 'error': str(error)[:4096]}


def facts():
    # These calls use the actual new session, not any inherited bus/AT-SPI address.
    guid = call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                'org.freedesktop.DBus.GetId')
    match = re.search(r'\bstring \"([0-9a-fA-F]{32})\"', guid.get('stdout', ''))
    return {'recorded_ns': time.time_ns(),
            'guid_value': match.group(1).lower() if match else None,
            'session_address': os.environ.get('DBUS_SESSION_BUS_ADDRESS'),
            'guid': guid,
            'bus_owner_pid': call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                                  'org.freedesktop.DBus.GetConnectionUnixProcessID',
                                  'string:org.freedesktop.DBus'),
            # NameHasOwner does not activate an absent service. Service absence is
            # an observation, never a reason to disable the accessibility backend.
            'atspi_has_owner': call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                                    'org.freedesktop.DBus.NameHasOwner',
                                    'string:org.a11y.Bus'),
            'atspi_owner': call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                                'org.freedesktop.DBus.GetNameOwner',
                                'string:org.a11y.Bus')}


def publish(path, value):
    encoded = json.dumps(value, separators=(',', ':')) + '\n'
    if len(encoded.encode()) > 65536:
        raise ValueError('session metadata exceeds 64 KiB')
    temporary = path.with_name(path.name + '.updating')
    with temporary.open('x') as output:
        output.write(encoded)
    temporary.replace(path)


def stop_owned_child(child, number, errors):
    """Reap the actual child before bus teardown, including diagnostic failures."""
    if child is None:
        return True
    try:
        if child.poll() is None:
            try:
                child.send_signal(number)
            except ProcessLookupError:
                pass
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            try:
                child.kill()
            except ProcessLookupError:
                pass
            child.wait(timeout=5)
        return True
    except (OSError, subprocess.TimeoutExpired) as error:
        errors.append({'phase': 'owned-child-cleanup', 'error': str(error)[:4096]})
        return child.poll() is not None


def run_inside(receipt, command):
    record = {'schema': 1, 'kind': 'owned-linux-capture-session',
              'state': 'STARTING', 'source_commit': os.environ.get('SOURCE'),
              'custodian_pid': os.getpid(), 'command': command,
              'accessibility_disabled_by_adapter': False,
              'begin': facts(), 'child_actual_exit': None,
              'diagnostic_errors': []}
    if record['begin']['guid']['actual_exit'] != 0 or not record['begin']['guid_value']:
        record['state'] = 'BUS_ADMISSION_FAILED'
        publish(receipt, record)
        return 3
    publish(receipt, record)
    child = None
    received_signal = None
    errors = record['diagnostic_errors']

    def forward(number, _frame):
        nonlocal received_signal
        received_signal = number
        if child is not None and child.poll() is None:
            try:
                child.send_signal(number)
            except ProcessLookupError:
                pass
            except OSError as error:
                errors.append({'phase': 'signal-forward', 'error': str(error)[:4096]})

    previous = {number: signal.signal(number, forward)
                for number in (signal.SIGTERM, signal.SIGINT)}
    try:
        # A signal cached while the handlers were installed cancels startup.
        # If it arrives during Popen, forward it immediately after assignment.
        if received_signal is None:
            child = subprocess.Popen(command)
            record['child_pid'] = child.pid
            if received_signal is not None:
                forward(received_signal, None)
            record['state'] = 'RUNNING'
            publish(receipt, record)
            while child.poll() is None and received_signal is None:
                try:
                    child.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    pass
        else:
            record['startup_cancelled'] = True
    except (OSError, ValueError) as error:
        errors.append({'phase': 'child-start-or-running-publication',
                       'error': str(error)[:4096]})
    finally:
        # The live bus belongs to this whole finally path. A failed RUNNING
        # publication must not drop it underneath an already started shell.
        record['child_reaped'] = stop_owned_child(
            child, received_signal or signal.SIGTERM, errors)
        record['child_actual_exit'] = None if child is None else child.returncode
        record['end'] = facts()
        record['received_signal'] = received_signal
        record['state'] = 'TERMINAL'
        record['session_live_at_end'] = record['end']['guid']['actual_exit'] == 0
        record['same_bus_at_end'] = (
            record['session_live_at_end'] and
            record['begin']['guid_value'] == record['end']['guid_value'])
        try:
            publish(receipt, record)
        except (OSError, ValueError) as error:
            errors.append({'phase': 'terminal-publication', 'error': str(error)[:4096]})
            # A filesystem fault can also prevent the terminal receipt. Keep
            # actual cleanup/status facts in the supervisor's captured stderr.
            print(json.dumps(record, separators=(',', ':')), file=sys.stderr)
        finally:
            for number, handler in previous.items():
                signal.signal(number, handler)
    code = record['child_actual_exit']
    if errors or not record['child_reaped']:
        return 3
    if code is None:
        return 128 + received_signal if received_signal else 3
    if code == 0 and (not record['same_bus_at_end'] or received_signal):
        return 3 if not received_signal else 128 + received_signal
    return code if code >= 0 else 128 - code


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--receipt', required=True)
    parser.add_argument('--inside-owned-session', action='store_true')
    parser.add_argument('--dbus-config')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command[:1] == ['--']:
        command = command[1:]
    if not command:
        parser.error('a command is required')
    receipt = Path(args.receipt).absolute()
    if args.inside_owned_session:
        return run_inside(receipt, command)
    receipt.parent.mkdir(parents=True, exist_ok=True)
    # O_EXCL reservation protects prior proofs before starting any session.
    with receipt.open('x') as reservation:
        json.dump({'schema': 1, 'state': 'SESSION_LAUNCHING'}, reservation)
    environment = os.environ.copy()
    removed = [name for name in STALE_BUS_VARIABLES if name in environment]
    for name in STALE_BUS_VARIABLES:
        environment.pop(name, None)
    inside = [sys.executable, str(Path(__file__).resolve()),
              '--inside-owned-session', '--receipt', str(receipt), '--', *command]
    try:
        launcher = ['dbus-run-session']
        if args.dbus_config:
            launcher += ['--config-file', str(Path(args.dbus_config).resolve(strict=True))]
        child = subprocess.Popen([*launcher, '--', *inside], env=environment,
                                 start_new_session=True)
        forwarded = []
        def forward(number, _frame):
            # The inner custodian performs ordered shell/X teardown while its
            # session bus remains live. Do not broadcast into that bus first.
            try:
                inner = json.loads(receipt.read_text()).get('custodian_pid')
                if inner and os.getpgid(inner) == child.pid:
                    os.kill(inner, number)
                    forwarded.append(number)
                    return
            except (OSError, ValueError):
                pass
            try:
                if child.poll() is None:
                    child.send_signal(number)
            except ProcessLookupError:
                pass
            forwarded.append(number)
        previous = {number: signal.signal(number, forward)
                    for number in (signal.SIGTERM, signal.SIGINT)}
        try:
            child.wait()
        finally:
            for number, handler in previous.items():
                signal.signal(number, handler)
        result = child
    except OSError as error:
        publish(receipt, {'schema': 1, 'state': 'SESSION_START_FAILED',
                          'startup_error': str(error)[:4096],
                          'sanitized_inherited_variables': removed})
        return 3
    if receipt.exists():
        record = json.loads(receipt.read_text())
        record['session_wrapper_actual_exit'] = result.returncode
        record['forwarded_signals'] = forwarded
        record['sanitized_inherited_variables'] = removed
        record['session_wrapper_finished_ns'] = time.time_ns()
        publish(receipt, record)
    return result.returncode if result.returncode >= 0 else 128 - result.returncode


if __name__ == '__main__':
    raise SystemExit(main())
