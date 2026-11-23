"""Explicit new-file CFT v2 build using the existing benchmark and memory monitor."""
import argparse
import ctypes
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import phone_benchmark as inputs

spec = importlib.util.spec_from_file_location("existing_zim_benchmark", Path(__file__).with_name("benchmark-zim.py"))
monitor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(monitor)


def verify_pack(root, dataset):
    pack = (root/dataset["pack_path"]).resolve()
    inputs.ordinary(pack)
    before = inputs.identity(pack)
    if int(before["bytes"]) != dataset["zim"]["bytes"]:
        raise ValueError("Source corpus length differs")
    if "receipt_path" in dataset:
        receipt_path = root/dataset["receipt_path"]
        receipt = json.loads(receipt_path.read_bytes())
        if receipt.get("actualSha256") != dataset["zim"]["sha256"] or receipt.get("identity") != before:
            raise ValueError("Existing full SHA receipt does not bind unchanged corpus; no automatic scan")
        proof = {"method":"existing_full_sha_receipt", "receipt":str(receipt_path.resolve()), "receipt_sha256":inputs.digest(receipt_path)}
    else:
        if inputs.digest(pack) != dataset["zim"]["sha256"]:
            raise ValueError("Source corpus SHA mismatch")
        proof = {"method":"streaming_small_corpus_sha"}
    if inputs.identity(pack) != before:
        raise ValueError("Corpus changed during verification")
    return {**proof, "sha256":dataset["zim"]["sha256"], "identity":before}


def decode_identity(header):
    if len(header) < 112 or header[:8] != b"TONTITL\0" or int.from_bytes(header[8:12], "little") != 2 or int.from_bytes(header[12:16], "little") != 112:
        raise ValueError("Builder did not produce a complete v2 header")
    versions = [int.from_bytes(header[i:i+2], "little") for i in range(100,112,2)]
    return {"format":2,"header_bytes":112,"algorithm_revision":int.from_bytes(header[96:100],"little"),
            "normalization_unicode":versions[:3],"lowercase_unicode":versions[3:]}


class MemoryStatus(ctypes.Structure):
    _fields_ = [("length",ctypes.c_ulong),("load",ctypes.c_ulong)] + [(name,ctypes.c_ulonglong) for name in
        ["total_physical","available_physical","total_pagefile","available_pagefile","total_virtual","available_virtual","available_extended_virtual"]]


def memory_status():
    status = MemoryStatus()
    status.length = ctypes.sizeof(status)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
        raise OSError("GlobalMemoryStatusEx failed")
    return {"total_physical_bytes":status.total_physical,"available_physical_bytes":status.available_physical}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dataset", choices=list(inputs.DATASETS), required=True)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New directory; all results and v2 table go here")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Windows lifetime memory measurement required")
    output = args.output.resolve()
    output.mkdir(parents=True,exist_ok=False)
    report = {"status":"preparing","started_utc":datetime.now(timezone.utc).isoformat(),"dataset":args.dataset}
    def save():
        temporary = output/"build.json.tmp"
        temporary.write_bytes(inputs.json_bytes(report))
        temporary.replace(output/"build.json")
    try:
        dataset = inputs.DATASETS[args.dataset]
        proof = verify_pack(inputs.ROOT,dataset)
        report["input"] = proof
        executable = args.executable.resolve(strict=True)
        source_hash = inputs.digest(executable)
        source_identity = inputs.identity(executable)
        copied = output/"zim-title-v2.exe"
        shutil.copyfile(executable,copied)
        if inputs.digest(copied) != source_hash or inputs.identity(executable) != source_identity or inputs.digest(executable) != source_hash:
            raise ValueError("Harness changed during snapshot")
        copied.chmod(0o444)
        report["executable"] = {"source":str(executable),"source_identity":source_identity,"snapshot":str(copied),"sha256":source_hash}
        report["sources"] = {name:inputs.digest(inputs.ROOT/name) for name in ["crates/core/src/title_index.rs","crates/core/benches/zim_title.rs","Cargo.lock","rust-toolchain.toml"]}
        report["compiler"] = subprocess.check_output(["rustc","--version","--verbose"],text=True,creationflags=subprocess.CREATE_NO_WINDOW)
        memory = memory_status()
        required = 5*1024**3 if args.dataset == "full" else 0
        report["memory_preflight"] = {**memory,"minimum_available_bytes":required}
        if memory["available_physical_bytes"] < required:
            raise RuntimeError("Full build requires at least 5 GiB available physical RAM; no process was started")
        table = output/"titles-v2.idx"
        if table.exists():
            raise ValueError("New table path unexpectedly exists")
        environment = os.environ.copy()
        environment.update(TONALLI_BENCH_ZIM=proof["identity"]["path"],TONALLI_BENCH_TITLES=str(table),
                           TONALLI_BENCH_PREFIX=dataset["prefix"],TONALLI_BENCH_ARTICLE=dataset["article_url"],TONALLI_BENCH_MODE="fresh")
        report["status"] = "building"
        save()
        process = monitor.measured_process([str(copied)],environment,output,"build")
        report["process"] = process
        if process["exit_code"]:
            raise RuntimeError("Builder failed; raw stdout/stderr retained")
        text = Path(process["stdout"]).read_text(encoding="utf-8")
        with table.open("rb") as source:
            header = source.read(112)
        fold = decode_identity(header)
        if int.from_bytes(header[56:64],"little") != table.stat().st_size:
            raise ValueError("Table header/file lengths differ")
        report["table"] = {"path":str(table),"bytes":table.stat().st_size,"sha256":inputs.digest(table),
                           "folding_identity":fold,"build_ms":monitor.number(text,r"^title_table_build_ms=([\d.]+)$")}
        if inputs.identity(Path(proof["identity"]["path"])) != proof["identity"]:
            raise ValueError("Corpus identity changed during build")
        if inputs.digest(copied) != source_hash:
            raise ValueError("Frozen executable changed")
        table.chmod(0o444)
        report["peak_private_commit_bytes"] = process["peak_commit_bytes"]
        report["memory_metric"] = "Windows PROCESS_MEMORY_COUNTERS lifetime PeakWorkingSetSize and PeakPagefileUsage (private commit), whole harness process"
        report["verification"] = "Rust builder staged checksum/open before activation; harness reopened and validated nonempty suggest/article; Python checked header/length/SHA and unchanged source identity"
        report["status"] = "complete"
    except Exception as error:
        report.update(status="failed",error=f"{type(error).__name__}: {error}")
        raise
    finally:
        save()


if __name__ == "__main__":
    main()
