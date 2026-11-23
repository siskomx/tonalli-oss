"""Export existing benchmark inputs or collect explicit-serial Android results.

Importing this module never invokes adb. Export never downloads or builds.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import uuid
from adb_receipts import cat_pending

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "org.tonalli.spike"
ACTION = PACKAGE + ".BENCHMARK"
DATASETS = json.loads((ROOT/"docs/spike/phone-benchmark-inputs/catalog.json").read_bytes())


def json_bytes(value):
    return (json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n").encode("utf-8")


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as source:
        for block in iter(lambda: source.read(1024*1024), b""):
            h.update(block)
    return h.hexdigest()


def identity(path):
    path = Path(path).resolve(strict=True)
    info = path.stat()
    # Node's Windows dev is the legacy 32-bit volume serial; Python uses 64 bits.
    dev = info.st_dev & 0xffffffff if os.name == "nt" else info.st_dev
    return {"path": str(path), "bytes": str(info.st_size), "mtimeNs": str(info.st_mtime_ns),
            "birthtimeNs": str(getattr(info, "st_birthtime_ns", info.st_ctime_ns)),
            "dev": str(dev), "ino": str(info.st_ino)}


def manifest_for(dataset, specifications=DATASETS):
    spec = specifications[dataset]
    return {"version": 1, "id": dataset, "acceptance_eligible": spec.get("acceptance_eligible", False),
            **{k: spec[k] for k in ["name", "zim", "title_index", "prefix", "article_url"]}}


def ordinary(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"Input is not an ordinary file: {path}")


def verify_inputs(root, spec):
    pack, table = root/spec["pack_path"], root/spec["table_path"]
    for path in [pack, table]:
        ordinary(path)
    before = {"zim": identity(pack), "title_index": identity(table)}
    for key in before:
        if int(before[key]["bytes"]) != spec[key]["bytes"]:
            raise ValueError(f"Wrong {key} length")
    receipt = None
    if "receipt_path" in spec:
        receipt_path = root/spec["receipt_path"]
        receipt = json.loads(receipt_path.read_bytes())
        if receipt.get("actualSha256") != spec["zim"]["sha256"] or receipt.get("identity") != before["zim"]:
            raise ValueError("Existing full-corpus receipt does not bind this file; refusing an automatic rescan")
        receipt = {"path": str(receipt_path.resolve()), "sha256": digest(receipt_path),
                   "verification": "existing full streaming SHA receipt bound to unchanged file identity; no new pack scan"}
    elif digest(pack) != spec["zim"]["sha256"]:
        raise ValueError("ZIM SHA-256 mismatch")
    if digest(table) != spec["title_index"]["sha256"]:
        raise ValueError("Title table SHA-256 mismatch")
    if before != {"zim": identity(pack), "title_index": identity(table)}:
        raise ValueError("Inputs changed during verification")
    return pack, table, before, receipt


def export_inputs(root, output, datasets, specifications=DATASETS):
    """Publish small verified copies; full payloads remain references, never copied."""
    root, output = Path(root).resolve(), Path(output).resolve()
    if any(not re.fullmatch(r"[a-z][a-z0-9_-]{0,31}", dataset) for dataset in datasets):
        raise ValueError("Invalid trusted dataset identifier")
    output.mkdir(parents=True, exist_ok=False)
    summary = {"version": 1, "status": "preparing", "datasets": []}
    (output/"export.json").write_bytes(json_bytes(summary))
    try:
        for dataset in datasets:
            spec = specifications[dataset]
            pack, table, before, receipt = verify_inputs(root, spec)
            staged = Path(tempfile.mkdtemp(prefix=".stage-", dir=output))
            try:
                manifest = json_bytes(manifest_for(dataset, specifications))
                (staged/"manifest.json").write_bytes(manifest)
                files = []
                for name, source, key in [("pack.zim", pack, "zim"), ("titles.bin", table, "title_index")]:
                    if "receipt_path" not in spec:
                        copied = staged/name
                        shutil.copyfile(source, copied)
                        if copied.stat().st_size != spec[key]["bytes"] or digest(copied) != spec[key]["sha256"]:
                            raise ValueError("Staged copy checksum mismatch")
                        push_source = str((output/dataset/name).resolve())
                    else:
                        push_source = str(source.resolve())
                    files.append({"source": push_source, "remote": f"/sdcard/Android/data/{PACKAGE}/files/benchmark/{dataset}/{name}", **spec[key]})
                files.insert(0, {"source": str(output/dataset/"manifest.json"), "remote": f"/sdcard/Android/data/{PACKAGE}/files/benchmark/{dataset}/manifest.json",
                                 "bytes": len(manifest), "sha256": hashlib.sha256(manifest).hexdigest()})
                if before != {"zim": identity(pack), "title_index": identity(table)}:
                    raise ValueError("Source changed during export")
                row = {"dataset": dataset, "manifest_sha256": hashlib.sha256(manifest).hexdigest(), "files": files,
                       "acceptance_eligible": spec.get("acceptance_eligible", False),
                       "host_identity": before, "pack_verification_receipt": receipt,
                       "payload_mode": "reference_only" if receipt else "verified_small_copies",
                       "external_payload_bytes": spec["zim"]["bytes"] + spec["title_index"]["bytes"],
                       "internal_payload_bytes": spec["zim"]["bytes"] + spec["title_index"]["bytes"]}
                (staged/"push-files.json").write_bytes(json_bytes(row))
                staged.rename(output/dataset)
                for file in (output/dataset).iterdir():
                    file.chmod(0o444)
                summary["datasets"].append(row)
            finally:
                if staged.exists():
                    if staged.resolve().parent != output:
                        raise ValueError("Unsafe stage cleanup path")
                    shutil.rmtree(staged)
        summary["status"] = "complete"
    except Exception as error:
        summary.update(status="failed", error=str(error))
        raise
    finally:
        (output/"export.json").write_bytes(json_bytes(summary))
    return summary


def number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value >= 0


def integer(value, minimum=0):
    return isinstance(value, int) and not isinstance(value, bool) and value >= minimum


def validate_saf_receipt(raw, manifest, manifest_hash):
    if len(raw) > 16384:
        raise ValueError("Manual receipt exceeds 16 KiB")
    receipt = json.loads(raw)
    keys = {"version", "dataset_id", "manifest_sha256", "selection_revision", "selection_fingerprint",
            "observed_logical_zim_sha256", "observed_logical_zim_bytes", "observed_at_utc",
            "medium", "card_class", "filesystem", "evidence"}
    if not isinstance(receipt, dict) or set(receipt) != keys:
        raise ValueError("Manual receipt fields differ; never include private URIs")
    if (type(receipt["version"]) is not int or receipt["version"] != 1
            or receipt["dataset_id"] != manifest["id"] or receipt["manifest_sha256"] != manifest_hash
            or receipt["observed_logical_zim_sha256"] != manifest["zim"]["sha256"]
            or not integer(receipt["observed_logical_zim_bytes"], 1)
            or receipt["observed_logical_zim_bytes"] != manifest["zim"]["bytes"]
            or receipt["evidence"] != "external_manual_not_rechecked"):
        raise ValueError("Manual statement differs from expected dataset or evidence boundary")
    if (not isinstance(receipt["selection_revision"], str)
            or not re.fullmatch(r"[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}", receipt["selection_revision"])
            or not isinstance(receipt["selection_fingerprint"], str)
            or not re.fullmatch(r"[a-f0-9]{64}", receipt["selection_fingerprint"])):
        raise ValueError("Invalid selection revision/fingerprint")
    for field in ["observed_at_utc", "medium", "card_class", "filesystem"]:
        value = receipt[field]
        if not isinstance(value, str) or not value or len(value.encode("utf-8")) > 128 or any(ord(c) < 32 or ord(c) == 127 for c in value):
            raise ValueError("Invalid manual observation text")
    try:
        observed = datetime.fromisoformat(receipt["observed_at_utc"].replace("Z", "+00:00"))
        if observed.utcoffset() is None or observed.utcoffset().total_seconds() != 0:
            raise ValueError("UTC required")
    except (ValueError, TypeError) as error:
        raise ValueError("Manual observation needs an explicit UTC time") from error
    return {"receipt": receipt, "sha256": hashlib.sha256(raw).hexdigest()}


def validate_saf_selection(selection, saf):
    receipt = saf["receipt"]
    if (not isinstance(selection, dict) or set(selection) != {"revision", "fingerprint", "kind", "part_count"}
            or selection["revision"] != receipt["selection_revision"]
            or selection["fingerprint"] != receipt["selection_fingerprint"]
            or selection["kind"] not in {"FILES", "TREE"}
            or not integer(selection["part_count"], 1) or selection["part_count"] > 128):
        raise ValueError("SAF selection changed; explicitly rebind a new receipt")


def validate_result(result, manifest, manifest_hash, run_id, mode, live_pid, saf=None):
    if mode not in {"warm", "fresh-suggest", "fresh-article"}:
        raise ValueError("Unknown benchmark mode")
    expected = {"version": 1, "run_id": run_id, "dataset": manifest["id"], "manifest_sha256": manifest_hash,
                "mode": mode, "status": "complete", "source": "internal_mmap", "cache_state": "uncontrolled_os_cache",
                "acceptance_eligible": manifest["acceptance_eligible"],
                "zim_sha256": manifest["zim"]["sha256"], "title_index_sha256": manifest["title_index"]["sha256"],
                "prefix": manifest["prefix"], "article_url": manifest["article_url"], "percentile_method": "nearest_rank"}
    if saf is not None:
        expected.pop("zim_sha256")
        expected.update(source="saf_pread", expected_zim_sha256=manifest["zim"]["sha256"], expected_zim_bytes=manifest["zim"]["bytes"])
    if not isinstance(result, dict) or any(result.get(k) != v for k, v in expected.items()):
        raise ValueError("Result status, dataset, identity, operation or cache label differs")
    if saf is not None:
        if "zim_sha256" in result:
            raise ValueError("SAF expected hash must not masquerade as verified current bytes")
        validate_saf_selection(result.get("selection"), saf)
        manual = {key: saf["receipt"][key] for key in ["observed_logical_zim_sha256", "observed_logical_zim_bytes", "observed_at_utc", "medium", "card_class", "filesystem"]}
        manual.update(receipt_sha256=saf["sha256"], verification="external_manual_not_rechecked")
        if result.get("external_manual") != manual:
            raise ValueError("Manual observation evidence differs")
        parts = result.get("parts")
        if not isinstance(parts, list) or len(parts) != result["selection"]["part_count"]:
            raise ValueError("Missing SAF part observations")
        for ordinal, part in enumerate(parts):
            if (not isinstance(part, dict) or set(part) != {"ordinal", "bytes"}
                    or type(part["ordinal"]) is not int or part["ordinal"] != ordinal
                    or (part["bytes"] is not None and not integer(part["bytes"]))):
                raise ValueError("Invalid SAF part observation")
        if all(part["bytes"] is not None for part in parts) and sum(part["bytes"] for part in parts) != manifest["zim"]["bytes"]:
            raise ValueError("Observed SAF lengths differ from expected logical size")
    if any(not isinstance(result.get(k), dict) for k in ["ram", "setup_ms", "priming", "samples"]):
        raise ValueError("Missing benchmark result objects")
    if type(result.get("acceptance_eligible")) is not bool or not integer(result.get("version"), 1) or not integer(result.get("pid"), 1) or result["pid"] != live_pid or not integer(result.get("process_started_elapsed_ms")):
        raise ValueError("Result does not bind the live app process")
    if not integer(result.get("api_level"), 1) or any(not integer(result.get("ram", {}).get(k)) for k in ["advertised_bytes", "total_bytes", "available_bytes"]):
        raise ValueError("Missing actual device metadata")
    if any(not number(result.get("setup_ms", {}).get(k)) for k in ["native_init", "open", "attach", "total"]):
        raise ValueError("Invalid setup durations")
    priming = {"suggest": 1 if mode == "warm" else 0, "article": 1 if mode == "warm" else 0}
    if result.get("priming") != priming or any(not integer(v) for v in result["priming"].values()):
        raise ValueError("Unexpected benchmark priming")
    counts = (30, 30) if mode == "warm" else ((1, 0) if mode == "fresh-suggest" else (0, 1))
    for key, count in zip(["suggest_ms", "article_ms"], counts):
        values = result.get("samples", {}).get(key)
        if not isinstance(values, list) or len(values) != count or not all(map(number, values)):
            raise ValueError("Wrong sample count or invalid duration")


def percentiles(values):
    if not values or not all(map(number, values)):
        raise ValueError("Need finite nonnegative samples")
    values = sorted(values)
    return {"count": len(values), "p50_ms": values[math.ceil(len(values)*.5)-1], "p95_ms": values[math.ceil(len(values)*.95)-1]}


class Adb:
    def __init__(self, executable, serial, output, runner=subprocess.run):
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:\-]{0,127}", serial):
            raise ValueError("An explicit valid adb serial is required")
        self.executable, self.serial, self.output, self.runner, self.sequence = executable, serial, output, runner, 0

    def call(self, *arguments):
        self.sequence += 1
        command = [self.executable, "-s", self.serial, *arguments]
        prefix = self.output/f"command-{self.sequence:05}"
        (prefix.with_suffix(".json")).write_bytes(json_bytes({"arguments": command}))
        try:
            result = self.runner(command, capture_output=True, timeout=30,
                                 creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        except subprocess.TimeoutExpired as error:
            prefix.with_suffix(".stdout.bin").write_bytes(error.stdout or b"")
            prefix.with_suffix(".stderr.bin").write_bytes(error.stderr or b"")
            raise RuntimeError("ADB command timed out; raw evidence retained") from error
        prefix.with_suffix(".stdout.bin").write_bytes(result.stdout)
        prefix.with_suffix(".stderr.bin").write_bytes(result.stderr)
        prefix.with_suffix(".exit.json").write_bytes(json_bytes({"returncode": result.returncode}))
        return result

    def pid(self):
        result = self.call("shell", "pidof", PACKAGE)
        raw = result.stdout.strip()
        if result.returncode in [0, 1] and not raw:
            return None
        if result.returncode != 0 or not re.fullmatch(rb"[1-9][0-9]*", raw):
            raise ValueError("Cannot identify the single live app process")
        return int(raw)


def collect(serial, manifest_path, output, executable="adb", timeout=120, runner=subprocess.run, sleep=time.sleep, saf_receipt=None):
    if not serial:
        raise ValueError("Explicit serial required")
    manifest_raw = Path(manifest_path).read_bytes()
    manifest = json.loads(manifest_raw)
    if not isinstance(manifest, dict) or manifest.get("id") not in DATASETS or manifest != manifest_for(manifest["id"]) or type(manifest.get("version")) is not int or manifest_raw != json_bytes(manifest_for(manifest["id"])):
        raise ValueError("Manifest differs from pinned benchmark inputs")
    manifest_hash = hashlib.sha256(manifest_raw).hexdigest()
    saf_raw = None
    if saf_receipt is not None:
        with Path(saf_receipt).open("rb") as receipt_file:
            saf_raw = receipt_file.read(16385)
    saf = validate_saf_receipt(saf_raw, manifest, manifest_hash) if saf_raw is not None else None
    output = Path(output).resolve()
    adb = Adb(executable, serial, output, runner)
    output.mkdir(parents=True, exist_ok=False)
    base = "pb_" + uuid.uuid4().hex
    report = {"version": 1, "status": "collecting", "serial": serial, "collection_id": base,
              "dataset": manifest, "manifest_sha256": manifest_hash, "started_utc": datetime.now(timezone.utc).isoformat(),
              "acceptance_eligible": manifest["acceptance_eligible"],
              "source": "saf_pread" if saf else "internal_mmap",
              "measurement_role": "representative_dataset" if manifest["acceptance_eligible"] else "functional_smoke_only",
              "cache": "fresh Android app processes; OS caches untouched; table checksum scan occurs in setup.attach", "attempts": []}
    (output/"manifest.json").write_bytes(manifest_raw)
    if saf:
        (output/"manual-receipt.json").write_bytes(saf_raw)
        report["source_verification"] = "external_manual_not_rechecked; no current Android-byte SHA verification"
        report["manual_receipt_sha256"] = saf["sha256"]
    def save():
        (output/"collection.json").write_bytes(json_bytes(report))
    save()
    seen_processes = set()
    try:
        state = adb.call("get-state")
        if state.returncode or state.stdout.strip() != b"device":
            raise ValueError("Selected adb device is not ready")
        if saf:
            selected = adb.call("exec-out", "run-as", PACKAGE, "cat", "files/benchmarks/saf-selection.json")
            if selected.returncode or len(selected.stdout) > 4096:
                raise ValueError("Prepare the current SAF selection and bind its manual receipt first")
            selection = json.loads(selected.stdout)
            if not isinstance(selection, dict) or type(selection.get("version")) is not int or selection["version"] != 1:
                raise ValueError("Unsupported SAF selection summary")
            selection = {key: value for key, value in selection.items() if key != "version"}
            validate_saf_selection(selection, saf)
            saf["selection"] = selection
            report["selection"] = selection
        for mode, count in [("warm", 1), ("fresh-suggest", 30), ("fresh-article", 30)]:
            for sample in range(1, count+1):
                run_id = f"{base}_{mode.replace('-', '_')}_{sample:02}"
                attempt = {"run_id": run_id, "mode": mode, "status": "started"}
                report["attempts"].append(attempt)
                save()
                stopped = adb.call("shell", "am", "force-stop", PACKAGE)
                if stopped.returncode or adb.pid() is not None:
                    raise ValueError("Our app did not stop; refusing a mislabeled fresh process")
                launch = ["shell", "am", "start", "-W", "-n", PACKAGE+"/.MainActivity", "-a", PACKAGE+".SAF_BENCHMARK" if saf else ACTION,
                          "--es", "dataset_id", manifest["id"], "--es", "manifest_sha256", manifest_hash,
                          "--es", "run_id", run_id, "--es", "mode", mode,
                          "--ei", "sample_count", "30" if mode == "warm" else "1"]
                if saf:
                    for key, value in {"selection_revision": saf["receipt"]["selection_revision"],
                                       "selection_fingerprint": saf["receipt"]["selection_fingerprint"],
                                       "manual_receipt_sha256": saf["sha256"]}.items():
                        launch += ["--es", key, value]
                started = adb.call(*launch)
                if started.returncode or not re.search(rb"Status:\s*ok\b", started.stdout) or b"Error:" in started.stdout:
                    raise ValueError("Benchmark Activity launch failed")
                deadline = time.monotonic() + timeout
                result_path = f"files/benchmarks/{run_id}.json"
                while True:
                    exported = adb.call("exec-out", "run-as", PACKAGE, "cat", result_path)
                    if exported.returncode == 0 and not cat_pending(exported.stdout, result_path):
                        break
                    if time.monotonic() >= deadline:
                        raise TimeoutError("No published benchmark result; raw ADB failures retained")
                    sleep(.25)
                (output/f"{run_id}.json").write_bytes(exported.stdout)
                result = json.loads(exported.stdout.decode("utf-8"))
                validate_result(result, manifest, manifest_hash, run_id, mode, adb.pid(), saf=saf)
                if saf:
                    if result["selection"] != saf["selection"] or ("parts" in report and result["parts"] != report["parts"]):
                        raise ValueError("SAF selection or observable part metadata changed during collection")
                    report["parts"] = result["parts"]
                process = (result["pid"], result["process_started_elapsed_ms"])
                if process in seen_processes:
                    raise ValueError("A previous app process was reused")
                seen_processes.add(process)
                attempt.update(status="complete", pid=result["pid"], result=f"{run_id}.json", samples=result["samples"])
                save()
        report["summary"] = {}
        for condition, modes in [("warm", ["warm"]), ("fresh_process", ["fresh-suggest", "fresh-article"])]:
            report["summary"][condition] = {key: percentiles([value for a in report["attempts"] if a["mode"] in modes for value in a["samples"][key]]) for key in ["suggest_ms", "article_ms"]}
        report["status"] = "complete"
    except Exception as error:
        report.update(status="failed", error=f"{type(error).__name__}: {error}")
        if report["attempts"] and report["attempts"][-1]["status"] != "complete":
            report["attempts"][-1]["status"] = "failed"
        raise
    finally:
        save()
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    export = commands.add_parser("export")
    export.add_argument("--dataset", choices=[*DATASETS, "all"], required=True)
    export.add_argument("--output", type=Path, required=True, help="New output directory; never overwrite")
    run = commands.add_parser("collect")
    run.add_argument("--serial", required=True)
    run.add_argument("--manifest", type=Path, required=True)
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--adb", default="adb")
    run.add_argument("--timeout", type=float, default=120)
    run.add_argument("--saf-receipt", type=Path, help="Bound external/manual source receipt; dataset must already be prepared and bound in the app")
    args = parser.parse_args()
    if args.command == "export":
        export_inputs(ROOT, args.output, list(DATASETS) if args.dataset == "all" else [args.dataset])
    else:
        if not math.isfinite(args.timeout) or args.timeout <= 0:
            parser.error("--timeout must be finite and positive")
        collect(args.serial, args.manifest, args.output, args.adb, args.timeout, saf_receipt=args.saf_receipt)


if __name__ == "__main__":
    main()
