"""Freeze the small Windows index for Android's checked portability probe."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import tempfile

MAX_BYTES = 32 * 1024 * 1024
MAX_FILES = 128
LOCKS = {".tantivy-meta.lock", ".tantivy-writer.lock"}


def generation_name(value):
    if not isinstance(value, str) or not re.fullmatch(r"generation-[A-Za-z0-9_-]+", value):
        raise ValueError("Invalid generation name")
    return value


def index_filename(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9._-]+", value) or value.endswith("."):
        raise ValueError("Unsupported index filename")
    return value


def ordinary(path, directory=False):
    if path.is_symlink() or path.is_junction():
        raise ValueError(f"Links are not supported: {path}")
    if not (path.is_dir() if directory else path.is_file()):
        raise ValueError(f"Not an ordinary {'directory' if directory else 'file'}: {path}")


def export(source, destination):
    source, destination = Path(source), Path(destination)
    if destination.exists():
        raise FileExistsError(destination)
    ordinary(source, directory=True)
    pointer = source / "CURRENT"
    ordinary(pointer)
    if pointer.stat().st_size > 256:
        raise ValueError("Oversized CURRENT")
    generation = generation_name(pointer.read_text(encoding="utf-8").strip())
    folder = source / generation
    ordinary(folder, directory=True)
    paths = [pointer]
    for entry in sorted(folder.iterdir()):
        ordinary(entry)
        index_filename(entry.name)
        if entry.name not in LOCKS:
            paths.append(entry)
    if not {"meta.json", "tonalli-meta.json"}.issubset({p.name for p in paths}):
        raise ValueError("Missing index metadata")
    if len(paths) > MAX_FILES or sum(p.stat().st_size for p in paths) > MAX_BYTES:
        raise ValueError("Bundle exceeds small-probe limits")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".index-export-", dir=destination.parent) as scratch:
        staged = Path(scratch) / "payload"
        staged.mkdir()
        records = []
        total = 0
        for path in paths:
            relative = path.relative_to(source).as_posix()
            target = staged / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            digest = hashlib.sha256()
            count = 0
            with path.open("rb") as incoming, target.open("xb") as outgoing:
                while block := incoming.read(64 * 1024):
                    count += len(block)
                    total += len(block)
                    if total > MAX_BYTES:
                        raise ValueError("Source grew beyond small-probe limit")
                    digest.update(block)
                    outgoing.write(block)
            records.append({"path": relative, "bytes": count, "sha256": digest.hexdigest()})
        # Detect source selection changes rather than publishing a mixed snapshot.
        if (staged / "CURRENT").read_bytes() != pointer.read_bytes():
            raise ValueError("CURRENT changed during export")
        if (staged / "CURRENT").read_text(encoding="utf-8").strip() != generation:
            raise ValueError("CURRENT changed before copying")
        manifest = (json.dumps({"version": 1, "generation": generation, "files": records},
                               ensure_ascii=True, indent=2) + "\n").encode("utf-8")
        if len(manifest) > 64 * 1024:
            raise ValueError("Oversized manifest")
        (staged / "snapshot.json").write_bytes(manifest)
        # Windows rename refuses an existing destination; retained bundles stay immutable.
        staged.rename(destination)
    return manifest


def validated_bundle(bundle, trusted_manifest):
    """Validate exact trusted export bytes before preparing any remote directory."""
    bundle, trusted_manifest = Path(bundle), Path(trusted_manifest)
    ordinary(bundle, directory=True)
    ordinary(trusted_manifest)
    ordinary(bundle/"snapshot.json")
    if trusted_manifest.stat().st_size > 64*1024 or (bundle/"snapshot.json").stat().st_size > 64*1024:
        raise ValueError("Oversized trusted snapshot manifest")
    raw = trusted_manifest.read_bytes()
    if raw != (bundle/"snapshot.json").read_bytes():
        raise ValueError("Bundle snapshot differs from trusted packaged manifest")
    manifest = json.loads(raw)
    if not isinstance(manifest,dict) or type(manifest.get("version")) is not int or manifest["version"] != 1:
        raise ValueError("Unsupported snapshot version")
    generation = generation_name(manifest.get("generation"))
    ordinary(bundle/generation, directory=True)
    records = manifest.get("files")
    if not isinstance(records,list) or not 2 <= len(records) <= MAX_FILES:
        raise ValueError("Invalid snapshot file count")
    selected, total, names = [], 0, set()
    for record in records:
        if not isinstance(record,dict):
            raise ValueError("Invalid snapshot record")
        relative = record.get("path")
        if not isinstance(relative,str):
            raise ValueError("Invalid snapshot path")
        parts = relative.split("/")
        if relative != "CURRENT" and not (len(parts)==2 and parts[0]==generation and index_filename(parts[1]) not in LOCKS):
            raise ValueError("Snapshot path escapes its generation")
        size, expected = record.get("bytes"), record.get("sha256")
        if relative in names or type(size) is not int or size < 0 or not isinstance(expected,str) or not re.fullmatch(r"[0-9a-f]{64}",expected):
            raise ValueError("Invalid or duplicate snapshot record")
        names.add(relative)
        total += size
        if total > MAX_BYTES:
            raise ValueError("Bundle exceeds small-probe limit")
        path = bundle/relative
        ordinary(path)
        if path.stat().st_size != size:
            raise ValueError("Snapshot payload length changed")
        digest = hashlib.sha256()
        read = 0
        with path.open("rb") as source:
            for block in iter(lambda: source.read(64*1024),b""):
                read += len(block)
                if read > size:
                    raise ValueError("Snapshot payload grew during validation")
                digest.update(block)
        if digest.hexdigest() != expected:
            raise ValueError("Snapshot payload checksum changed")
        selected.append((relative,path))
    if not {"CURRENT",generation+"/meta.json",generation+"/tonalli-meta.json"}.issubset(names):
        raise ValueError("Missing index metadata")
    if (bundle/"CURRENT").stat().st_size > 256 or (bundle/"CURRENT").read_text(encoding="utf-8").strip() != generation:
        raise ValueError("CURRENT differs from snapshot generation")
    return manifest, hashlib.sha256(raw).hexdigest(), selected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path, help="New immutable output directory")
    parser.add_argument("--manifest-output", type=Path, help="Explicit trusted Android asset path")
    args = parser.parse_args()
    if args.manifest_output and args.manifest_output.exists():
        parser.error("Trusted manifest output already exists; compare it rather than overwriting")
    manifest = export(args.source, args.destination)
    if args.manifest_output:
        args.manifest_output.parent.mkdir(parents=True, exist_ok=True)
        with args.manifest_output.open("xb") as output:
            output.write(manifest)
    print(f"Bundle: {args.destination.resolve()}")
    print(f"Manifest SHA256: {hashlib.sha256(manifest).hexdigest()}")


if __name__ == "__main__":
    main()
