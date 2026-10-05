"""Native filesystem controls only; no fabricated engine/WPT pass records."""
import importlib.util
import json
import pathlib
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("admission", pathlib.Path(__file__).with_name("admission.py"))
admission = importlib.util.module_from_spec(spec)
spec.loader.exec_module(admission)
SOURCE = "a" * 40


class OutputAdmission(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name) / "gate"

    def receipt(self, **changes):
        self.root.mkdir()
        data = dict(source_commit=SOURCE, phase="before", native_free_bytes=32, minimum_free_bytes=24)
        data.update(changes)
        (self.root / "space.summary.json").write_text(json.dumps(data))

    def rejects(self, **changes):
        self.receipt(**changes)
        with self.assertRaises(ValueError):
            admission.admit_output(self.root, SOURCE, "before")

    def test_fresh_output_created(self):
        self.assertEqual(admission.admit_output(self.root, SOURCE, "before"), self.root)
        self.assertTrue(self.root.is_dir())

    def test_recorded_preflight_is_retained_byte_exact(self):
        self.receipt()
        before = (self.root / "space.summary.json").read_bytes()
        admission.admit_output(self.root, SOURCE, "before")
        self.assertEqual((self.root / "space.summary.json").read_bytes(), before)
        (self.root / "workspace.log").write_text("native log")
        with self.assertRaises(ValueError):
            admission.admit_output(self.root, SOURCE, "before")
        self.assertEqual((self.root / "workspace.log").read_text(), "native log")

    def test_wrong_source(self): self.rejects(source_commit="b" * 40)
    def test_wrong_phase(self): self.rejects(phase="after")
    def test_insufficient_capacity(self): self.rejects(native_free_bytes=23)
    def test_missing_required_capacity(self): self.rejects(minimum_free_bytes=None)
    def test_boolean_capacity_is_rejected(self): self.rejects(native_free_bytes=True)
    def test_nonpositive_required_capacity(self): self.rejects(minimum_free_bytes=0)

    def test_unexpected_artifact_preserved(self):
        self.receipt()
        (self.root / "before.summary.json").write_text("old native summary")
        with self.assertRaises(ValueError): admission.admit_output(self.root, SOURCE, "before")
        self.assertEqual((self.root / "before.summary.json").read_text(), "old native summary")

    def test_empty_existing_output_rejected(self):
        self.root.mkdir()
        with self.assertRaises(ValueError): admission.admit_output(self.root, SOURCE, "before")

    def test_symlink_root_rejected(self):
        target = self.root.with_name("target"); target.mkdir(); self.root.symlink_to(target)
        with self.assertRaises(ValueError): admission.admit_output(self.root, SOURCE, "before")
        self.assertTrue(target.is_dir())

    def test_symlink_capacity_rejected(self):
        self.receipt()
        target = self.root.with_name("capacity")
        (self.root / "space.summary.json").rename(target)
        (self.root / "space.summary.json").symlink_to(target)
        with self.assertRaises(ValueError): admission.admit_output(self.root, SOURCE, "before")
        self.assertTrue(target.is_file())

    def test_symlink_parent_rejected(self):
        target = self.root.with_name("real-parent"); target.mkdir()
        linked = self.root.with_name("linked-parent"); linked.symlink_to(target)
        with self.assertRaises(ValueError): admission.admit_output(linked / "gate", SOURCE, "before")
        self.assertEqual(list(target.iterdir()), [])

    def test_after_receipt_is_admitted_without_baseline_mutation(self):
        self.receipt(phase="after", native_free_bytes=14, minimum_free_bytes=12)
        baseline = self.root.with_name("before"); baseline.mkdir()
        (baseline / "before.summary.json").write_text("original baseline")
        admission.admit_output(self.root, SOURCE, "after")
        self.assertEqual((baseline / "before.summary.json").read_text(), "original baseline")


if __name__ == "__main__": unittest.main(verbosity=2)
