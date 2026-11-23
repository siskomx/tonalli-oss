"""Deliver a trusted small index through app-created directories, never recursive adb push."""
import argparse
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import subprocess
import time
import uuid
from adb_receipts import cat_pending

spec = importlib.util.spec_from_file_location("index_exporter",Path(__file__).with_name("export-index-probe.py"))
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)
PACKAGE = "org.tonalli.spike"


def prepare_and_push(bundle,trusted_manifest,serial,adb="adb",runner=subprocess.run,timeout=60,sleep=time.sleep):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:\-]{0,127}",serial) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("Explicit serial and positive timeout required")
    manifest,digest,files = exporter.validated_bundle(bundle,trusted_manifest)
    request = str(uuid.uuid4())
    incoming = "probe-index-"+digest[:12]
    def call(*arguments):
        return runner([adb,"-s",serial,*arguments],capture_output=True,timeout=30,
                      creationflags=subprocess.CREATE_NO_WINDOW if os.name=="nt" else 0)
    state = call("get-state")
    if state.returncode or state.stdout.strip()!=b"device":
        raise ValueError("Selected adb device is not ready")
    started = call("shell","am","start","-W","-n",PACKAGE+"/.MainActivity","-a",PACKAGE+".PREPARE_PROBE_INPUTS",
                   "--es","request_id",request,"--es","manifest_sha256",digest)
    if started.returncode or not re.search(rb"Status:\s*ok\b",started.stdout) or b"Error:" in started.stdout:
        raise ValueError("Probe-input preparation action could not start")
    deadline = time.monotonic()+timeout
    receipt_path = f"files/probe-inputs-ready/{request}.json"
    while True:
        reply = call("exec-out","run-as",PACKAGE,"cat",receipt_path)
        if reply.returncode==0 and not cat_pending(reply.stdout,receipt_path):
            break
        if time.monotonic() >= deadline:
            raise TimeoutError("App preparation receipt missing; no payload was pushed")
        sleep(.25)
    if len(reply.stdout)>64*1024:
        raise ValueError("Oversized app preparation receipt")
    ready = json.loads(reply.stdout.decode("utf-8"))
    if ready.get("version")!=1 or type(ready.get("version")) is not int or ready.get("request_id")!=request or ready.get("manifest_sha256")!=digest:
        raise ValueError("App preparation receipt identity differs; no payload was pushed")
    if ready.get("status")!="ready":
        error = ready.get("error","PreparationFailed")
        safe = error if isinstance(error,str) and re.fullmatch(r"[A-Za-z0-9_]{1,80}",error) else "PreparationFailed"
        raise ValueError(f"App refused incoming directory ({safe}); preserve it and inspect app ownership. No automatic deletion or permission changes.")
    if ready.get("incoming_name")!=incoming or ready.get("generation")!=manifest["generation"]:
        raise ValueError("App prepared a different destination; no payload was pushed")
    # Snapshot is the final marker; partially transferred input is never announced complete.
    payloads = [*files,("snapshot.json",Path(bundle)/"snapshot.json")]
    for relative,source in payloads:
        target = f"/sdcard/Android/data/{PACKAGE}/files/{incoming}/{relative}"
        result = call("push",str(source.resolve()),target)
        if result.returncode:
            raise RuntimeError("Individual payload push failed; incoming remains for inspection, installed indexes were not changed")
    return {"status":"transferred","serial":serial,"request_id":request,"manifest_sha256":digest,"incoming_name":incoming,"pushed_files":len(payloads),
            "note":"Delivery only; run the app's small-index probe to verify app-UID reads, checksums and Windows parity"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial",required=True)
    parser.add_argument("--bundle",type=Path,required=True)
    parser.add_argument("--trusted-manifest",type=Path,required=True)
    parser.add_argument("--adb",default="adb")
    parser.add_argument("--timeout",type=float,default=60)
    args = parser.parse_args()
    print(json.dumps(prepare_and_push(args.bundle,args.trusted_manifest,args.serial,args.adb,timeout=args.timeout)))


if __name__=="__main__":
    main()
