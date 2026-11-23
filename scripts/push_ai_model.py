"""Deliver one pinned GGUF into a debug app's request-bound incoming directory."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import time
import tomllib
import uuid

from adb_receipts import cat_pending

PACKAGE = "org.tonalli.spike"


def identity(path):
    info = path.stat()
    return dict(bytes=info.st_size, mtime_ns=info.st_mtime_ns, ino=info.st_ino, dev=info.st_dev)


def verified_model(manifest_path, model_id):
    manifest_path = Path(manifest_path).resolve(strict=True)
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    if type(manifest.get("manifest_version")) is not int or manifest["manifest_version"] != 1:
        raise ValueError("Unsupported pack manifest")
    packs = manifest.get("packs", [])
    if not isinstance(packs, list) or any(not isinstance(pack, dict) for pack in packs):
        raise ValueError("Invalid model records in pack manifest")
    matches = [pack for pack in packs if pack.get("id") == model_id]
    if len(matches) != 1:
        raise ValueError("Select exactly one known model ID")
    model = matches[0]
    if (not isinstance(model_id, str) or not re.fullmatch(r"[a-z0-9][a-z0-9.-]{0,79}", model_id)
            or model.get("kind") != "gguf" or model.get("role") not in ("generation", "embedding")
            or type(model.get("bytes")) is not int or not 0 < model["bytes"] <= 2**63 - 1
            or not isinstance(model.get("sha256"), str) or not re.fullmatch(r"[a-f0-9]{64}", model["sha256"])):
        raise ValueError("Invalid trusted model metadata")
    relative = model.get("path")
    if (not isinstance(relative, str) or "\\" in relative or ":" in relative or "\0" in relative
            or relative.split("/")[0] != "packs" or len(relative.split("/")) < 2
            or any(part in ("", ".", "..") for part in relative.split("/"))):
        raise ValueError("Model destination must stay under the manifest's packs directory")
    source = manifest_path.parent
    for part in relative.split("/"):
        source = source / part
        if source.is_symlink():
            raise ValueError("Linked model inputs are refused")
    source = source.resolve(strict=True)
    if not source.is_relative_to(manifest_path.parent) or not source.is_file():
        raise ValueError("Model input escapes its manifest root or is not a regular file")
    before = identity(source)
    if before["bytes"] != model["bytes"]:
        raise ValueError("Model input length differs from its trusted pin")
    digest = hashlib.sha256()
    remaining = model["bytes"]
    with source.open("rb") as stream:
        while remaining:
            chunk = stream.read(min(64 * 1024, remaining))
            if not chunk:
                raise ValueError("Model input ended during verification")
            digest.update(chunk)
            remaining -= len(chunk)
        if stream.read(1):
            raise ValueError("Model input grew during verification")
    if identity(source) != before or digest.hexdigest() != model["sha256"]:
        raise ValueError("Model input changed or checksum differs from its trusted pin")
    return model, source, before


def prepare_and_push(model_id, manifest_path, serial, adb="adb", *, runner=subprocess.run,
                     timeout=60, push_timeout=1800, sleep=time.sleep):
    if (not isinstance(serial, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:\-]{0,127}", serial)
            or not math.isfinite(timeout) or timeout <= 0
            or not math.isfinite(push_timeout) or push_timeout <= 0):
        raise ValueError("Explicit serial and finite positive timeouts required")
    model, source, before = verified_model(manifest_path, model_id)
    request = str(uuid.uuid4())

    def call(*arguments, duration=30):
        return runner([adb, "-s", serial, *arguments], capture_output=True, timeout=duration,
                      creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)

    state = call("get-state")
    if state.returncode or state.stdout.strip() != b"device":
        raise ValueError("Selected adb device is not ready")
    started = call("shell", "am", "start", "-W", "-n", PACKAGE + "/.MainActivity",
                   "-a", PACKAGE + ".PREPARE_AI_INPUT", "--es", "request_id", request,
                   "--es", "model_id", model_id)
    if started.returncode or not re.search(rb"Status:\s*ok\b", started.stdout) or b"Error:" in started.stdout:
        raise ValueError("App model-input preparation did not start")
    receipt_path = f"files/ai-inputs-ready/{request}.json"
    deadline = time.monotonic() + timeout
    while True:
        reply = call("exec-out", "run-as", PACKAGE, "cat", receipt_path)
        if len(reply.stdout) > 4096:
            raise ValueError("Oversized app model-input receipt")
        if reply.returncode == 0 and not cat_pending(reply.stdout, receipt_path):
            break
        if time.monotonic() >= deadline:
            raise TimeoutError("App preparation receipt missing; no model was pushed")
        sleep(.25)
    ready = json.loads(reply.stdout.decode("utf-8"))
    if (not isinstance(ready, dict) or type(ready.get("version")) is not int or ready["version"] != 1
            or ready.get("status") != "ready" or ready.get("request_id") != request
            or ready.get("model_id") != model_id or ready.get("sha256") != model["sha256"]
            or type(ready.get("bytes")) is not int or ready["bytes"] != model["bytes"]
            or ready.get("role") != model["role"] or ready.get("incoming_name") != f"ai-input-{request}"
            or ready.get("file_name") != "model.gguf"
            or type(ready.get("incoming_additional_bytes")) is not int
            or ready["incoming_additional_bytes"] < model["bytes"]
            or type(ready.get("available_bytes")) is not int
            or ready["available_bytes"] < ready["incoming_additional_bytes"]):
        raise ValueError("App model-input identity/space refusal; no model was pushed or permissions changed")
    if identity(source) != before:
        raise ValueError("Verified model changed before delivery; no payload pushed")
    target = f"/sdcard/Android/data/{PACKAGE}/files/ai-input-{request}/model.gguf"
    pushed = call("push", str(source), target, duration=push_timeout)
    if pushed.returncode:
        raise RuntimeError("Model push failed; incoming preserved, installed models untouched")
    if identity(source) != before:
        raise ValueError("Model changed during delivery; preserve incoming and do not import it")
    return dict(status="transferred", installed=False, serial=serial, model_id=model_id,
                request_id=request, incoming_name=f"ai-input-{request}", pushed_files=1,
                expected_bytes=model["bytes"], sha256=model["sha256"], source_identity=before,
                note="Host checksum and delivery only; app import must verify staged bytes before Core load")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--id", required=True)
    parser.add_argument("--manifest", type=Path, default=Path("docs/spike/packs.toml"))
    parser.add_argument("--serial", required=True)
    parser.add_argument("--adb", default="adb")
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--push-timeout", type=float, default=1800)
    parser.add_argument("--evidence", type=Path, required=True, help="New output directory")
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=False)
    try:
        result = prepare_and_push(args.id, args.manifest, args.serial, args.adb,
                                  timeout=args.timeout, push_timeout=args.push_timeout)
    except Exception as error:
        (args.evidence / "result.json").write_text(json.dumps(dict(status="failed", installed=False,
            error_type=type(error).__name__), indent=2), encoding="utf-8")
        raise
    (args.evidence / "result.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
