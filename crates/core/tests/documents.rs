use std::{
    io::{Cursor, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tonalli_core::{
    documents::{Book, BookKind},
    pack::{PackError, PackSource},
};
use zip::{write::SimpleFileOptions, ZipWriter};

struct Source {
    bytes: Vec<u8>,
    removed: AtomicBool,
    short: bool,
}
impl PackSource for Source {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize, PackError> {
        if self.removed.load(Ordering::SeqCst) {
            return Err(PackError::MediaRemoved);
        }
        let start = usize::try_from(offset).map_err(|_| PackError::Truncated)?;
        let maximum = if self.short { 3 } else { output.len() };
        let count = output
            .len()
            .min(maximum)
            .min(self.bytes.len().saturating_sub(start));
        if count > 0 {
            output[..count].copy_from_slice(&self.bytes[start..start + count]);
        }
        Ok(count)
    }
}
fn source(bytes: Vec<u8>, short: bool) -> Arc<Source> {
    Arc::new(Source {
        bytes,
        removed: AtomicBool::new(false),
        short,
    })
}
fn archive(rows: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in rows {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn epub(extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut rows:Vec<(&str,&[u8])>=vec![
        ("mimetype",b"application/epub+zip"),
        ("META-INF/container.xml",br#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OPS/package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#),
        ("OPS/package.opf",br#"<package><metadata><title>Libro de prueba</title></metadata><manifest><item id="a" href="chapter-one.xhtml" media-type="application/xhtml+xml"/><item id="b" href="chapter-two.xhtml" media-type="application/xhtml+xml"/><item id="pic" href="../images/cover.png" media-type="image/png"/></manifest><spine><itemref idref="b"/><itemref idref="a"/></spine></package>"#),
        ("OPS/chapter-one.xhtml",b"<html><body><h1>First</h1></body></html>"),
        ("OPS/chapter-two.xhtml",b"<html><body><h1>Second</h1></body></html>"),
        ("images/cover.png",b"synthetic image bytes"),
    ];
    rows.extend_from_slice(extra);
    archive(&rows)
}
#[test]
fn epub_uses_container_and_spine_and_reads_without_extracting_or_shared_cursor() {
    let input = source(epub(&[]), true);
    let book = Book::open(input.clone(), BookKind::Epub).unwrap();
    let metadata = book.metadata();
    assert_eq!(metadata.kind, BookKind::Epub);
    assert_eq!(
        metadata
            .chapters
            .iter()
            .map(|chapter| chapter.path.as_str())
            .collect::<Vec<_>>(),
        ["OPS/chapter-two.xhtml", "OPS/chapter-one.xhtml"]
    );
    assert_eq!(
        book.resource("OPS/chapter-two.xhtml").unwrap().mime,
        "text/html; charset=utf-8"
    );
    assert!(
        String::from_utf8(book.resource("OPS/chapter-one.xhtml").unwrap().bytes)
            .unwrap()
            .contains("First")
    );
    assert_eq!(
        book.resource("images/cover.png").unwrap().bytes,
        b"synthetic image bytes"
    );
    assert!(matches!(
        book.resource("../notes.db"),
        Err(PackError::NotFound(_))
    ));
    input.removed.store(true, Ordering::SeqCst);
    assert!(matches!(
        book.resource("OPS/chapter-one.xhtml"),
        Err(PackError::MediaRemoved)
    ));
}
#[test]
fn comic_pages_are_naturally_ordered_and_other_entries_are_not_pages() {
    let bytes = archive(&[
        ("pages/page10.png", b"ten"),
        ("pages/page2.png", b"two"),
        ("pages/page01.png", b"one"),
        ("README.txt", b"not a page"),
    ]);
    let book = Book::open(source(bytes, false), BookKind::Comic).unwrap();
    assert_eq!(
        book.metadata()
            .chapters
            .iter()
            .map(|chapter| chapter.path.as_str())
            .collect::<Vec<_>>(),
        ["pages/page01.png", "pages/page2.png", "pages/page10.png"]
    );
}

#[test]
fn epub_accepts_svg_spine_documents() {
    let bytes = archive(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", br#"<container><rootfiles><rootfile full-path="package.opf"/></rootfiles></container>"#),
        ("package.opf", br#"<package><manifest><item id="cover" href="cover.svg" media-type="image/svg+xml"/></manifest><spine><itemref idref="cover"/></spine></package>"#),
        ("cover.svg", br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 600"><text x="20" y="40">SVG cover</text></svg>"#),
    ]);
    let book = Book::open(source(bytes, true), BookKind::Epub).unwrap();
    assert_eq!(book.metadata().chapters[0].path, "cover.svg");
    assert_eq!(book.resource("cover.svg").unwrap().mime, "image/svg+xml");
}
#[test]
fn archive_bounds_traversal_duplicate_names_and_truncation_are_typed_errors() {
    for name in [
        "../private/notes.db",
        "/absolute.png",
        "folder\\page.png",
        "C:/page.png",
    ] {
        assert!(
            Book::open(
                source(archive(&[(name, b"untrusted")]), false),
                BookKind::Comic
            )
            .is_err(),
            "{name}"
        );
    }
    let mut duplicate = archive(&[("page-a.png", b"one"), ("page-b.png", b"two")]);
    for start in 0..duplicate.len().saturating_sub(10) {
        if &duplicate[start..start + 10] == b"page-b.png" {
            duplicate[start + 5] = b'a';
        }
    }
    assert!(Book::open(source(duplicate, false), BookKind::Comic).is_err());
    let good = epub(&[]);
    for end in [0, 4, 20, good.len() - 1] {
        assert!(Book::open(source(good[..end].to_vec(), false), BookKind::Epub).is_err());
    }
    let mut excessive = good.clone();
    let eocd = excessive.len() - 22;
    excessive[eocd + 8..eocd + 12].fill(0xfe);
    assert!(matches!(
        Book::open(source(excessive, false), BookKind::Epub),
        Err(PackError::Mismatch(_))
    ));
    let bomb = archive(&[("huge.xhtml", &vec![b'A'; 33 * 1024 * 1024])]);
    assert!(Book::open(source(bomb, false), BookKind::Epub).is_err());
}
#[test]
fn epub_external_spine_and_entity_definitions_do_not_resolve_to_other_data() {
    let opf=br#"<!DOCTYPE package [<!ENTITY escape SYSTEM "file:///notes.db">]><package><manifest><item id="a" href="https://example.invalid/book.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="a"/></spine></package>"#;
    let bytes = archive(&[
        ("mimetype", b"application/epub+zip"),
        (
            "META-INF/container.xml",
            br#"<container><rootfiles><rootfile full-path="book.opf"/></rootfiles></container>"#,
        ),
        ("book.opf", opf),
    ]);
    assert!(Book::open(source(bytes, false), BookKind::Epub).is_err());
}

#[test]
fn epub_utf16_metadata_and_external_doctype_need_no_network_or_entity_resolver() {
    let xml="<?xml version=\"1.0\" encoding=\"UTF-16\"?><container><rootfiles><rootfile full-path=\"book.opf\"/></rootfiles></container>";
    let mut encoded = vec![0xff, 0xfe];
    for unit in xml.encode_utf16() {
        encoded.extend(unit.to_le_bytes());
    }
    let opf=br#"<?xml version="1.0"?><!DOCTYPE package PUBLIC "epub package" "https://example.invalid/never-fetch.dtd"><package><manifest><item id="one" href="one.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="one"/></spine></package>"#;
    let bytes = archive(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", &encoded),
        ("book.opf", opf),
        ("one.xhtml", b"<html><body>Page</body></html>"),
    ]);
    assert_eq!(
        Book::open(source(bytes, false), BookKind::Epub)
            .unwrap()
            .metadata()
            .chapters[0]
            .path,
        "one.xhtml"
    );
}

#[test]
fn zip64_locator_is_rejected_even_before_a_maximum_length_comment() {
    let mut bytes = epub(&[]);
    let end = bytes.len() - 22;
    let mut footer = bytes.split_off(end);
    let size = u32::from_le_bytes(footer[12..16].try_into().unwrap());
    footer[12..16].copy_from_slice(&(size + 20).to_le_bytes());
    footer[20..22].copy_from_slice(&u16::MAX.to_le_bytes());
    bytes.extend(b"PK\x06\x07");
    bytes.extend([0; 16]);
    bytes.extend(footer);
    bytes.extend(vec![0; u16::MAX as usize]);
    assert!(matches!(
        Book::open(source(bytes, false), BookKind::Epub),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn corrupt_member_checksum_and_symlink_are_rejected() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "page.png",
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"ABCDE").unwrap();
    let mut bytes = writer.finish().unwrap().into_inner();
    let index = bytes
        .windows(5)
        .position(|value| value == b"ABCDE")
        .unwrap();
    bytes[index] = b'X';
    let book = Book::open(source(bytes, false), BookKind::Comic).unwrap();
    assert!(matches!(
        book.resource("page.png"),
        Err(PackError::Corrupt(_))
    ));
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink("page.png", "../notes.db", SimpleFileOptions::default())
        .unwrap();
    assert!(Book::open(
        source(writer.finish().unwrap().into_inner(), false),
        BookKind::Comic
    )
    .is_err());
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("page.png", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"one page").unwrap();
    writer
        .add_symlink(
            "linked-folder/",
            "../notes.db",
            SimpleFileOptions::default(),
        )
        .unwrap();
    assert!(Book::open(
        source(writer.finish().unwrap().into_inner(), false),
        BookKind::Comic
    )
    .is_err());
}
