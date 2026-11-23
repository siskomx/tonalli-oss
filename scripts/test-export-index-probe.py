"""Small-fixture Android transfer invariants; no phone results are inferred."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


class ExportTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("exporter", Path(__file__).with_name("export-index-probe.py"))
        self.exporter = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.exporter)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        generation = self.source / "generation-test"
        generation.mkdir(parents=True)
        (self.source / "CURRENT").write_bytes(b"generation-test")
        (generation / "meta.json").write_bytes(b"{}")
        (generation / "tonalli-meta.json").write_bytes(b"{}")
        (generation / ".tantivy-meta.lock").write_bytes(b"")

    def test_verified_deterministic_bundle_excludes_locks(self):
        first = self.exporter.export(self.source, self.root / "first")
        second = self.exporter.export(self.source, self.root / "second")
        self.assertEqual(first, second)
        manifest = json.loads(first)
        self.assertEqual(manifest["generation"], "generation-test")
        self.assertEqual(len(manifest["files"]), 3)
        for item in manifest["files"]:
            payload = (self.root / "first" / item["path"]).read_bytes()
            self.assertEqual(len(payload), item["bytes"])
            self.assertEqual(hashlib.sha256(payload).hexdigest(), item["sha256"])
        self.assertEqual((self.root / "first/snapshot.json").read_bytes(), first)

    def test_refuses_overwrite(self):
        target = self.root / "output"
        target.mkdir()
        (target / "sentinel").write_bytes(b"keep")
        with self.assertRaises(FileExistsError):
            self.exporter.export(self.source, target)
        self.assertEqual((target / "sentinel").read_bytes(), b"keep")

    def test_rejects_path_escape_without_publishing(self):
        (self.source / "CURRENT").write_bytes(b"../generation-test")
        target = self.root / "output"
        with self.assertRaises(ValueError):
            self.exporter.export(self.source, target)
        self.assertFalse(target.exists())

    def test_rejects_unbounded_payload_without_publishing(self):
        self.exporter.MAX_BYTES = 1
        target = self.root / "output"
        with self.assertRaises(ValueError):
            self.exporter.export(self.source, target)
        self.assertFalse(target.exists())

    def test_rejects_nested_directory(self):
        (self.source / "generation-test/nested").mkdir()
        with self.assertRaises(ValueError):
            self.exporter.export(self.source, self.root / "output")


if __name__ == "__main__":
    unittest.main()
