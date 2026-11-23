import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from windows_process_memory import OwnedProcessMemory


class FakeApi:
    def __init__(self):
        self.entries = [(12, 11), (99, 1), (11, 10), (10, 1)]
        self.created_at = {10: 100, 11: 101, 12: 102, 99: 1}
        self.live = {(10, 100), (11, 101), (12, 102), (99, 1)}
        self.counters = {(pid, stamp): dict(working_set=pid * 10, pagefile=pid,
            peak_working_set=1000, peak_pagefile=1000) for pid, stamp in self.created_at.items()}
        self.opened, self.closed, self.reads = [], [], []
        self.denied = set()

    def snapshot(self): return list(self.entries)
    def open(self, pid):
        self.opened.append(pid)
        return pid, self.created_at[pid]
    def created(self, handle): return handle[-1]
    def running(self, handle): return handle in self.live
    def read(self, handle):
        self.reads.append(handle)
        if handle in self.denied: raise PermissionError("test counter refusal")
        return dict(self.counters[handle])
    def close(self, handle): self.closed.append(handle)


class OwnedMemoryUnitTest(unittest.TestCase):
    def test_discovers_nested_descendants_without_foreign_process_and_samples_current_sums(self):
        api = FakeApi()
        with OwnedProcessMemory(10, ("borrowed", 10, 100), api=api) as collector:
            first = collector.sample("startup")
            self.assertEqual({p["pid"] for p in first["processes"]}, {10, 11, 12})
            self.assertNotIn(99, api.opened)
            self.assertEqual(first["current_working_set_sum_bytes"], 330)
            self.assertEqual(first["current_private_commit_sum_bytes"], 33)
            self.assertTrue(first["attribution_complete"])
            for value in api.counters.values(): value.update(working_set=20, pagefile=2)
            warm = collector.sample("startup")
            self.assertEqual(warm["current_working_set_sum_bytes"], 60)
            self.assertEqual(warm["phase_sampled_peak_working_set_sum_bytes"], 330)
            phase = collector.sample("article")
            self.assertEqual(phase["phase_sampled_peak_working_set_sum_bytes"], 60)
            self.assertEqual(phase["sampled_peak_working_set_sum_bytes"], 330)
            self.assertIn("not PSS", phase["memory_scope"])
        self.assertCountEqual(api.closed, [(10, 100), (11, 101), (12, 102)])
        self.assertNotIn(("borrowed", 10, 100), api.closed)
        collector.close()
        self.assertEqual(len(api.closed), 3)

    def test_reused_pid_is_not_a_live_ancestor_or_a_replacement_for_retained_handle(self):
        api = FakeApi()
        with OwnedProcessMemory(10, ("borrowed", 10, 100), api=api) as collector:
            collector.sample("startup")
            api.live.remove((11, 101))
            api.created_at.update({11: 500, 13: 501})
            api.live.update({(11, 500), (13, 501)})
            api.entries = [(10, 1), (11, 99), (12, 11), (13, 11)]
            later = collector.sample("article")
            self.assertNotIn(13, api.opened)
            self.assertNotIn((11, 500), api.reads)
            old = next(p for p in later["processes"] if p["pid"] == 11)
            self.assertEqual(old["creation_time_100ns"], 101)
            self.assertFalse(old["running"])
            self.assertEqual(later["current_working_set_sum_bytes"], 220)

    def test_child_predating_parent_and_root_identity_mismatch_are_refused(self):
        api = FakeApi()
        api.created_at[13] = 50
        api.entries.append((13, 10))
        with OwnedProcessMemory(10, ("borrowed", 10, 100), api=api) as collector:
            row = collector.sample("startup")
            self.assertNotIn(13, {p["pid"] for p in row["processes"]})
            self.assertIn((13, 50), api.closed)
        api = FakeApi()
        api.created_at[10] = 999
        with self.assertRaises(RuntimeError): OwnedProcessMemory(10, ("borrowed", 10, 100), api=api)
        self.assertEqual(api.closed, [(10, 999)])

    def test_missing_counter_marks_partial_sum_and_does_not_fabricate_zero(self):
        api = FakeApi()
        api.denied.add((12, 102))
        with OwnedProcessMemory(10, ("borrowed", 10, 100), api=api) as collector:
            row = collector.sample("map")
            self.assertFalse(row["attribution_complete"])
            self.assertEqual(row["current_working_set_sum_bytes"], 210)
            missing = next(p for p in row["processes"] if p["pid"] == 12)
            self.assertIsNone(missing["current_working_set_bytes"])
            self.assertTrue(row["errors"])

    def test_late_child_of_exited_parent_is_unknown_not_silently_complete(self):
        api = FakeApi()
        with OwnedProcessMemory(10, ("borrowed", 10, 100), api=api) as collector:
            collector.sample("startup")
            api.live.remove((10, 100))
            api.entries.append((13, 10))
            api.created_at[13] = 110
            row = collector.sample("closed")
            self.assertNotIn(13, api.opened)
            self.assertFalse(row["attribution_complete"])
            self.assertTrue(any(error["pid"] == 13 for error in row["errors"]))


@unittest.skipUnless(os.name == "nt", "Windows readonly process API integration")
class OwnedMemoryWindowsTest(unittest.TestCase):
    def test_small_owned_python_tree_uses_query_handles_and_exits_naturally(self):
        child = "import os,time; data=bytearray(4*1024*1024); print(os.getpid(),flush=True); time.sleep(2)"
        parent = "import os,subprocess,sys,json; data=bytearray(8*1024*1024); child=subprocess.Popen([sys.executable,'-c',sys.argv[1]],stdout=subprocess.PIPE,text=True,creationflags=subprocess.CREATE_NO_WINDOW); print(json.dumps({'root':os.getpid(),'child':int(child.stdout.readline())}),flush=True); child.wait(); child.stdout.close()"
        with subprocess.Popen([sys.executable, "-c", parent, child], stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True, creationflags=subprocess.CREATE_NO_WINDOW) as process:
            ids = json.loads(process.stdout.readline())
            self.assertEqual(ids["root"], process.pid)
            with OwnedProcessMemory(process.pid, process._handle) as collector:
                row = collector.sample("ready")
                by_pid = {p["pid"]: p for p in row["processes"]}
                self.assertTrue({ids["root"], ids["child"]}.issubset(by_pid), row)
                # Windows may create additional launch-support descendants. Include them;
                # every reported member still needs the retained root's actual ancestry.
                for member in row["processes"]:
                    if member["pid"] != ids["root"]:
                        self.assertIn(member["parent_pid"], by_pid, row)
                        self.assertGreaterEqual(member["creation_time_100ns"], by_pid[member["parent_pid"]]["creation_time_100ns"])
                for pid in (ids["root"], ids["child"]):
                    self.assertTrue(by_pid[pid]["running"], row)
                    self.assertIsNotNone(by_pid[pid]["current_working_set_bytes"], row)
                    self.assertIsNotNone(by_pid[pid]["current_private_commit_bytes"], row)
                # An optional Windows support process may exit or deny queries.
                # That remains explicitly partial, never a silently complete sum.
                if row["errors"]: self.assertFalse(row["attribution_complete"], row)
                self.assertGreaterEqual(row["current_private_commit_sum_bytes"], 12 * 1024 * 1024)
                self.assertLess(row["current_private_commit_sum_bytes"], 100 * 1024 * 1024)
                process.wait(timeout=8)
                ended = collector.sample("closed")
                self.assertEqual(ended["current_working_set_sum_bytes"], 0)
            self.assertEqual(process.returncode, 0, process.stderr.read())


if __name__ == "__main__": unittest.main()
