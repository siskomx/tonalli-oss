"""Mocked delivery protocol only: never invokes adb."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import push_index_probe as push


class PushTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        source = self.root/"source"
        (source/"generation-test").mkdir(parents=True)
        (source/"CURRENT").write_bytes(b"generation-test")
        for name in ["meta.json","tonalli-meta.json"]:
            (source/"generation-test"/name).write_bytes(b"{}")
        self.bundle = self.root/"bundle"
        self.trusted = self.root/"trusted.json"
        self.trusted.write_bytes(push.exporter.export(source,self.bundle))

    def fake(self, failed=False):
        state = {"calls":[]}
        def run(command, **kwargs):
            self.assertEqual(command[:3],["fake-adb","-s","explicit-serial"])
            self.assertTrue(kwargs["capture_output"])
            args = command[3:]
            state["calls"].append(args)
            if args == ["get-state"]:
                output = b"device\n"
            elif args[:3] == ["shell","am","start"]:
                state["request"] = args[args.index("request_id")+1]
                state["hash"] = args[args.index("manifest_sha256")+1]
                output = b"Status: ok\n"
            elif args[:4] == ["exec-out","run-as",push.PACKAGE,"cat"]:
                self.assertEqual(args[4],f"files/probe-inputs-ready/{state['request']}.json")
                output = json.dumps({"version":1,"request_id":state["request"],"manifest_sha256":state["hash"],
                                     "status":"failed" if failed else "ready","error":"OwnershipMismatch" if failed else "",
                                     "incoming_name":"probe-index-"+state["hash"][:12],"generation":"generation-test","benchmark_ids":["small","full"]}).encode()
            elif args[0] == "push":
                self.assertTrue(Path(args[1]).is_file(), "individual files only")
                self.assertTrue(args[2].startswith(f"/sdcard/Android/data/{push.PACKAGE}/files/probe-index-"))
                output = b"1 file pushed\n"
            else:
                self.fail(f"Disallowed command: {args}")
            return subprocess.CompletedProcess(command,0,output,b"")
        return state,run

    def test_app_prepares_before_individual_push_and_snapshot_is_last(self):
        state,runner = self.fake()
        report = push.prepare_and_push(self.bundle,self.trusted,"explicit-serial","fake-adb",runner=runner)
        self.assertEqual(report["pushed_files"],4)
        pushes = [c for c in state["calls"] if c[0] == "push"]
        self.assertTrue(pushes[-1][1].endswith("snapshot.json"))
        self.assertFalse(any("mkdir" in c or "chmod" in c or "rm" in c for c in state["calls"]))

    def test_wrong_ownership_receipt_fails_before_any_payload(self):
        state,runner = self.fake(failed=True)
        with self.assertRaisesRegex(ValueError,"OwnershipMismatch"):
            push.prepare_and_push(self.bundle,self.trusted,"explicit-serial","fake-adb",runner=runner)
        self.assertFalse(any(c[0] == "push" for c in state["calls"]))

    def test_empty_or_exact_missing_receipt_with_zero_exit_is_retried(self):
        state,fake = self.fake()
        polls = 0
        def runner(command,**kwargs):
            nonlocal polls
            reply = fake(command,**kwargs)
            if "exec-out" in command:
                polls += 1
                if polls == 1:
                    reply.stdout = b""
                elif polls == 2:
                    reply.stdout = f"cat: {command[-1]}: No such file or directory\r\n".encode()
            return reply
        report = push.prepare_and_push(self.bundle,self.trusted,"explicit-serial","fake-adb",runner=runner,sleep=lambda _:None)
        self.assertEqual(polls,3)
        self.assertEqual(report["pushed_files"],4)

    def test_missing_receipt_deadline_stops_before_push(self):
        state,fake = self.fake()
        def runner(command,**kwargs):
            reply = fake(command,**kwargs)
            if "exec-out" in command:
                reply.stdout = f"cat: {command[-1]}: No such file or directory\n".encode()
            return reply
        with patch.object(push.time,"monotonic",side_effect=[0,1]):
            with self.assertRaises(TimeoutError):
                push.prepare_and_push(self.bundle,self.trusted,"explicit-serial","fake-adb",runner=runner,timeout=.1)
        self.assertFalse(any(c[0] == "push" for c in state["calls"]))

    def test_other_malformed_receipt_is_not_retried(self):
        state,fake = self.fake()
        def runner(command,**kwargs):
            reply = fake(command,**kwargs)
            if "exec-out" in command:
                reply.stdout = b"cat: another-path: No such file or directory\n"
            return reply
        with self.assertRaises(json.JSONDecodeError):
            push.prepare_and_push(self.bundle,self.trusted,"explicit-serial","fake-adb",runner=runner,sleep=lambda _:self.fail("no retry"))
        self.assertFalse(any(c[0] == "push" for c in state["calls"]))

    def test_local_escape_or_changed_payload_is_rejected_without_adb(self):
        (self.bundle/"generation-test/meta.json").write_bytes(b"changed")
        with self.assertRaises(ValueError):
            push.prepare_and_push(self.bundle,self.trusted,"explicit-serial",runner=lambda *a,**k:self.fail("no adb"))

    def test_manifest_path_escape_is_rejected_without_adb(self):
        manifest = json.loads(self.trusted.read_bytes())
        manifest["files"][1]["path"] = "generation-test/../outside"
        raw = json.dumps(manifest).encode()
        self.trusted.write_bytes(raw)
        (self.bundle/"snapshot.json").write_bytes(raw)
        with self.assertRaises(ValueError):
            push.prepare_and_push(self.bundle,self.trusted,"explicit-serial",runner=lambda *a,**k:self.fail("no adb"))

    def test_stale_receipt_is_rejected_before_push(self):
        state,fake = self.fake()
        def runner(command,**kwargs):
            result = fake(command,**kwargs)
            if "exec-out" in command:
                receipt = json.loads(result.stdout)
                receipt["request_id"] = "stale"
                result.stdout = json.dumps(receipt).encode()
            return result
        with self.assertRaises(ValueError):
            push.prepare_and_push(self.bundle,self.trusted,"explicit-serial","fake-adb",runner=runner)
        self.assertFalse(any(c[0]=="push" for c in state["calls"]))

    def test_failed_payload_never_publishes_snapshot_or_deletes(self):
        state,fake = self.fake()
        def runner(command,**kwargs):
            result = fake(command,**kwargs)
            if command[3]=="push":
                result.returncode = 1
            return result
        with self.assertRaises(RuntimeError):
            push.prepare_and_push(self.bundle,self.trusted,"explicit-serial","fake-adb",runner=runner)
        pushes = [c for c in state["calls"] if c[0]=="push"]
        self.assertEqual(len(pushes),1)
        self.assertFalse(pushes[0][1].endswith("snapshot.json"))
        self.assertFalse(any("rm" in c or "chmod" in c or "mkdir" in c for c in state["calls"]))


if __name__ == "__main__":
    unittest.main()
