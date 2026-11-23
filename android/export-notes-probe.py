"""Read-only binary-safe export/check of a force-stopped debuggable M1 vault."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

MARKER = b"TONALLI_M1_NOTES_SENTINEL_20261111"
NAMES = ("notes.db", "notes.keys")


class CheckFailed(Exception):
    pass


def inspect(path):
    if path.is_symlink() or not path.is_file() or path.stat().st_size < 16:
        raise CheckFailed("Missing, short or nonregular exported vault file")
    digest, previous, prefix, size = hashlib.sha256(), b"", b"", 0
    with path.open("rb") as source:
        while block := source.read(65536):
            if not prefix:
                prefix = block[:16]
            if MARKER in previous + block:
                raise CheckFailed("Plaintext synthetic marker found in exported vault")
            previous = block[-(len(MARKER) - 1):]
            digest.update(block)
            size += len(block)
    if prefix == b"SQLite format 3\0":
        raise CheckFailed("Plaintext SQLite header found in exported vault")
    return {"bytes": size, "sha256": digest.hexdigest(), "marker_absent": True, "sqlite_header_absent": True}


def device_hashes(command):
    output = subprocess.check_output(command + ["sha256sum"] + [f"files/core/{name}" for name in NAMES],
                                     stderr=subprocess.PIPE, text=True, encoding="ascii")
    hashes = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) != 2 or not re.fullmatch(r"[a-fA-F0-9]{64}", fields[0]) or fields[1] not in {f"files/core/{name}" for name in NAMES}:
            raise CheckFailed("Unexpected device fingerprint response")
        name = fields[1].rsplit("/", 1)[1]
        if name in hashes:
            raise CheckFailed("Duplicate device fingerprint response")
        hashes[name] = fields[0].lower()
    if set(hashes) != set(NAMES):
        raise CheckFailed("Incomplete device fingerprint response")
    return hashes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True, help="Explicit authorized device serial")
    parser.add_argument("destination", type=Path, help="New output directory; never overwritten")
    args = parser.parse_args()
    adb = Path(os.environ["LOCALAPPDATA"]) / "Android/Sdk/platform-tools/adb.exe"
    command = [str(adb), "-s", args.serial, "exec-out", "run-as", "org.tonalli.spike"]
    before = device_hashes(command)
    args.destination.mkdir(parents=True, exist_ok=False)
    for name in NAMES:
        with (args.destination / name).open("xb") as target:
            subprocess.run(command + ["cat", f"files/core/{name}"], stdout=target, stderr=subprocess.PIPE, check=True)
    after = device_hashes(command)
    if before != after:
        raise CheckFailed("Device vault changed during export; close all vault users and retry into a new directory")
    report = {name: inspect(args.destination / name) for name in NAMES}
    if any(report[name]["sha256"] != before[name] for name in NAMES):
        raise CheckFailed("Exported bytes do not match device fingerprints")
    print(json.dumps({"device_hashes_unchanged": True, "exports": report}, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (CheckFailed, OSError, subprocess.SubprocessError) as error:
        detail = str(error) if isinstance(error, CheckFailed) else type(error).__name__
        print(f"FAILED: {detail}; any exported files were retained", file=sys.stderr)
        sys.exit(1)
