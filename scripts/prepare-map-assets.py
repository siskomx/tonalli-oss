"""Explicit, pinned map-asset preparation; no network access at map-open/build time."""
import hashlib
import io
import json
import os
import posixpath
from pathlib import Path
import subprocess
import tarfile
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
COMMIT = "028c18f713baecad011301ff7a69acc39bcc2ae7"
REPOSITORY = "https://github.com/protomaps/basemaps-assets"
SOURCE_URL = f"https://codeload.github.com/protomaps/basemaps-assets/tar.gz/{COMMIT}"
RAW = f"https://raw.githubusercontent.com/protomaps/basemaps-assets/{COMMIT}/"
FONTS = ["Noto Sans Regular", "Noto Sans Medium", "Noto Sans Italic", "Noto Sans Devanagari Regular v1"]
DEST = ROOT / "web-stub/public/assets/map"
ARCHIVE = ROOT / "docs/spike/packs/map-assets-028c18f7.tar"


def require_workspace_path(path):
    resolved = path.resolve()
    if not resolved.is_relative_to(ROOT.resolve()):
        raise RuntimeError(f"asset path resolves outside workspace: {path}")
    return resolved


def require_member_mode(path, member, entry):
    mode = entry["mode"]
    if (mode == "120000" and member.issym()) or (mode in ("100644", "100755") and member.isfile()):
        return
    raise RuntimeError(f"Git/tar file mode mismatch: {path}")


def check_destinations():
    for path in (DEST, DEST.parent, ARCHIVE, ARCHIVE.parent, ROOT / "docs/spike/map-assets.toml"):
        require_workspace_path(path)


def fetch(url, api_token=None):
    """Download a URL, waiting and retrying when GitHub rate-limits or hiccups.

    api_token (the CI job token) is attached only when the caller names it, for the one GitHub API
    call: urllib would forward an Authorization header to a redirect target. Content is verified
    against pinned hashes afterwards, so credentials change availability, not trust.
    """
    for attempt in range(4):
        request = urllib.request.Request(url)
        if api_token:
            request.add_header("Authorization", f"Bearer {api_token}")
            request.add_header("Accept", "application/vnd.github+json")
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return response.read(32 * 1024 * 1024 + 1)
        except urllib.error.HTTPError as error:
            if error.code not in (403, 429, 500, 502, 503, 504) or attempt == 3:
                raise
            time.sleep(5 * 2 ** attempt)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def verify(root):
    manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    for record in manifest["files"]:
        data = (root / record["path"]).read_bytes()
        if len(data) != record["bytes"] or digest(data) != record["sha256"]:
            raise RuntimeError(f"asset verification failed: {record['path']}")


def write_inventory(manifest, archive_hash):
    quote = lambda value: json.dumps(value, ensure_ascii=False)
    lines = ["manifest_version = 1", 'status = "verified-generated"',
             f"repository = {quote(REPOSITORY)}", f"source_commit = {quote(COMMIT)}",
             f"source_archive_url = {quote(SOURCE_URL)}",
             f"source_archive_sha256 = {quote(manifest['source_archive_sha256'])}",
             "source_tree_files = 1029", "source_tree_bytes = 11536753",
             f"component_files = {len(manifest['files'])}",
             f"component_bytes = {sum(record['bytes'] for record in manifest['files'])}",
             'generated_archive = "packs/map-assets-028c18f7.tar"',
             f"archive_bytes = {ARCHIVE.stat().st_size}", f"archive_sha256 = {quote(archive_hash)}",
             f"manifest_sha256 = {quote(digest((DEST / 'manifest.json').read_bytes()))}",
             'provenance = "Locally generated deterministic TAR; no publisher SHA-256 claimed. Pinned Git tree blob hashes verified for selected source files and symlink text, then resolved font bytes materialized as regular files. Empty unused BMP files are retained. Git-tree bytes include symlink text; component bytes include materialized targets, generated styles, attribution and notices. Fresh extraction verified before publication."',
             "", "[npm]", 'maplibre_gl = "4.7.1"', 'pmtiles = "4.4.0"', 'protomaps_basemaps = "5.7.0"']
    for record in manifest["files"]:
        lines.extend(["", "[[files]]"])
        lines.extend(f"{key} = {value if isinstance(value, int) else quote(value)}" for key, value in record.items())
    (ROOT / "docs/spike/map-assets.toml").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main():
    check_destinations()
    ARCHIVE.parent.mkdir(parents=True, exist_ok=True)
    DEST.parent.mkdir(parents=True, exist_ok=True)
    check_destinations()
    with tempfile.TemporaryDirectory(prefix="map-assets-", dir=DEST.parent) as temporary:
        stage = Path(temporary) / "map"
        stage.mkdir()
        tree = json.loads(fetch(f"https://api.github.com/repos/protomaps/basemaps-assets/git/trees/{COMMIT}?recursive=1",
                                api_token=os.environ.get("GITHUB_TOKEN")))
        if tree.get("truncated"):
            raise RuntimeError("upstream tree inventory was truncated")
        entries = {entry["path"]: entry for entry in tree["tree"] if entry["type"] == "blob"}
        wanted = {f"fonts/{font}/{start}-{start + 255}.pbf": f"fonts/{font}/{start}-{start + 255}.pbf"
                  for font in FONTS for start in range(0, 65536, 256)}
        wanted["fonts/OFL.txt"] = "fonts/OFL.txt"
        wanted.update({f"sprites/v4/light{scale}.{ext}": f"sprites/sprite{scale}.{ext}"
                       for scale in ["", "@2x"] for ext in ["json", "png"]})
        source = fetch(SOURCE_URL)
        if len(source) > 32 * 1024 * 1024:
            raise RuntimeError("source archive exceeds preparation budget")
        records = []

        def install(path, data, url, **extra):
            target = stage / path
            require_workspace_path(target)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            records.append({"path": path, "url": url, "bytes": len(data), "sha256": digest(data), **extra})

        with tarfile.open(fileobj=io.BytesIO(source), mode="r:gz") as downloaded:
            members = {member.name.split("/", 1)[1]: member for member in downloaded.getmembers()
                       if "/" in member.name and (member.isfile() or member.issym())}

            def resolve_member(path, seen=()):
                if path in seen or len(seen) > 8:
                    raise RuntimeError(f"cyclic font link: {path}")
                member, expected = members[path], entries[path]
                require_member_mode(path, member, expected)
                data = member.linkname.encode("utf-8") if member.issym() else downloaded.extractfile(member).read()
                if len(data) != expected["size"] or len(data) > 1024 * 1024:
                    raise RuntimeError(f"unexpected source size: {path}")
                blob = hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()
                if blob != expected["sha"]:
                    raise RuntimeError(f"Git blob hash mismatch: {path}")
                if member.issym():
                    target = posixpath.normpath(posixpath.join(posixpath.dirname(path), member.linkname))
                    if not target.startswith("fonts/") or target not in wanted:
                        raise RuntimeError(f"font link escapes selected inventory: {path}")
                    return resolve_member(target, (*seen, path))
                return data, path, blob

            for path, destination in sorted(wanted.items()):
                data, resolved, blob = resolve_member(path)
                install(destination, data, RAW + urllib.parse.quote(resolved), git_blob=blob,
                        requested_source=path, requested_git_blob=entries[path]["sha"])
        if len(wanted) != 1029 or sum(entries[path]["size"] for path in wanted) != 11536753:
            raise RuntimeError("pinned font/sprite inventory does not match preflight")
        notices = {
            "sprites/LICENSE.txt": "https://raw.githubusercontent.com/tangrams/icons/d7e160177365e00532d36e35e689727b97eb6be1/LICENSE.md",
            "notices/basemaps-LICENSE.txt": "https://raw.githubusercontent.com/protomaps/basemaps/c4cac3d4b3b0c0b657bf61ebaa89659e83f4a362/LICENSE.md",
            "notices/pmtiles-LICENSE.txt": "https://raw.githubusercontent.com/protomaps/PMTiles/6f6c9c8697f26c5394dc351f04cec08656e905b6/LICENSE",
        }
        for path, url in notices.items():
            install(path, fetch(url), url)
        install("notices/maplibre-LICENSE.txt", (ROOT / "web-stub/node_modules/maplibre-gl/LICENSE.txt").read_bytes(),
                "https://registry.npmjs.org/maplibre-gl/-/maplibre-gl-4.7.1.tgz")
        subprocess.run(["node", str(ROOT / "web-stub/scripts/generate-map-style.mjs"), str(stage)], check=True)
        for path in ["style.json", "style-es.json"]:
            install(path, (stage / path).read_bytes(), "generated: @protomaps/basemaps@5.7.0; web-stub/scripts/generate-map-style.mjs")
        attribution = ("<!doctype html><meta charset=\"utf-8\"><title>Map attribution</title>"
                       "<h1>Map attribution</h1><p>© OpenStreetMap contributors. Map data: Open Database License (ODbL) 1.0. "
                       "Basemap design and tiles: Protomaps.</p><p>https://www.openstreetmap.org/copyright · https://protomaps.com</p>"
                       "<p>Fonts: Noto Project Authors, SIL OFL 1.1. Icons: Mapzen, MIT.</p>"
                       "<p><a href=\"/assets/map/fonts/OFL.txt\">Font license</a> · "
                       "<a href=\"/assets/map/sprites/LICENSE.txt\">Icon license</a></p>\n")
        install("attribution", attribution.encode("utf-8"), "generated: scripts/prepare-map-assets.py")
        manifest = {"source_commit": COMMIT, "source_archive": SOURCE_URL, "source_archive_sha256": digest(source),
                    "files": sorted(records, key=lambda record: record["path"])}
        (stage / "manifest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        verify(stage)
        staged_tar = Path(temporary) / "map-assets.tar"
        with tarfile.open(staged_tar, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            for path in sorted(stage.rglob("*")):
                if path.is_file():
                    data = path.read_bytes()
                    info = tarfile.TarInfo("assets/map/" + path.relative_to(stage).as_posix())
                    info.size, info.mode, info.mtime = len(data), 0o644, 0
                    archive.addfile(info, io.BytesIO(data))
        # Only our fixed, regular-file archive is extracted, with Python's data filter.
        extracted = Path(temporary) / "extracted"
        with tarfile.open(staged_tar) as archive:
            archive.extractall(extracted, filter="data")
        verify(extracted / "assets/map")
        archive_hash = digest(staged_tar.read_bytes())
        if ARCHIVE.exists() and digest(ARCHIVE.read_bytes()) != archive_hash:
            raise RuntimeError("existing generated archive differs; preserved for review")
        if DEST.exists():
            verify(DEST)
            if (DEST / "manifest.json").read_bytes() != (stage / "manifest.json").read_bytes():
                raise RuntimeError("existing assets differ; preserved for review")
        else:
            check_destinations()
            stage.rename(DEST)
        if not ARCHIVE.exists():
            # Hard link publication cannot clobber an existing archive.
            check_destinations()
            os.link(staged_tar, ARCHIVE)
        check_destinations()
        write_inventory(manifest, archive_hash)
        print(json.dumps({"archive": str(ARCHIVE.relative_to(ROOT)), "bytes": ARCHIVE.stat().st_size,
                          "sha256": archive_hash, "components": len(records),
                          "component_bytes": sum(record["bytes"] for record in records),
                          "manifest_sha256": digest((DEST / "manifest.json").read_bytes()),
                          "source_archive_sha256": digest(source)}, indent=2))


if __name__ == "__main__":
    main()
