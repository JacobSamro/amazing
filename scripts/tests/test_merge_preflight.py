"""Packaging enforcement checks against real temporary Git object stores."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("preflight", Path(__file__).resolve().parents[1] / "merge-preflight.py")
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)


class Packaging(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.repo = Path(self.temporary.name)
        self.original_repo = preflight.REPO
        preflight.REPO = self.repo
        self.git("init", "-q")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "user.name", "Fixture")
        self.write("source.rs", b"initial")
        self.commit("Published base")
        self.base = self.git("rev-parse", "HEAD").strip()

    def tearDown(self):
        preflight.REPO = self.original_repo
        self.temporary.cleanup()

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], stderr=subprocess.DEVNULL).decode()

    def write(self, path, data):
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)

    def commit(self, title):
        self.git("add", ".")
        self.git("commit", "-qm", title)

    def test_compact_ticket_and_historical_evidence(self):
        self.write("crates/example/evidence/old.jsonl", b"x" * (preflight.FILE_LIMIT + 1))
        self.commit("Historical evidence")
        base = self.git("rev-parse", "HEAD").strip()
        self.write("docs/lanes/ticket/compact.json", b"x" * preflight.FILE_LIMIT)
        self.commit("Complete engine area")
        result = preflight.check(base, "HEAD", ready=True)
        self.assertTrue(result["qualified"])
        self.assertEqual(result["tickets"][0]["evidence_bytes"], preflight.FILE_LIMIT)

    def test_oversized_file(self):
        self.write("crates/example/evidence/raw.jsonl", b"x" * (preflight.FILE_LIMIT + 1))
        self.commit("Complete area")
        self.assertFalse(preflight.check(self.base, "HEAD")["qualified"])

    def test_ticket_total_and_independent_ticket_limits(self):
        for n in range(4):
            self.write(f"docs/lanes/ticket/{n}.json", b"x" * preflight.FILE_LIMIT)
        self.commit("First ticket at limit")
        self.write("docs/lanes/second/compact.json", b"x" * preflight.FILE_LIMIT)
        self.commit("Independent second ticket")
        self.assertTrue(preflight.check(self.base, "HEAD")["qualified"])
        self.assertFalse(preflight.check(self.base, "HEAD", ready=True)["qualified"])
        self.write("docs/lanes/third/extra.json", b"x")
        for n in range(4):
            self.write(f"docs/lanes/third/{n}.json", b"x" * preflight.FILE_LIMIT)
        self.commit("Oversized third ticket")
        self.assertFalse(preflight.check(self.base, "HEAD")["qualified"])

    def test_wip_title(self):
        self.write("source.rs", b"changed")
        self.commit("WIP engine area")
        self.assertFalse(preflight.check(self.base, "HEAD")["qualified"])

    def test_merge_history(self):
        self.git("checkout", "-qb", "lane")
        self.write("lane.rs", b"lane")
        self.commit("Lane area")
        self.git("checkout", "-q", "-")
        self.write("other.rs", b"other")
        self.commit("Other area")
        self.git("merge", "--no-ff", "-qm", "Combine branches", "lane")
        self.assertFalse(preflight.check(self.base, "HEAD")["qualified"])


if __name__ == "__main__":
    unittest.main()
