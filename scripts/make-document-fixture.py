"""Create the small original document pack using the existing pinned libzim writer."""
import hashlib
import json
from pathlib import Path
import runpy
import sys
from importlib.metadata import version
from libzim.version import get_versions
from libzim.writer import Creator, Compression
from libzim.reader import Archive

root = Path(__file__).resolve().parents[1]
if version('libzim') != '4.1.0' or get_versions()['libzim'] != '9.8.2':
    raise RuntimeError('Use the existing pinned libzim4.1.0/native9.8.2 environment')
output = Path(sys.argv[1]).resolve()
if output.exists():
    raise RuntimeError('Refuse to overwrite an existing fixture')
inputs = [
    ('docs/guide.pdf', 'application/pdf', sys.argv[2], '03f0d728f59258bd86267f7397609747c027fa7b69ef51ba9bc9084a6187b30e'),
    ('docs/book.epub', 'application/epub+zip', sys.argv[3], 'fe4d6d8cfa4949fce03b87840752397a6df07d8ada12de8cbc2cbf62aecf0575'),
    ('docs/comic.cbz', 'application/vnd.comicbook+zip', sys.argv[4], '9cbb9218de29e65a4909742c7c3e8ccd01a0134fb1d171d28863f989e5332bb3'),
]
html = b'''<!doctype html><html><head><meta charset="utf-8"><title>Embedded documents</title></head><body>
<h1>Embedded documents</h1><p><a href="docs/guide.pdf">Read PDF</a> <a href="shortcut.pdf">PDF redirect</a>
<a href="docs/book.epub">Read EPUB</a> <a href="docs/comic.cbz">Read comic</a> <a href="spoof.pdf">HTML named PDF</a>
<a href="redirect.html#section">Redirect with anchor</a></p>
<object data="docs/guide.pdf" type="application/pdf">Embedded PDF fallback</object>
<embed src="docs/guide.pdf" type="application/pdf"><div style="height:1100px"></div><h2 id="section">Saved anchor target</h2>
</body></html>'''
rows = [('index.html', 'text/html', html), ('spoof.pdf', 'text/html', b'<html><head><title>HTML page</title></head><body><h1>This is HTML</h1></body></html>')]
for name, mime, source, expected in inputs:
    data = Path(source).read_bytes()
    if hashlib.sha256(data).hexdigest() != expected:
        raise RuntimeError('Source fixture hash mismatch')
    rows.append((name, mime, data))
Item = runpy.run_path(str(root / 'scripts/make-article-fixtures.py'))['FixtureItem']
output.parent.mkdir(parents=True, exist_ok=True)
with Creator(output).config_compression(Compression.none).config_indexing(False, 'eng').config_nbworkers(1) as creator:
    creator.set_mainpath('index.html')
    creator.add_metadata('Title', 'Tonalli embedded document fixture')
    creator.add_metadata('Language', 'eng')
    for name, mime, data in rows:
        creator.add_item(Item(name, name, mime, data))
    creator.add_redirection('shortcut.pdf', 'PDF redirect', 'docs/guide.pdf', {})
    creator.add_redirection('redirect.html', 'HTML redirect', 'index.html', {})
archive = Archive(output)
for name, mime, data in rows:
    assert bytes(archive.get_entry_by_path(name).get_item().content) == data
for source, target in [('shortcut.pdf', 'docs/guide.pdf'), ('redirect.html', 'index.html')]:
    entry = archive.get_entry_by_path(source)
    assert entry.is_redirect and entry.get_redirect_entry().path == target
raw = output.read_bytes()
assert hashlib.md5(raw[:-16]).digest() == raw[-16:]
manifest = {'path': output.relative_to(root).as_posix(), 'bytes': output.stat().st_size,
    'sha256': hashlib.sha256(output.read_bytes()).hexdigest(), 'python_libzim': version('libzim'),
    'native_versions': get_versions(), 'compression': 'none', 'indexing': False,
    'entries': [{'url': 'C/' + name, 'mime': mime, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()} for name, mime, data in rows]}
output.with_suffix('.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
print(json.dumps(manifest))
