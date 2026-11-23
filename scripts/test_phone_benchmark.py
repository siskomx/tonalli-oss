"""Local/mocked checks only; never invokes adb or touches a phone."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import phone_benchmark as bench
import index_handoff


def result(mode="warm"):
    spec = bench.DATASETS["small"]
    return {"version": 1, "run_id": "test_01", "dataset": "small", "manifest_sha256": "a"*64,
            "mode": mode, "status": "complete", "source": "internal_mmap", "cache_state": "uncontrolled_os_cache",
            "acceptance_eligible": False,
            "pid": 123, "process_started_elapsed_ms": 456, "api_level": 34,
            "ram": {"advertised_bytes": 0, "total_bytes": 1000, "available_bytes": 500},
            "zim_sha256": spec["zim"]["sha256"], "title_index_sha256": spec["title_index"]["sha256"],
            "prefix": spec["prefix"], "article_url": spec["article_url"], "percentile_method": "nearest_rank",
            "setup_ms": dict.fromkeys(["native_init", "open", "attach", "total"], 1.0),
            "priming": {"suggest": 1 if mode == "warm" else 0, "article": 1 if mode == "warm" else 0},
            "samples": {"suggest_ms": [1.0]*30 if mode == "warm" else ([1.0] if mode == "fresh-suggest" else []),
                        "article_ms": [2.0]*30 if mode == "warm" else ([2.0] if mode == "fresh-article" else [])}}


def manual_receipt(manifest_hash="a"*64):
    return {"version": 1, "dataset_id": "small", "manifest_sha256": manifest_hash,
            "selection_revision": "55d93ba2-98da-48b5-856a-7cfd094baaa1", "selection_fingerprint": "b"*64,
            "observed_logical_zim_sha256": bench.DATASETS["small"]["zim"]["sha256"],
            "observed_logical_zim_bytes": 14093352, "observed_at_utc": "2026-11-11T20:00:00Z",
            "medium": "emulator virtual SD", "card_class": "not physical", "filesystem": "vfat",
            "evidence": "external_manual_not_rechecked"}


class PhoneBenchmarkTests(unittest.TestCase):
    def test_named_index_handoff_assets_match_catalog_source_and_transfer_manifest(self):
        assets = bench.ROOT / 'android/app/src/main/assets'
        for dataset in ['full', 'medicine']:
            with self.subTest(dataset=dataset):
                manifest = bench.manifest_for(dataset)
                self.assertEqual((assets/f'benchmark-{dataset}.json').read_bytes(), bench.json_bytes(manifest))
                descriptor = index_handoff.validate_descriptor((assets/f'index-probe-{dataset}.json').read_bytes())
                self.assertEqual(descriptor['dataset_id'], dataset)
                self.assertEqual(descriptor['source']['bytes'], manifest['zim']['bytes'])
                self.assertEqual(descriptor['source']['expected_sha256'], manifest['zim']['sha256'])

    def test_manual_receipt_is_bounded_and_never_implies_android_hash_verification(self):
        manifest = bench.manifest_for("small")
        raw = bench.json_bytes(manual_receipt())
        context = bench.validate_saf_receipt(raw, manifest, "a"*64)
        self.assertEqual(context["sha256"], hashlib.sha256(raw).hexdigest())
        self.assertEqual(context["receipt"]["evidence"], "external_manual_not_rechecked")
        for changes in [{"version": True}, {"observed_logical_zim_bytes": True},
                        {"observed_logical_zim_sha256": "c"*64}, {"selection_revision": "../bad"},
                        {"observed_at_utc": "2026-11-11"}, {"evidence": "android_verified"},
                        {"uri": "content://private/selection"}, {"medium": "x"*129}]:
            invalid = {**manual_receipt(), **changes}
            with self.assertRaises(ValueError):
                bench.validate_saf_receipt(bench.json_bytes(invalid), manifest, "a"*64)
        with self.assertRaises(ValueError):
            bench.validate_saf_receipt(b" "*16385, manifest, "a"*64)

    def test_saf_results_separate_expected_sha_from_manual_observation_and_reselection(self):
        context = bench.validate_saf_receipt(bench.json_bytes(manual_receipt()), bench.manifest_for("small"), "a"*64)
        value = result()
        del value["zim_sha256"]
        value.update(source="saf_pread", expected_zim_sha256=bench.DATASETS["small"]["zim"]["sha256"],
                     expected_zim_bytes=14093352,
                     parts=[{"ordinal": 0, "bytes": 14093352}],
                     selection={"revision": manual_receipt()["selection_revision"], "fingerprint": "b"*64, "kind": "FILES", "part_count": 1},
                     external_manual={"receipt_sha256": context["sha256"],
                                      **{k: manual_receipt()[k] for k in ["observed_logical_zim_sha256", "observed_logical_zim_bytes", "observed_at_utc", "medium", "card_class", "filesystem"]},
                                      "verification": "external_manual_not_rechecked"})
        bench.validate_result(value, bench.manifest_for("small"), "a"*64, "test_01", "warm", 123, saf=context)
        for change in [lambda r: r.update(zim_sha256="c"*64),
                       lambda r: r["selection"].update(revision="55d93ba2-98da-48b5-856a-7cfd094baaa2"),
                       lambda r: r["external_manual"].update(verification="android_verified"),
                       lambda r: r["selection"].update(kind="ISO")]:
            invalid = copy.deepcopy(value)
            change(invalid)
            with self.assertRaises(ValueError):
                bench.validate_result(invalid, bench.manifest_for("small"), "a"*64, "test_01", "warm", 123, saf=context)

    def test_known_manifest_has_exact_small_and_full_pins(self):
        self.assertEqual(bench.manifest_for("small")["zim"]["bytes"], 14093352)
        self.assertEqual(bench.manifest_for("full")["zim"]["bytes"], 52690706555)

    def test_counts_hashes_and_live_pid_are_required(self):
        good = result()
        bench.validate_result(good, bench.manifest_for("small"), "a"*64, "test_01", "warm", 123)
        for mutate in [lambda r: r["samples"]["suggest_ms"].pop(), lambda r: r.update(pid=124),
                       lambda r: r.update(zim_sha256="b"*64), lambda r: r.update(status="failed")]:
            invalid = copy.deepcopy(good)
            mutate(invalid)
            with self.assertRaises(ValueError):
                bench.validate_result(invalid, bench.manifest_for("small"), "a"*64, "test_01", "warm", 123)

    def test_nearest_rank_is_not_interpolated(self):
        self.assertEqual(bench.percentiles(list(range(1,31))), {"count":30,"p50_ms":15,"p95_ms":29})

    def test_invalid_durations_and_priming_are_rejected(self):
        for value in [-1, float("nan"), float("inf"), True]:
            invalid = result()
            invalid["samples"]["article_ms"][0] = value
            with self.assertRaises(ValueError):
                bench.validate_result(invalid, bench.manifest_for("small"), "a"*64, "test_01", "warm", 123)
        invalid = result("fresh-article")
        invalid["priming"]["suggest"] = 1
        with self.assertRaises(ValueError):
            bench.validate_result(invalid, bench.manifest_for("small"), "a"*64, "test_01", "fresh-article", 123)

    def test_reference_export_never_reads_or_copies_full_pack(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pack, table = root/"pack", root/"table"
            pack.write_bytes(b"reference corpus")
            table.write_bytes(b"table")
            spec = {"name":"synthetic", "prefix":"x", "article_url":"C/X", "pack_path":"pack", "table_path":"table",
                    "zim":{"bytes":pack.stat().st_size,"sha256":hashlib.sha256(pack.read_bytes()).hexdigest()},
                    "title_index":{"bytes":5,"sha256":bench.digest(table)}, "receipt_path":"receipt.json"}
            receipt = {"actualSha256":spec["zim"]["sha256"],"identity":bench.identity(pack)}
            (root/"receipt.json").write_bytes(bench.json_bytes(receipt))
            actual_digest = bench.digest
            def no_pack_hash(path):
                self.assertNotEqual(Path(path), pack)
                return actual_digest(path)
            with patch.object(bench, "digest", side_effect=no_pack_hash), patch.object(bench.shutil, "copyfile", side_effect=AssertionError("no full copy")):
                exported = bench.export_inputs(root, root/"out", ["full"], {"full":spec})
            self.assertEqual(exported["status"], "complete")
            self.assertFalse((root/"out/full/pack.zim").exists())
            with self.assertRaises(FileExistsError):
                bench.export_inputs(root, root/"out", ["full"], {"full":spec})
            receipt["identity"]["mtimeNs"] = "changed"
            (root/"receipt.json").write_bytes(bench.json_bytes(receipt))
            with self.assertRaises(ValueError):
                bench.export_inputs(root, root/"bad", ["full"], {"full":spec})
            self.assertFalse((root/"bad/full").exists())

    def test_small_copies_verified_before_publication(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/"pack").write_bytes(b"pack")
            (root/"table").write_bytes(b"table")
            spec = {"name":"synthetic", "prefix":"x", "article_url":"C/X", "pack_path":"pack", "table_path":"table",
                    "zim":{"bytes":4,"sha256":bench.digest(root/"pack")}, "title_index":{"bytes":5,"sha256":bench.digest(root/"table")}}
            bench.export_inputs(root, root/"good", ["small"], {"small":spec})
            self.assertEqual((root/"good/small/pack.zim").read_bytes(), b"pack")
            with patch.object(bench.shutil, "copyfile", side_effect=lambda _, destination: Path(destination).write_bytes(b"evil")):
                with self.assertRaises(ValueError):
                    bench.export_inputs(root, root/"bad", ["small"], {"small":spec})
            self.assertFalse((root/"bad/small").exists())
            self.assertEqual(json.loads((root/"bad/export.json").read_bytes())["status"], "failed")

    def fake_adb(self, corrupt=False, saf=None, stale_selection=False):
        state = {"alive":False,"pid":100,"calls":[]}
        def run(command, **options):
            self.assertEqual(command[:3], ["adb-test", "-s", "explicit-serial"])
            self.assertTrue(options["capture_output"])
            args = command[3:]
            state["calls"].append(args)
            output, code = b"", 0
            if args == ["get-state"]:
                output = b"device\n"
            elif args == ["shell", "am", "force-stop", bench.PACKAGE]:
                state["alive"] = False
            elif args == ["shell", "pidof", bench.PACKAGE]:
                output, code = (str(state["pid"]).encode()+b"\n", 0) if state["alive"] else (b"", 1)
            elif args[:3] == ["shell", "am", "start"]:
                self.assertEqual(args[args.index("-a")+1], bench.PACKAGE + (".SAF_BENCHMARK" if saf else ".BENCHMARK"))
                if saf:
                    for key, value in {"selection_revision": saf["receipt"]["selection_revision"],
                                       "selection_fingerprint": saf["receipt"]["selection_fingerprint"],
                                       "manual_receipt_sha256": saf["sha256"]}.items():
                        self.assertEqual(args[args.index(key)+1], value)
                state["alive"], state["pid"] = True, state["pid"]+1
                state["run_id"], state["mode"], state["hash"] = [args[args.index(k)+1] for k in ["run_id","mode","manifest_sha256"]]
                output = b"Starting: Intent\nStatus: ok\n"
            elif args[:4] == ["exec-out", "run-as", bench.PACKAGE, "cat"]:
                if saf and args[4] == "files/benchmarks/saf-selection.json":
                    selection = {"version": 1, "revision": saf["receipt"]["selection_revision"],
                                 "fingerprint": "b"*64, "kind": "FILES", "part_count": 1}
                    if stale_selection:
                        selection["revision"] = "55d93ba2-98da-48b5-856a-7cfd094baaa2"
                    return subprocess.CompletedProcess(command, 0, bench.json_bytes(selection), b"")
                self.assertEqual(args[4], f"files/benchmarks/{state['run_id']}.json")
                value = result(state["mode"])
                if saf:
                    del value["zim_sha256"]
                    value.update(source="saf_pread", expected_zim_sha256=bench.DATASETS["small"]["zim"]["sha256"], expected_zim_bytes=14093352,
                                 selection={"revision": saf["receipt"]["selection_revision"], "fingerprint": "b"*64, "kind": "FILES", "part_count": 1},
                                 parts=[{"ordinal": 0, "bytes": 14093352}],
                                 external_manual={"receipt_sha256": saf["sha256"], "verification": "external_manual_not_rechecked",
                                     **{key: saf["receipt"][key] for key in ["observed_logical_zim_sha256", "observed_logical_zim_bytes", "observed_at_utc", "medium", "card_class", "filesystem"]}})
                value.update(run_id=state["run_id"], manifest_sha256=state["hash"], pid=state["pid"], process_started_elapsed_ms=state["pid"]*10)
                if corrupt:
                    value["dataset"] = "wrong"
                output = bench.json_bytes(value)
            else:
                self.fail(f"Unexpected ADB operation {args}")
            return subprocess.CompletedProcess(command, code, output, b"")
        return state, run

    def test_saf_collection_pins_receipt_and_rejects_reselection_before_force_stop(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root/"manifest.json"
            manifest.write_bytes(bench.json_bytes(bench.manifest_for("small")))
            receipt = root/"manual.json"
            receipt.write_bytes(bench.json_bytes(manual_receipt(bench.digest(manifest))))
            saf = bench.validate_saf_receipt(receipt.read_bytes(), bench.manifest_for("small"), bench.digest(manifest))
            state, runner = self.fake_adb(saf=saf)
            report = bench.collect("explicit-serial", manifest, root/"run", "adb-test", runner=runner, saf_receipt=receipt)
            self.assertEqual(report["source"], "saf_pread")
            self.assertEqual(len(report["attempts"]), 61)
            self.assertEqual(report["summary"]["fresh_process"]["suggest_ms"]["count"], 30)
            self.assertEqual((root/"run/manual-receipt.json").read_bytes(), receipt.read_bytes())
            self.assertEqual(sum(c == ["shell", "am", "force-stop", bench.PACKAGE] for c in state["calls"]), 61)
            state, runner = self.fake_adb(saf=saf, stale_selection=True)
            with self.assertRaises(ValueError):
                bench.collect("explicit-serial", manifest, root/"stale", "adb-test", runner=runner, saf_receipt=receipt)
            self.assertFalse(any(c[:3] == ["shell", "am", "force-stop"] for c in state["calls"]))
            self.assertEqual(json.loads((root/"stale/collection.json").read_bytes())["status"], "failed")

    def test_mock_collection_has_30_per_operation_and_fresh_processes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root/"manifest.json"
            manifest.write_bytes(bench.json_bytes(bench.manifest_for("small")))
            state, runner = self.fake_adb()
            report = bench.collect("explicit-serial", manifest, root/"run", "adb-test", runner=runner)
            self.assertEqual(report["status"], "complete")
            self.assertEqual(len(report["attempts"]), 61)
            self.assertEqual(report["summary"]["fresh_process"]["article_ms"]["count"], 30)
            self.assertEqual(report["summary"]["warm"]["suggest_ms"]["count"], 30)
            self.assertEqual(sum(c == ["shell","am","force-stop",bench.PACKAGE] for c in state["calls"]),61)
            self.assertEqual(len({a["pid"] for a in report["attempts"]}),61)

    def test_collection_retries_empty_and_exact_missing_result_despite_zero_adb_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root/"manifest.json"
            manifest.write_bytes(bench.json_bytes(bench.manifest_for("small")))
            state, fake = self.fake_adb()
            polls = {}
            def runner(command, **kwargs):
                reply = fake(command, **kwargs)
                if "exec-out" in command:
                    path = command[-1]
                    polls[path] = polls.get(path, 0) + 1
                    if polls[path] == 1:
                        reply.stdout = b""
                    elif polls[path] == 2:
                        reply.stdout = f"cat: {path}: No such file or directory\r\n".encode()
                return reply
            report = bench.collect("explicit-serial", manifest, root/"run", "adb-test", runner=runner, sleep=lambda _: None)
            self.assertEqual(report["status"], "complete")
            self.assertEqual(len(polls), 61)
            self.assertEqual(set(polls.values()), {3})

    def test_missing_result_retry_stops_at_deadline_and_preserves_adb_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root/"manifest.json"
            manifest.write_bytes(bench.json_bytes(bench.manifest_for("small")))
            _, fake = self.fake_adb()
            def runner(command, **kwargs):
                reply = fake(command, **kwargs)
                if "exec-out" in command:
                    reply.stdout = f"cat: {command[-1]}: No such file or directory\n".encode()
                return reply
            with patch.object(bench.time, "monotonic", side_effect=[0, 1]):
                with self.assertRaises(TimeoutError):
                    bench.collect("explicit-serial", manifest, root/"run", "adb-test", timeout=0, runner=runner)
            self.assertEqual(json.loads((root/"run/collection.json").read_bytes())["status"], "failed")
            self.assertTrue(any(b"No such file" in p.read_bytes() for p in (root/"run").glob("*.stdout.bin")))

    def test_invalid_result_retains_raw_failure_without_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root/"manifest.json"
            manifest.write_bytes(bench.json_bytes(bench.manifest_for("small")))
            _, runner = self.fake_adb(corrupt=True)
            with self.assertRaises(ValueError):
                bench.collect("explicit-serial", manifest, root/"run", "adb-test", runner=runner)
            report = json.loads((root/"run/collection.json").read_bytes())
            self.assertEqual(report["status"], "failed")
            self.assertEqual(len(report["attempts"]),1)
            raw = (root/"run"/(report["attempts"][0]["run_id"]+".json")).read_bytes()
            self.assertEqual(json.loads(raw)["dataset"], "wrong")
            with self.assertRaises(ValueError):
                bench.collect("", manifest, root/"never", runner=runner)
            self.assertFalse((root/"never").exists())

    def test_binary_result_bytes_are_preserved_even_when_not_json(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root/"manifest.json"
            manifest.write_bytes(bench.json_bytes(bench.manifest_for("small")))
            _, fake = self.fake_adb()
            def runner(command, **kwargs):
                reply = fake(command, **kwargs)
                if "exec-out" in command:
                    reply.stdout = b"\xff\x00\r\n"
                return reply
            with self.assertRaises(UnicodeDecodeError):
                bench.collect("explicit-serial", manifest, root/"run", "adb-test", runner=runner)
            report = json.loads((root/"run/collection.json").read_bytes())
            raw = root/"run"/(report["attempts"][0]["run_id"]+".json")
            self.assertEqual(raw.read_bytes(), b"\xff\x00\r\n")

    def test_nonexact_manifest_and_invalid_serial_never_call_adb(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root/"manifest.json"
            canonical = bench.json_bytes(bench.manifest_for("small"))
            manifest.write_bytes(canonical.replace(b"\n", b"\r\n"))
            with self.assertRaises(ValueError):
                bench.collect("explicit-serial", manifest, root/"badmanifest", runner=lambda *a, **k: self.fail("no adb"))
            manifest.write_bytes(canonical)
            with self.assertRaises(ValueError):
                bench.collect("bad serial", manifest, root/"badserial", runner=lambda *a, **k: self.fail("no adb"))
            self.assertFalse((root/"badmanifest").exists())
            self.assertFalse((root/"badserial").exists())


if __name__ == "__main__":
    unittest.main()
