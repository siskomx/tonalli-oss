import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("notes_export", Path(__file__).with_name("export-notes-probe.py"))
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)


class CiphertextInspection(unittest.TestCase):
    def test_streamed_fingerprint(self):
        with tempfile.TemporaryDirectory(prefix="tonalli-export-test-") as folder:
            file = Path(folder) / "notes.db"
            payload = b"\x9e" * (128 * 1024 + 7)
            file.write_bytes(payload)
            result = exporter.inspect(file)
            self.assertEqual(result["sha256"], hashlib.sha256(payload).hexdigest())
            self.assertEqual(result["bytes"], len(payload))

    def test_marker_across_buffer_boundary_is_rejected(self):
        with tempfile.TemporaryDirectory(prefix="tonalli-export-test-") as folder:
            file = Path(folder) / "notes.db"
            file.write_bytes(b"\x9e" * (65536 - 5) + exporter.MARKER + b"tail")
            with self.assertRaises(exporter.CheckFailed):
                exporter.inspect(file)

    def test_sqlite_plaintext_header_is_rejected(self):
        with tempfile.TemporaryDirectory(prefix="tonalli-export-test-") as folder:
            file = Path(folder) / "notes.db"
            file.write_bytes(b"SQLite format 3\0" + b"payload")
            with self.assertRaises(exporter.CheckFailed):
                exporter.inspect(file)


if __name__ == "__main__":
    unittest.main()
