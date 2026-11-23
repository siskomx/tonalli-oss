"""Measure an already-built Windows ZIM harness; never downloads or compiles."""
import argparse
import ctypes
from ctypes import wintypes
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib


class MemoryCounters(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("faults", wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in (
            "peak_working_set", "working_set", "peak_paged_pool", "paged_pool",
            "peak_nonpaged_pool", "nonpaged_pool", "pagefile", "peak_pagefile")]


def digest(path):
    hasher = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def measured_process(command, environment, directory, name):
    stdout, stderr = directory / f"{name}.stdout.txt", directory / f"{name}.stderr.txt"
    memory = ctypes.WinDLL("psapi", use_last_error=True).GetProcessMemoryInfo
    memory.argtypes = [wintypes.HANDLE, ctypes.POINTER(MemoryCounters), wintypes.DWORD]
    memory.restype = wintypes.BOOL
    peak, peak_commit = 0, 0
    terminal_counters = False
    start = time.perf_counter()
    with stdout.open("w", encoding="utf-8") as out, stderr.open("w", encoding="utf-8") as err:
        with subprocess.Popen(command, env=environment, stdout=out, stderr=err,
                              creationflags=subprocess.CREATE_NO_WINDOW) as process:
            print(f"{name}: pid={process.pid}", flush=True)
            try:
                while True:
                    counters = MemoryCounters()
                    counters.cb = ctypes.sizeof(counters)
                    # Kernel lifetime peaks, equivalent to Process.PeakWorkingSet64;
                    # current RSS samples would miss a peak between polls.
                    if memory(wintypes.HANDLE(int(process._handle)), ctypes.byref(counters), counters.cb):
                        peak = max(peak, counters.peak_working_set)
                        peak_commit = max(peak_commit, counters.peak_pagefile)
                    if process.poll() is not None:
                        if memory(wintypes.HANDLE(int(process._handle)), ctypes.byref(counters), counters.cb):
                            peak = max(peak, counters.peak_working_set)
                            peak_commit = max(peak_commit, counters.peak_pagefile)
                            terminal_counters = True
                        break
                    time.sleep(0.05)
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait()
            code = process.returncode
    if not peak:
        raise RuntimeError("Windows did not provide process lifetime memory counters")
    return {"command": command, "pid": process.pid, "exit_code": code,
            "observed_wall_ms": (time.perf_counter() - start) * 1000,
            "peak_working_set_bytes": peak, "peak_commit_bytes": peak_commit,
            "memory_counter_status": "terminal_lifetime_counter" if terminal_counters else "lower_bound_last_live_counter",
            "stdout": str(stdout), "stderr": str(stderr)}


def number(output, pattern):
    match = re.search(pattern, output, flags=re.MULTILINE)
    if match is None:
        raise ValueError(f"Missing harness output: {pattern}")
    value = float(match.group(1))
    if not math.isfinite(value) or value < 0:
        raise ValueError("Invalid measurement")
    return value


def percentiles(values):
    ordered = sorted(values)
    return {label: ordered[math.ceil(len(ordered) * q) - 1]
            for label, q in (("p50_ms", 0.5), ("p95_ms", 0.95))}


def self_test():
    assert percentiles(list(range(1, 31))) == {"p50_ms": 15, "p95_ms": 29}
    assert number("suggest_ms sample=0.125\n", r"^suggest_ms sample=([\d.]+)$") == 0.125
    try:
        number("", r"missing=([\d.]+)")
        raise AssertionError("missing measurements accepted")
    except ValueError:
        pass
    with tempfile.TemporaryDirectory() as directory:
        # Release the allocation before the delayed next poll: the lifetime
        # counter must still retain this short-lived memory peak.
        for name, suffix in (("transient", "; time.sleep(.15)"), ("immediate", "")):
            result = measured_process([sys.executable, "-c",
                "import time; x=bytearray(32*1024*1024); del x" + suffix],
                os.environ.copy(), Path(directory), name)
            assert result["exit_code"] == 0
            if name == "transient" or result["memory_counter_status"] == "terminal_lifetime_counter":
                assert result["peak_working_set_bytes"] >= 32 * 1024 * 1024, result
            print(name, result["memory_counter_status"], result["peak_working_set_bytes"])
        failed = measured_process([sys.executable, "-c", "raise SystemExit(7)"],
                                  os.environ.copy(), Path(directory), "failed-child")
        assert failed["exit_code"] == 7
    print("PASS: nearest-rank percentiles, output validation and memory-counter classification")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--executable", type=Path)
    parser.add_argument("--build-mode", choices=("release", "debug"), default="release")
    parser.add_argument("--manifest", type=Path, default=Path("docs/spike/packs.toml"))
    parser.add_argument("--pack-id", default="wikipedia-en-all-nopic")
    parser.add_argument("--prefix", default="earth")
    parser.add_argument("--article", default="C/Earth")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--table", type=Path)
    parser.add_argument("--reuse-table", action="store_true")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Windows is required for lifetime peak-memory measurement")
    if args.self_test:
        self_test()
        return
    if args.executable is None or not args.executable.is_file():
        parser.error("Pass the existing zim_title harness .exe; this driver does not compile it")
    executable = args.executable.resolve()
    if args.build_mode not in executable.parts:
        parser.error("Executable must be under its declared target/release or target/debug directory")
    manifest = args.manifest.resolve()
    packs = tomllib.loads(manifest.read_text(encoding="utf-8"))["packs"]
    pack = next((p for p in packs if p["id"] == args.pack_id), None)
    if pack is None:
        parser.error(f"Unknown manifest pack id: {args.pack_id}")
    corpus = (manifest.parent / pack["path"]).resolve()
    if corpus.suffix != ".zim" or not corpus.is_file() or corpus.stat().st_size != pack["bytes"]:
        parser.error("Final corpus is absent or has the wrong size; .part files are never benchmarked")
    output = (args.output or Path("target/benchmarks") / datetime.now().strftime("%Y%m%d-%H%M%S")).resolve()
    output.mkdir(parents=True, exist_ok=False)
    table = (args.table or output / "titles.idx").resolve()
    report = {"status": "running", "started_utc": datetime.now(timezone.utc).isoformat(),
        "host": {"name": platform.node(), "os": platform.platform(),
                 "processor": platform.processor(), "logical_cpus": os.cpu_count()},
        "dataset": {**pack, "path": str(corpus), "manifest": str(manifest)},
        "binary": {"source_path": str(executable), "build_mode_from_path": args.build_mode,
                   "sha256": digest(executable)},
        "table": str(table), "fresh_process_samples": [],
        "os_cold": {"status": "not_run", "reason": "No cache purge is performed by this driver"}}

    def save():
        temporary = output / "results.json.tmp"
        temporary.write_text(json.dumps(report, indent=2), encoding="utf-8")
        temporary.replace(output / "results.json")

    try:
        save()
        start = time.perf_counter()
        actual = digest(corpus)
        if actual.lower() != pack["sha256"].lower():
            raise ValueError("Corpus SHA-256 does not match the manifest")
        report["verification"] = {"sha256": actual, "seconds": time.perf_counter() - start,
            "reference": str(manifest), "method": "full-file SHA-256 immediately before run"}
        if table.exists() and not args.reuse_table:
            raise ValueError("Table exists; close its readers and choose a new path or pass --reuse-table")
        if args.reuse_table and not table.is_file():
            raise ValueError("--reuse-table requires an existing table")
        table.parent.mkdir(parents=True, exist_ok=True)
        runner = output / "zim-benchmark.exe"
        shutil.copy2(executable, runner)
        if digest(runner) != report["binary"]["sha256"]:
            raise ValueError("Copied executable differs from its recorded source digest; rebuild/copy raced")
        report["binary"]["run_path"] = str(runner)
        environment = os.environ.copy()
        environment.update({"TONALLI_BENCH_ZIM": str(corpus), "TONALLI_BENCH_TITLES": str(table),
            "TONALLI_BENCH_PREFIX": args.prefix, "TONALLI_BENCH_ARTICLE": args.article})

        def run(name, mode):
            environment["TONALLI_BENCH_MODE"] = mode
            result = measured_process([str(runner)], environment, output, name)
            report["last_process"] = result
            save()
            if result["exit_code"]:
                raise RuntimeError(f"Harness failed; see {result['stderr']}")
            text = Path(result["stdout"]).read_text(encoding="utf-8")
            if name != "build" and "title_table_build_ms=" in text:
                raise ValueError("Table disappeared or was rebuilt during a query-only stage")
            return result, text

        if not args.reuse_table:
            result, text = run("build", "fresh")
            report["build"] = {**result, "title_table_build_ms": number(text, r"^title_table_build_ms=([\d.]+)$"),
                               "table_bytes": table.stat().st_size}
        else:
            report["build"] = {"performed": False, "reason": "existing table reused", "table_bytes": table.stat().st_size}
        result, text = run("warm", "warm")
        if "condition=warm process runs=30" not in text:
            raise ValueError("Harness did not perform 30 warm runs")
        report["warm"] = {**result, **{operation: {
            "p50_ms": number(text, rf"^{operation}_ms p50=([\d.]+) p95="),
            "p95_ms": number(text, rf"^{operation}_ms p50=[\d.]+ p95=([\d.]+)$")}
            for operation in ("suggest", "article")}}
        for i in range(30):
            result, text = run(f"fresh-{i+1:02}", "fresh")
            if "fresh-process; OS cache not flushed by harness runs=1" not in text:
                raise ValueError("Harness did not perform one fresh-process sample")
            report["fresh_process_samples"].append({**result, **{
                f"{operation}_ms": number(text, rf"^{operation}_ms sample=([\d.]+)$")
                for operation in ("suggest", "article")}})
            save()
        report["fresh_process"] = {operation: percentiles([s[f"{operation}_ms"] for s in report["fresh_process_samples"]])
                                   for operation in ("suggest", "article")}
        report["status"] = "complete"
        save()
        print(output / "results.json")
    except BaseException as error:
        report.update(status="interrupted" if isinstance(error, KeyboardInterrupt) else "failed", error=str(error))
        save()
        raise


if __name__ == "__main__":
    main()
