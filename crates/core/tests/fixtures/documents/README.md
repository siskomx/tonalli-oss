# Original embedded-document fixture

`documents-v1.zim` is a small original test pack, not downloadable user content.
Its frozen bytes, member hashes and exact writer versions are in
`documents-v1.json`. Root GPL-3.0-or-later applies. It contains synthetic PDF,
EPUB and CBZ inputs already recorded in `docs/spike/packs.toml`, an HTML page
with embedded documents, an HTML page named `.pdf`, and two redirects.

Author with the already verified Python libzim4.1.0/native9.8.2 environment:

```powershell
rtk proxy python scripts/make-document-fixture.py OUTPUT.zim Document-check.pdf Book-check.epub Comic-check.cbz
```

Input hashes are enforced. Generate the original inputs using the existing
`web-stub/tests/e2e/documents.spec.mjs` and `books.spec.mjs` fixture code; those
scripts retain their inputs under `target/documents-browser`/`documents-books`.
The authoring script refuses to overwrite a pack. Writer UUIDs vary, so a new
archive has a different SHA even with identical member bytes. Preserve this
frozen pack for ordinary tests, which need neither libzim nor network access.

Author verification checks exact member bytes, both redirects and the official
archive MD5. Libzim's own small title scratch files are ignored; they are not
the product index worker and are never bundled as fixture inputs.
