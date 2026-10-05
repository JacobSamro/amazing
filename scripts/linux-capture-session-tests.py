#!/usr/bin/env python3
"""Bulk native Linux tests of real D-Bus sessions; run only via linux-run."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

TOOL = Path(__file__).with_name('linux-capture-session.py')


class OwnedSession(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='amazing-native-session-controls-')
        self.root = Path(self.temp.name)
        self.receipt = self.root / 'capture.session.manifest.json'

    def tearDown(self):
        self.temp.cleanup()

    def run_child(self, body, extra=(), environment=None):
        child = self.root / 'child.py'
        child.write_text(body)
        result = subprocess.run([sys.executable, str(TOOL), '--receipt',
                                 str(self.receipt), *extra, '--',
                                 sys.executable, str(child)], env=environment, timeout=40)
        return result.returncode, json.loads(self.receipt.read_text())

    def test_real_live_bus_and_actual_exit(self):
        code, facts = self.run_child("import subprocess\nsubprocess.run(['dbus-send','--session','--print-reply','--dest=org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus.GetId'],check=True)\n")
        self.assertEqual(code, 0)
        self.assertTrue(facts['same_bus_at_end'])
        self.assertEqual(facts['child_actual_exit'], 0)
        self.assertLess(facts['begin']['recorded_ns'], facts['end']['recorded_ns'])
        self.assertFalse(facts['accessibility_disabled_by_adapter'])

    def test_stale_inherited_session_and_atspi_are_replaced(self):
        env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS='unix:path=/missing/stale-bus',
                   AT_SPI_BUS_ADDRESS='unix:path=/missing/stale-atspi',
                   DBUS_STARTER_ADDRESS='unix:path=/missing/starter')
        code, facts = self.run_child("import os\nassert 'AT_SPI_BUS_ADDRESS' not in os.environ\nassert 'DBUS_STARTER_ADDRESS' not in os.environ\nassert '/missing/' not in os.environ['DBUS_SESSION_BUS_ADDRESS']\n", environment=env)
        self.assertEqual(code, 0)
        self.assertIn('AT_SPI_BUS_ADDRESS', facts['sanitized_inherited_variables'])
        self.assertTrue(facts['same_bus_at_end'])

    def test_real_child_failure_is_preserved(self):
        code, facts = self.run_child('raise SystemExit(23)\n')
        self.assertEqual(code, 23)
        self.assertEqual(facts['child_actual_exit'], 23)
        self.assertTrue(facts['session_live_at_end'])

    def test_existing_receipt_is_not_overwritten(self):
        self.receipt.write_text('original proof')
        result = subprocess.run([sys.executable, str(TOOL), '--receipt',
                                 str(self.receipt), '--', 'true'], timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.receipt.read_text(), 'original proof')

    def test_genuine_bus_disconnect_cannot_certify_success(self):
        body = """import os,re,signal,subprocess,time
text=subprocess.check_output(['dbus-send','--session','--print-reply','--dest=org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus.GetConnectionUnixProcessID','string:org.freedesktop.DBus'],text=True)
pid=int(re.search(r'uint32 (\\d+)',text).group(1))
os.kill(pid,signal.SIGTERM)
time.sleep(0.3)
"""
        code, facts = self.run_child(body)
        self.assertNotEqual(code, 0)
        self.assertFalse(facts['same_bus_at_end'])

    def test_no_atspi_service_is_recorded_without_disabling_backend(self):
        # Native dbus-daemon configuration intentionally has no service dirs.
        config = self.root / 'no-services.conf'
        config.write_text('<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>')
        code, facts = self.run_child("import os\nassert os.environ.get('NO_AT_BRIDGE') != '1'\n", extra=('--dbus-config', str(config)))
        self.assertEqual(code, 0)
        self.assertIn('boolean false', facts['begin']['atspi_has_owner']['stdout'])
        self.assertNotEqual(facts['begin']['atspi_owner']['actual_exit'], 0)
        self.assertFalse(facts['accessibility_disabled_by_adapter'])

    def test_actual_atspi_owner_disconnect_keeps_session_bus_distinct(self):
        body = """import os,re,signal,subprocess,time
base=['dbus-send','--session','--print-reply','--reply-timeout=3000']
subprocess.run(base+['--dest=org.a11y.Bus','/org/a11y/bus','org.a11y.Bus.GetAddress'],check=True)
text=subprocess.check_output(base+['--dest=org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus.GetConnectionUnixProcessID','string:org.a11y.Bus'],text=True)
pid=int(re.search(r'uint32 (\\d+)',text).group(1))
assert os.getpgid(pid)==os.getpgrp(), 'AT-SPI service owner is outside the actual owned session; do not signal it'
os.kill(pid,signal.SIGTERM)
time.sleep(.3)
subprocess.run(base+['--dest=org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus.GetId'],check=True)
"""
        code, facts = self.run_child(body)
        self.assertEqual(code, 0, 'native AT-SPI activation/owned-service custody must qualify; absence cannot be treated as success')
        self.assertTrue(facts['same_bus_at_end'])
        self.assertIn('boolean false', facts['end']['atspi_has_owner']['stdout'])

    def boundary_harness(self, mode):
        # Real run_inside, real bus/child, and precise real OS fault/signal.
        harness = """import importlib.util,json,os,signal,subprocess,sys,time
from pathlib import Path
spec=importlib.util.spec_from_file_location('custodian',%r)
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
root=Path(%r);parent=root/'inner';parent.mkdir()
receipt=parent/'proof.session.manifest.json';moved=root/'inner-detached'
ready=root/'ready';cleaned=root/'cleaned';started=root/'started'
child_file=root/'owned-child.py'
child_file.write_text('''import signal,subprocess,time
from pathlib import Path
ready=Path(%%r);cleaned=Path(%%r);parent=Path(%%r);moved=Path(%%r)
def done(number,frame):
 if moved.exists():moved.rename(parent)
 text=subprocess.check_output(['dbus-send','--session','--print-reply','--dest=org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus.GetId'],text=True)
 cleaned.write_text(text)
 raise SystemExit(143)
signal.signal(signal.SIGTERM,done)
ready.write_text('real child ready')
while True:time.sleep(.02)
''' %% (str(ready),str(cleaned),str(parent),str(moved)))
command=[sys.executable,str(child_file)]
real_popen=module.subprocess.Popen;real_signal=module.signal.signal
children=[];fired=False;mode=%r
def spawn(args,*a,**k):
 proc=real_popen(args,*a,**k)
 if args==command:
  children.append(proc);started.write_text(str(proc.pid))
  deadline=time.monotonic()+10
  while not ready.exists() and time.monotonic()<deadline:time.sleep(.02)
  assert ready.exists(),'actual child failed to start'
  if mode=='publication-fault':parent.rename(moved)
  if mode=='signal-during-spawn':os.kill(os.getpid(),signal.SIGTERM)
 return proc
def install(number,handler):
 global fired
 previous=real_signal(number,handler)
 if mode=='signal-before-spawn' and number==signal.SIGINT and callable(handler) and not fired:
  fired=True;os.kill(os.getpid(),signal.SIGTERM)
 return previous
module.subprocess.Popen=spawn;module.signal.signal=install
try:code=module.run_inside(receipt,command)
finally:
 module.subprocess.Popen=real_popen;module.signal.signal=real_signal
 if moved.exists():moved.rename(parent)
facts=json.loads(receipt.read_text())
assert facts['same_bus_at_end'],facts
assert facts['child_reaped'],facts
if mode=='signal-before-spawn':
 assert code==143 and not children and not started.exists(),facts
 assert facts['startup_cancelled'] and facts['child_actual_exit'] is None,facts
else:
 assert len(children)==1 and children[0].wait(timeout=1)==143,facts
 assert cleaned.exists() and 'string' in cleaned.read_text(),facts
 assert facts['child_actual_exit']==143,facts
 if mode=='publication-fault':assert code==3 and facts['diagnostic_errors'],facts
 else:assert code==143 and not facts['diagnostic_errors'],facts
""" % (str(TOOL), str(self.root), mode)
        code, facts = self.run_child(harness)
        self.assertEqual(code, 0)
        self.assertTrue(facts['same_bus_at_end'])

    def test_running_receipt_actual_filesystem_fault_reaps_live_child(self):
        self.boundary_harness('publication-fault')

    def test_termination_after_handlers_prevents_new_child(self):
        self.boundary_harness('signal-before-spawn')

    def test_signal_during_actual_spawn_is_forwarded_before_wait(self):
        self.boundary_harness('signal-during-spawn')

    def test_actual_capture_shell_function_reaps_children_before_bus_exit(self):
        # Execute the exact public Bash function prefix/traps against real owned
        # native processes. This is lifecycle coverage, never a fake UI proof.
        body = """import json,os,signal,subprocess,sys,time
from pathlib import Path
root=Path(%r);tool=Path(%r);remote=tool.with_name('linux-shell-capture-remote.sh')
text=remote.read_text();prefix=text.split('cd crates/amazing-shell-gpui',1)[0]
child=root/'shell-child.py';ready=root/'native-owned-children';cleaned=root/'shell-cleaned'
child.write_text('''import signal,subprocess,time
from pathlib import Path
def done(number,frame):
 text=subprocess.check_output(['dbus-send','--session','--print-reply','--dest=org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus.GetId'],text=True)
 Path(%%r).write_text(text)
 raise SystemExit(143)
signal.signal(signal.SIGTERM,done)
Path(%%r).write_text('child handler installed')
while True:time.sleep(.02)
''' %% (str(cleaned),str(root/'handler-ready')))
script=root/'capture-cleanup.sh'
# The prefix contains actual stop_owned_child, traps, and the actual function
# process boundary. No duplicated teardown algorithm or bus facts are supplied.
end=')' if 'capture_owned_window() (' in prefix else '}'
script.write_text(prefix+'''\npython3 "$1" > /dev/null 2>&1 & APID=$!
Xvfb :$((100+$$%%400)) -screen 0 320x200x24 -nolisten tcp > /dev/null 2>&1 & XPID=$!
printf '%%s %%s\\n' "$APID" "$XPID" > "$2"
wait "$APID"
'''+end+'\\ncapture_owned_window "$@"\\n')
receipt=root/'real-shell.session.manifest.json'
env=dict(os.environ,HELPER='')
proc=subprocess.Popen([sys.executable,str(tool),'--receipt',str(receipt),'--','bash',str(script),str(child),str(ready)],env=env)
pids=[]
try:
 deadline=time.monotonic()+20
 while time.monotonic()<deadline:
  if ready.exists() and (root/'handler-ready').exists():
   pids=[int(x) for x in ready.read_text().split()]
   if len(pids)==2:break
  time.sleep(.05)
 else:raise AssertionError('actual Bash children never ready')
 proc.send_signal(signal.SIGTERM)
 assert proc.wait(timeout=25)==143
 facts=json.loads(receipt.read_text())
 assert facts['child_reaped'] and facts['same_bus_at_end'],facts
 assert cleaned.exists() and 'string' in cleaned.read_text()
 for pid in pids:assert not Path('/proc/'+str(pid)).exists(),('owned child survives',pid)
finally:
 if proc.poll() is None:proc.kill();proc.wait()
 # On a failed control, clean only the captured native processes this harness
 # created; this is not provider or unrelated-process signalling.
 for pid in pids:
  if Path('/proc/'+str(pid)).exists():
   try:os.kill(pid,signal.SIGKILL)
   except ProcessLookupError:pass
""" % (str(self.root), str(TOOL))
        code, facts = self.run_child(body)
        self.assertEqual(code, 0)
        self.assertTrue(facts['same_bus_at_end'])

    def test_custodian_signal_allows_child_cleanup_on_live_bus(self):
        child = self.root / 'child.py'
        cleaned = self.root / 'cleaned.json'
        child.write_text("""import json,signal,subprocess,time
from pathlib import Path
def done(number,frame):
 text=subprocess.check_output(['dbus-send','--session','--print-reply','--dest=org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus.GetId'],text=True)
 Path(%r).write_text(json.dumps({'live_bus':bool(text)}))
 raise SystemExit(143)
signal.signal(signal.SIGTERM,done)
while True:time.sleep(.05)
""" % str(cleaned))
        proc = subprocess.Popen([sys.executable, str(TOOL), '--receipt', str(self.receipt), '--', sys.executable, str(child)])
        try:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                try:
                    facts = json.loads(self.receipt.read_text())
                    if facts.get('state') == 'RUNNING':
                        break
                except (OSError, ValueError):
                    pass
                time.sleep(.05)
            else:
                self.fail('native custodian never entered RUNNING')
            # Child installs its real signal handler before this control signal.
            time.sleep(.3)
            proc.send_signal(signal.SIGTERM)
            self.assertEqual(proc.wait(timeout=20), 143)
            self.assertTrue(json.loads(cleaned.read_text())['live_bus'])
            facts = json.loads(self.receipt.read_text())
            self.assertTrue(facts['same_bus_at_end'])
            self.assertEqual(facts['received_signal'], signal.SIGTERM)
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait()


if __name__ == '__main__':
    if sys.platform != 'linux':
        raise SystemExit('These real native lifecycle controls require Linux/Boat')
    unittest.main()
