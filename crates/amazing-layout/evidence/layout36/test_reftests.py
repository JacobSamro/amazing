"""Harness regression checks; run on Linux, from the repository root."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("compare", "scripts/wpt_render_compare.py")
compare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compare)


class References(unittest.TestCase):
    def evaluate(self, links):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            test = root / "test.html"
            test.write_text("<p>static</p>")
            candidate = {"path": "test.html", "abs_path": test, "links": [
                {"relation": relation, "href": str(index), "reference_path": root / str(index)}
                for index, (relation, _) in enumerate(links)
            ]}

            def render(exe, source, output, *args):
                value = 0 if source == test else links[int(source.name)][1]
                if isinstance(value, str):
                    return {"classification": value, "detail": value}
                compare.write_ppm(output, 1, 1, bytes([value] * 3))
                return {"classification": "rendered"}

            with patch.object(compare, "invoke_fixed_render", side_effect=render):
                result = compare.run_one_test(candidate, root / "exe", root, root / "out", 1, 1, 1)
            if result["classification"] == "pass":
                artifact = root / "out" / compare.sha256_hex(result["id"])[:16] / "reference.ppm"
                self.assertEqual(compare.sha256_file(artifact), result["reference_sha256"])
            return result

    def test_alternate_match(self):
        result = self.evaluate([("match", 255), ("match", 0)])
        self.assertEqual(result["classification"], "pass")
        self.assertEqual(result["reference_href"], "1")

    def test_no_match(self):
        self.assertEqual(self.evaluate([("match", 255), ("match", 128)])["classification"], "mismatch")

    def test_all_mismatches_required(self):
        self.assertEqual(self.evaluate([("mismatch", 255), ("mismatch", 0)])["classification"], "mismatch")

    def test_mixed_relations(self):
        self.assertEqual(self.evaluate([("match", 0), ("mismatch", 255)])["classification"], "pass")
        self.assertEqual(self.evaluate([("match", 0), ("mismatch", 0)])["classification"], "mismatch")
        result = self.evaluate([("match", 255), ("match", 0), ("mismatch", 0)])
        self.assertEqual(result["classification"], "mismatch")
        self.assertEqual(result["reference_href"], "2")

    def test_failure_is_not_a_successful_mismatch(self):
        self.assertEqual(self.evaluate([("mismatch", "timeout")])["classification"], "timeout")
        self.assertEqual(self.evaluate([("match", 0), ("mismatch", "crash")])["classification"], "crash")

    def test_alternate_match_can_survive_failed_render(self):
        self.assertEqual(self.evaluate([("match", "timeout"), ("match", 0)])["classification"], "pass")
        self.assertEqual(self.evaluate([("match", "timeout"), ("match", 255)])["classification"], "timeout")

    def test_every_reference_is_checked_for_eligibility(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            test = root / "test.html"
            test.write_text('<link rel="match" href="a.html"><link rel="match" href="b.html">')
            (root / "a.html").write_text("static")
            (root / "b.html").write_text("<script>unsafe()</script>")
            self.assertEqual(compare.classify_candidate(test, root)["reason"], "scripted")
            (root / "b.html").unlink()
            self.assertEqual(compare.classify_candidate(test, root)["reason"], "missing-reference")


if __name__ == "__main__":
    unittest.main()
