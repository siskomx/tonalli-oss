"""Check benchmark stdin protocol on the verified small English fixture."""
import argparse
import os
from pathlib import Path
import re
import runpy
import shutil
import subprocess
import threading
import time
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    helper = runpy.run_path(str(root / "scripts/benchmark-zim.py"))
    manifest = root / "docs/spike/packs.toml"
    pack = next(p for p in tomllib.loads(manifest.read_text(encoding="utf-8"))["packs"] if p["id"] == "wikipedia-en-knots")
    corpus = manifest.parent / pack["path"]
    assert corpus.stat().st_size == pack["bytes"]
    assert helper["digest"](corpus) == pack["sha256"]
    output = (args.output or root / "target" / f"benchmark-protocol-{time.time_ns()}").resolve()
    output.mkdir(parents=True, exist_ok=False)
    runner = output / "runner.exe"
    expected = helper["digest"](args.executable)
    shutil.copy2(args.executable, runner)
    assert helper["digest"](runner) == expected
    environment = os.environ.copy()
    environment.update(TONALLI_BENCH_ZIM=str(corpus), TONALLI_BENCH_TITLES=str(output / "titles.idx"),
                       TONALLI_BENCH_PREFIX="trefoil", TONALLI_BENCH_ARTICLE="C/Trefoil_knot",
                       TONALLI_BENCH_MODE="fresh")
    prepare = subprocess.run([str(runner)], env=environment, capture_output=True, text=True,
                             encoding="utf-8", timeout=30, creationflags=subprocess.CREATE_NO_WINDOW)
    assert prepare.returncode == 0, prepare.stderr
    (output / "prepare.txt").write_text(prepare.stdout + prepare.stderr, encoding="utf-8")
    for operation, command, succeeds in (("suggest", "run\n", True), ("article", "run\n", True),
                                         ("suggest", "RUN\n", False), ("article", "", False)):
        environment["TONALLI_BENCH_MODE"] = f"cold-{operation}"
        with subprocess.Popen([str(runner)], env=environment, stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                              encoding="utf-8", creationflags=subprocess.CREATE_NO_WINDOW) as process:
            watchdog = threading.Timer(30, process.kill)
            watchdog.daemon = True
            watchdog.start()
            try:
                prefix = ""
                while True:
                    line = process.stdout.readline()
                    assert line, "Harness exited or timed out before flushing READY"
                    prefix += line
                    if line.startswith("READY "):
                        assert re.fullmatch(rf"READY pid={process.pid} operation={operation}\n", line), line
                        break
                stdout, stderr = process.communicate(input=command, timeout=30)
                text = prefix + stdout + stderr
                name = f"{operation}-{'run' if succeeds else 'wrong' if command else 'eof'}"
                (output / f"{name}.txt").write_text(text, encoding="utf-8")
                assert (process.returncode == 0) == succeeds, text
                if succeeds:
                    assert f"{operation}_ms sample=" in stdout
                    assert "reset_verified=false" in stdout
                    other = "article" if operation == "suggest" else "suggest"
                    assert f"{other}_ms sample=" not in stdout
                else:
                    assert "expected exactly 'run'" in stderr
                    assert "_ms sample=" not in stdout
            finally:
                watchdog.cancel()
    print("PASS: READY flush/PID, single-operation run, wrong command and EOF rejection")
    print("Protocol checks only; no cache reset or cold-performance claim")
    print(output)


if __name__ == "__main__":
    main()
