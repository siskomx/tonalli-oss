import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("push_ai_model", Path(__file__).with_name("push_ai_model.py"))
delivery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(delivery)


class AiDeliveryTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "packs").mkdir()
        self.payload = self.root / "packs/model.gguf"
        self.data = bytes(range(256)) * 3
        self.payload.write_bytes(self.data)
        self.sha = hashlib.sha256(self.data).hexdigest()
        self.manifest = self.root / "packs.toml"
        self.manifest.write_text(
            'manifest_version = 1\n[[packs]]\nid = "model-fixture"\nkind = "gguf"\n'
            f'role = "embedding"\nbytes = {len(self.data)}\nsha256 = "{self.sha}"\npath = "packs/model.gguf"\n',
            encoding="utf-8",
        )
        self.calls = []
        self.overrides = {}
        self.request = None

    def runner(self, command, **kwargs):
        self.calls.append((command, kwargs))
        arguments = command[3:]
        if arguments == ["get-state"]:
            return subprocess.CompletedProcess(command, 0, b"device\n", b"")
        if arguments[:3] == ["shell", "am", "start"]:
            self.request = arguments[arguments.index("request_id") + 1]
            return subprocess.CompletedProcess(command, 0, b"Status: ok\n", b"")
        if arguments[:3] == ["exec-out", "run-as", "org.tonalli.spike"]:
            receipt = dict(version=1, status="ready", request_id=self.request, model_id="model-fixture",
                           sha256=self.sha, bytes=len(self.data), role="embedding",
                           incoming_name=f"ai-input-{self.request}", file_name="model.gguf",
                           incoming_additional_bytes=len(self.data) + 32 * 1024**2,
                           available_bytes=1024**3)
            receipt.update(self.overrides)
            return subprocess.CompletedProcess(command, 0, json.dumps(receipt).encode(), b"")
        if arguments[0] == "push":
            return subprocess.CompletedProcess(command, 0, b"pushed\n", b"")
        self.fail(f"Unexpected command: {command}")

    def run_delivery(self, runner=None):
        return delivery.prepare_and_push("model-fixture", self.manifest, "emulator-5582",
                                         adb="adb-test", runner=runner or self.runner, sleep=lambda _: None)

    def test_one_payload_to_exact_app_created_directory_is_delivery_only(self):
        result = self.run_delivery()
        self.assertEqual(result["status"], "transferred")
        self.assertFalse(result["installed"])
        pushes = [call for call, _ in self.calls if call[3] == "push"]
        self.assertEqual(len(pushes), 1)
        self.assertEqual(pushes[0][-1], f"/sdcard/Android/data/org.tonalli.spike/files/ai-input-{self.request}/model.gguf")
        self.assertTrue(all("-s" in call and "emulator-5582" in call for call, _ in self.calls))
        self.assertFalse(any("chmod" in call or "chown" in call or "rm" in call for call, _ in self.calls))

    def test_corrupt_source_fails_before_any_adb_operation(self):
        self.payload.write_bytes(b"x" * len(self.data))
        with self.assertRaises(ValueError):
            self.run_delivery()
        self.assertEqual(self.calls, [])

    def test_receipt_identity_schema_and_space_refusals_never_push(self):
        for override in [dict(version=True), dict(bytes=True), dict(bytes=1), dict(role="generation"),
                         dict(model_id="other"), dict(sha256="0" * 64), dict(request_id="other"),
                         dict(incoming_name="../../outside"), dict(file_name="outside"),
                         dict(status="failed"), dict(available_bytes=0)]:
            with self.subTest(override=override):
                self.calls.clear()
                self.overrides = override
                with self.assertRaises(ValueError):
                    self.run_delivery()
                self.assertFalse(any(call[3] == "push" for call, _ in self.calls))

    def test_source_change_during_preparation_never_pushes(self):
        def changed(command, **kwargs):
            answer = self.runner(command, **kwargs)
            if command[3:6] == ["exec-out", "run-as", "org.tonalli.spike"]:
                self.payload.write_bytes(self.data + b"changed")
            return answer
        with self.assertRaises(ValueError):
            self.run_delivery(changed)
        self.assertFalse(any(call[3] == "push" for call, _ in self.calls))

    def test_failed_push_preserves_incoming_and_never_claims_installation(self):
        def failed(command, **kwargs):
            if command[3] == "push":
                self.calls.append((command, kwargs))
                return subprocess.CompletedProcess(command, 1, b"", b"failed")
            return self.runner(command, **kwargs)
        with self.assertRaises(RuntimeError):
            self.run_delivery(failed)
        self.assertFalse(any("rm" in call or "chmod" in call for call, _ in self.calls))

    def test_missing_receipt_times_out_without_payload_or_permission_fallback(self):
        def missing(command, **kwargs):
            if command[3:6] == ["exec-out", "run-as", "org.tonalli.spike"]:
                return subprocess.CompletedProcess(command, 0, b"", b"")
            return self.runner(command, **kwargs)
        with patch.object(delivery.time, "monotonic", side_effect=[0, 2]):
            with self.assertRaises(TimeoutError):
                delivery.prepare_and_push("model-fixture", self.manifest, "emulator-5582", runner=missing, timeout=1, sleep=lambda _: None)

    def test_serial_and_timeouts_are_validated_before_io(self):
        for serial, timeout, push_timeout in [("-invalid", 1, 1), ("emulator-5582", float("nan"), 1),
                                              ("emulator-5582", 1, float("inf")), ("emulator-5582", 1, 0)]:
            with self.subTest(serial=serial, timeout=timeout, push_timeout=push_timeout):
                with self.assertRaises(ValueError):
                    delivery.prepare_and_push("model-fixture", self.manifest, serial, runner=self.runner,
                                              timeout=timeout, push_timeout=push_timeout)
        self.assertEqual(self.calls, [])

    def test_bad_manifest_and_escaping_model_path_never_contact_device(self):
        original = self.manifest.read_text(encoding="utf-8")
        malformed = [original.replace('"packs/model.gguf"', '"packs/../model.gguf"'),
                     original.replace('"embedding"', '"other"'),
                     original.replace('bytes = 768', 'bytes = true'),
                     'manifest_version = 1\npacks = ["not a model record"]\n']
        for content in malformed:
            with self.subTest(content=content):
                self.manifest.write_text(content, encoding="utf-8")
                with self.assertRaises(ValueError):
                    self.run_delivery()
                self.assertEqual(self.calls, [])

    def test_oversized_receipt_and_source_change_during_push_are_refused(self):
        def oversized(command, **kwargs):
            if command[3:6] == ["exec-out", "run-as", "org.tonalli.spike"]:
                return subprocess.CompletedProcess(command, 0, b" " * 4097, b"")
            return self.runner(command, **kwargs)
        with self.assertRaises(ValueError):
            self.run_delivery(oversized)

        def changed(command, **kwargs):
            answer = self.runner(command, **kwargs)
            if command[3] == "push":
                self.payload.write_bytes(self.data + b"changed")
            return answer
        with self.assertRaises(ValueError):
            self.run_delivery(changed)


if __name__ == "__main__":
    unittest.main()
