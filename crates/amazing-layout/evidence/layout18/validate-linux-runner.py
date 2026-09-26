"""Small remote parity checks for reference selection and pixel metrics."""
import importlib.util
import tempfile
from pathlib import Path

spec = importlib.util.spec_from_file_location('render_linux', Path(__file__).with_name('render-linux.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    a, b, diff = [root / name for name in ['a.ppm', 'b.ppm', 'diff.ppm']]
    a.write_bytes(b'P6\n2 1\n255\n' + bytes([0, 10, 20, 255, 254, 253]))
    b.write_bytes(b'P6\n2 1\n255\n' + bytes([0, 10, 20, 248, 254, 253]))
    assert runner.compare(a, b, diff) == (1, 7)
    assert runner.ppm(diff)[2] == bytes([255, 255, 255, 255, 0, 255])
    assert runner.compare(a, a, diff) == (0, 0)
    assert runner.ppm(diff)[2] == bytes([255] * 6)
    source = '<link rel="help" href="https://example.invalid"><link href="b.html?x=1#h" rel="mismatch"><link rel="match" href="ignored.html">'
    assert runner.reference(source, root / 'a.html', root) == ('mismatch', 'b.html?x=1#h', root / 'b.html')
    assert runner.reference('<link rel=match href="b.html ">', root / 'a.html', root)[2] == root / 'b.html'
    assert runner.fuzzy('<meta name="fuzzy" content="maxDifference=0-7;totalPixels=0-3">') == (7, 3)
    assert runner.fuzzy('') == (0, 0)
print('Linux render runner: pixel counts, channel maximum, diff bytes, first reftest link, fuzzy limits verified')
