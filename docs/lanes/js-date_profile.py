"""Phase timing diagnostic using unchanged pinned regExpUtils.js (not a gate)."""
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import time

root = Path(os.environ['TEST262_ROOT'])
binary = Path(os.environ['CARGO_TARGET_DIR']) / 'release/examples/test262_host'
helper = (root / 'harness/regExpUtils.js').read_text()
source = r'''
for (var count of [65536, 1114112]) {
  var started = Date.now();
  var text = buildString({loneCodePoints:[], ranges:[[0,count-1]]});
  print('construction '+count+' '+(Date.now()-started)+'ms length '+text.length);
  started = Date.now();
  var matched = /^\p{Any}+$/u.test(text);
  print('regexp Any '+count+' '+(Date.now()-started)+'ms matched '+matched);
  started = Date.now();
  matched = /^\P{ASCII}+$/u.test(text.slice(128));
  print('regexp nonASCII '+count+' '+(Date.now()-started)+'ms matched '+matched);
}
'''
fields = ['AMAZING_TEST262_1', source, 'regExpUtils.js', helper]
packet = b''.join(struct.pack('<I', len(encoded)) + encoded
                  for encoded in (field.encode() for field in fields))
started = time.monotonic()
result = subprocess.run([str(binary)], input=packet, capture_output=True,
                        timeout=300)
record = dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
              helper_sha256=hashlib.sha256(helper.encode()).hexdigest(),
              elapsed_seconds=time.monotonic()-started, exit_code=result.returncode,
              stdout=result.stdout.decode(errors='replace'),
              stderr=result.stderr.decode(errors='replace'),
              mode='default engine; standalone phase timing, no conformance claim')
output = Path(os.environ['OUT']) / 'jsd7-profile.json'
output.write_text(json.dumps(record, indent=2)+'\n')
print(json.dumps(record), flush=True)
assert result.returncode == 0, 'profiling host failed'
