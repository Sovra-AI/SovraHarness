//! Local index extraction is independent of picture-model attachment packaging.

use super::*;
use sovra_core::file_index::ExtractionState;
use std::io::Write as _;
use std::sync::Arc;

#[test]
fn binary_fragments_are_not_indexed_as_document_text() {
    use sovra_core::file_index::{NoticeKind, NoticeQuery};
    let bytes = b"Binaryfragmentneedle\0\xff\x01\x02";
    for name in ["notes.txt", "pg_filenode.map", "unknown.custom"] {
        let fixture = Fixture::new();
        std::fs::write(fixture.source().join(name), bytes).unwrap();
        let store = fixture.store();
        let root = store.add_root(&fixture.source()).unwrap();
        store.scan(&root.id, &Observer::default()).unwrap();
        for store in [&store, &fixture.store()] {
            let page = store.catalogue(&CatalogQuery::default()).unwrap();
            assert_eq!(page.total_files, 1);
            let entry = &page.files[0];
            assert_eq!(entry.extraction, ExtractionState::MetadataOnly, "{name}");
            assert_eq!(entry.passage_count, 0);
            assert!(entry.first_passage.is_none());
            assert!(entry.note.as_deref().unwrap().contains("Attach it in chat"));
            assert_eq!(search(store, "Binaryfragmentneedle"), []);
            let report = store
                .report(&NoticeQuery {
                    root_id: root.id.clone(),
                    kind: Some(NoticeKind::Unreadable),
                    contains: None,
                    match_case: false,
                    cursor: None,
                    limit: 40,
                })
                .unwrap();
            assert_eq!(report.total, 1);
            assert_eq!(report.notices[0].path, name);
        }
        let attached = crate::prepare_bytes("binary", name, bytes).unwrap();
        assert_eq!(attached.raw.as_deref(), Some(bytes.as_slice()));
        assert!(attached.text.contains("CONTENT NOT FULLY DECODED"));
        assert_eq!(
            store
                .scan(&root.id, &Observer::default())
                .unwrap()
                .progress
                .reused,
            1
        );
        fixture.write(
            name,
            "Convertedtextneedle has readable Unicode: café and �.",
        );
        store.scan(&root.id, &Observer::default()).unwrap();
        let readable = store.catalogue(&CatalogQuery::default()).unwrap();
        assert_eq!(readable.files[0].extraction, ExtractionState::Text);
        assert_eq!(search(&store, "Convertedtextneedle").len(), 1);
    }
}

fn partial_presentation() -> Vec<u8> {
    use std::io::{Cursor, Read as _};
    let original = crate::format_tests::packages::presentation_with_slides(3);
    let mut input = zip::ZipArchive::new(Cursor::new(original)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..input.len() {
        let mut part = input.by_index(index).unwrap();
        let name = part.name().unwrap().into_owned();
        if name == "ppt/slides/slide2.xml" {
            continue;
        }
        let mut bytes = Vec::new();
        part.read_to_end(&mut bytes).unwrap();
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

fn archive_member(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive
        .start_file(name, zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(bytes).unwrap();
    archive.finish().unwrap().into_inner()
}

#[test]
fn partial_presentation_keeps_readable_slides_and_explains_the_missing_slide() {
    use sovra_core::file_index::{CatalogGroupBy, FileHistoryQuery, NoticeKind, NoticeQuery};
    let broken = partial_presentation();
    let attached = crate::prepare_bytes("partial", "partial.pptx", &broken).unwrap();
    assert!(attached.attachment.note.unwrap().contains("slide2.xml"));
    for name in ["partial.pptx", "bundle.zip"] {
        let package = |bytes: &[u8]| {
            if name == "bundle.zip" {
                archive_member("partial.pptx", bytes)
            } else {
                bytes.to_vec()
            }
        };
        let fixture = Fixture::new();
        std::fs::write(fixture.source().join(name), package(&broken)).unwrap();
        let store = fixture.store();
        let root = store.add_root(&fixture.source()).unwrap();
        let scanned = store.scan(&root.id, &Observer::default()).unwrap();
        assert_eq!(scanned.progress.failed, 0);
        assert_eq!(search(&store, "marker1needle").len(), 1);
        let passage = search(&store, "marker3needle").remove(0);
        assert!(passage.note.as_ref().unwrap().contains("slide2.xml"));
        assert!(!passage.text.contains("Reader details"));
        let mut query = CatalogQuery::default();
        query.filter.extraction = Some(ExtractionState::PartialText);
        query.filter.has_text = Some(true);
        let report = NoticeQuery {
            root_id: root.id.clone(),
            kind: Some(NoticeKind::Partial),
            contains: None,
            match_case: false,
            cursor: None,
            limit: 40,
        };
        let history_query = FileHistoryQuery {
            root_id: root.id.clone(),
            path: Some(name.into()),
            ..FileHistoryQuery::default()
        };
        for store in [&store, &fixture.store()] {
            let files = store.catalogue(&query).unwrap();
            assert_eq!(files.total_files, 1);
            assert!(files.files[0].first_passage.is_some());
            assert!(
                files.files[0]
                    .note
                    .as_deref()
                    .unwrap()
                    .contains("slide2.xml")
            );
            let page = store.report(&report).unwrap();
            assert_eq!(page.total, 1);
            assert_eq!(page.counts, vec![(NoticeKind::Partial, 1)]);
            assert!(page.notices[0].message.contains("slide2.xml"));
            assert_eq!(
                store
                    .scan(&root.id, &Observer::default())
                    .unwrap()
                    .progress
                    .reused,
                1
            );
        }
        query.group_by = Some(CatalogGroupBy::Extension);
        assert_eq!(
            store.catalogue(&query).unwrap().groups[0].files_with_text,
            1
        );
        query.group_by = None;
        query.filter.has_text = Some(false);
        assert_eq!(store.catalogue(&query).unwrap().total_files, 0);
        assert_eq!(
            store.history(&history_query).unwrap().observations[0].extraction,
            ExtractionState::PartialText
        );
        std::fs::write(
            fixture.source().join(name),
            package(&crate::format_tests::packages::presentation_with_slides(3)),
        )
        .unwrap();
        store.scan(&root.id, &Observer::default()).unwrap();
        let fixed = store.catalogue(&CatalogQuery::default()).unwrap();
        assert_eq!(fixed.files[0].extraction, ExtractionState::Text);
        assert!(
            !fixed.files[0]
                .note
                .as_deref()
                .unwrap_or("")
                .contains("slide2.xml")
        );
        assert_eq!(search(&store, "marker2needle").len(), 1);
        assert_eq!(store.report(&report).unwrap().total, 0);
        assert_eq!(store.history(&history_query).unwrap().observations.len(), 2);
        assert!(store.read(&passage.reference).is_err());
    }
}

#[test]
fn cancellation_during_document_decoding_preserves_the_previous_version() {
    struct CancelDuringDecode(AtomicBool);
    impl ScanObserver for CancelDuringDecode {
        fn cancellation(&self) -> Option<ScanCancellation> {
            self.0
                .load(Ordering::Relaxed)
                .then_some(ScanCancellation::Requested)
        }
        fn progress(&self, _: &IndexedRoot) {}
        fn watch_cancellation(
            &self,
            signal: Arc<dyn Fn() + Send + Sync>,
        ) -> Option<Box<dyn sovra_core::file_index::ScanCancellationWatch>> {
            self.0.store(true, Ordering::Relaxed);
            signal();
            None
        }
    }
    for name in ["notes.pdf", "bundle.zip"] {
        let package = |bytes: Vec<u8>| {
            if name == "notes.pdf" {
                return bytes;
            }
            let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            archive
                .start_file("notes.pdf", zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(&bytes).unwrap();
            archive.finish().unwrap().into_inner()
        };
        let fixture = Fixture::new();
        std::fs::write(
            fixture.source().join(name),
            package(document(1, |_| {
                "Originaldocumentneedle is retained until a complete replacement.".into()
            })),
        )
        .unwrap();
        let store = fixture.store();
        let root = store.add_root(&fixture.source()).unwrap();
        store.scan(&root.id, &Observer::default()).unwrap();
        let original = search(&store, "Originaldocumentneedle")[0]
            .reference
            .clone();
        std::fs::write(
            fixture.source().join(name),
            package(document(601, |_| {
                "Replacementdocumentneedle is new source content.".into()
            })),
        )
        .unwrap();
        let cancelled = store
            .scan(&root.id, &CancelDuringDecode(AtomicBool::new(false)))
            .unwrap();
        assert_eq!(cancelled.status, IndexStatus::Cancelled);
        assert_eq!(cancelled.progress.failed, 0);
        assert_eq!(cancelled.progress.indexed, 0);
        assert_eq!(search(&store, "Originaldocumentneedle").len(), 1);
        assert_eq!(
            search(&store, "Replacementdocumentneedle"),
            [] as [IndexPassage; 0]
        );
        assert!(store.read(&original).is_ok());
        let count: i64 = store
            .connect()
            .unwrap()
            .query_row("SELECT count(*) FROM index_versions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        store.scan(&root.id, &Observer::default()).unwrap();
        assert_ne!(
            search(&store, "Replacementdocumentneedle"),
            [] as [IndexPassage; 0]
        );
    }
}

fn document(pages: usize, text: impl Fn(usize) -> String) -> Vec<u8> {
    document_with_padding(pages, text, 0)
}

fn document_with_padding(pages: usize, text: impl Fn(usize) -> String, padding: usize) -> Vec<u8> {
    let children = (0..pages)
        .map(|page| format!("{} 0 R", 4 + page * 2))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{children}] /Count {pages} >>").into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    for page in 0..pages {
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", 5 + page * 2).into_bytes());
        objects.push(crate::format_tests::stream(
            format!("BT /F1 12 Tf 20 700 Td ({}) Tj ET", text(page)).as_bytes(),
            "",
        ));
    }
    if padding > 0 {
        objects.push(crate::format_tests::stream(&vec![b'x'; padding], ""));
    }
    crate::format_tests::pdf(&objects)
}

#[test]
fn rich_documents_larger_than_chat_attachments_are_searchable() {
    let fixture = Fixture::new();
    let bytes = document_with_padding(
        1,
        |_| "Largedocumentneedle is searchable beyond the attachment size limit.".into(),
        65 * 1024 * 1024,
    );
    assert!(bytes.len() as u64 > sovra_core::files::MAX_FILE_BYTES);
    std::fs::write(fixture.source().join("large.pdf"), &bytes).unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let scanned = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(scanned.progress.failed, 0);
    assert_eq!(
        search(&store, "Largedocumentneedle").len(),
        1,
        "{:?}",
        scanned.issues
    );
    let reference = search(&store, "Largedocumentneedle")[0].reference.clone();
    let mut options = root.scan_options;
    options.documents = sovra_core::file_index::DocumentBudget::LowerMemory;
    store.set_scan_options(&root.id, options).unwrap();
    assert_eq!(
        fixture.store().list().unwrap()[0].scan_options.documents,
        options.documents
    );
    let limited = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(limited.progress.skipped, 1);
    assert_eq!(limited.progress.failed, 0);
    assert!(
        limited
            .issues
            .iter()
            .any(|note| note.contains("64 MiB") && note.contains("Folder settings"))
    );
    assert!(store.read(&reference).is_err());
    options.documents = sovra_core::file_index::DocumentBudget::Standard;
    store.set_scan_options(&root.id, options).unwrap();
    assert_eq!(
        store
            .scan(&root.id, &Observer::default())
            .unwrap()
            .progress
            .reused,
        1
    );
    assert!(store.read(&reference).is_ok());
}

#[test]
fn indexed_documents_over_five_hundred_pages_or_slides_keep_the_last_text() {
    for (name, bytes, first, last) in [
        (
            "long.pdf",
            document(601, |page| {
                format!(
                    "Garden invoice page {}. uniquepage{}needle",
                    page + 1,
                    page + 1
                )
            }),
            "uniquepage1needle",
            "uniquepage601needle",
        ),
        (
            "slides.pptx",
            crate::format_tests::packages::presentation_with_slides(601),
            "marker1needle",
            "marker601needle",
        ),
    ] {
        assert!(bytes.len() < 1024 * 1024);
        let fixture = Fixture::new();
        std::fs::write(fixture.source().join(name), &bytes).unwrap();
        let store = fixture.store();
        let root = store.add_root(&fixture.source()).unwrap();
        let scanned = store.scan(&root.id, &Observer::default()).unwrap();
        assert_eq!(scanned.progress.failed, 0, "{name}: {:?}", scanned.issues);
        assert_eq!(
            search(&store, last).len(),
            1,
            "{name}: {:?}",
            scanned.issues
        );
        assert_eq!(search(&store, first).len(), 1);
        let reopened = fixture.store();
        assert_eq!(search(&reopened, last).len(), 1);
        assert_eq!(
            reopened
                .scan(&root.id, &Observer::default())
                .unwrap()
                .progress
                .reused,
            1
        );
        // Chat attachment packaging keeps its separate per-turn resource ceiling.
        assert!(crate::prepare_bytes("attachment", name, &bytes).is_err());
    }
}

#[test]
fn rich_document_text_uses_the_selected_budget_and_keeps_the_last_passage() {
    use sovra_core::file_index::DocumentBudget;
    use std::io::Cursor;
    let mut xml = String::from(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
    );
    let paragraph = "garden source context ".repeat(1024);
    for _ in 0..900 {
        xml.push_str("<w:p><w:r><w:t>");
        xml.push_str(&paragraph);
        xml.push_str("</w:t></w:r></w:p>");
    }
    xml.push_str("<w:p><w:r><w:t>Finaldocumenttextneedle completes the document.</w:t></w:r></w:p></w:body></w:document>");
    let mut input = zip::ZipArchive::new(Cursor::new(include_bytes!(
        "../../../tests/fixtures/invoice.docx"
    )))
    .unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..input.len() {
        let part = input.by_index(index).unwrap();
        if part.name().unwrap() == "word/document.xml" {
            output
                .start_file(
                    "word/document.xml",
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            output.write_all(xml.as_bytes()).unwrap();
        } else {
            output.raw_copy_file(part).unwrap();
        }
    }
    let bytes = output.finish().unwrap().into_inner();
    assert!((bytes.len() as u64) < DocumentBudget::LowerMemory.source_bytes());
    assert!(
        crate::prepare_bytes("attachment", "large.docx", &bytes)
            .map_or(true, |file| file.raw.is_some())
    );
    let fixture = Fixture::new();
    std::fs::write(fixture.source().join("large.docx"), bytes).unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let scanned = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(scanned.progress.failed, 0, "{:?}", scanned.issues);
    assert_eq!(search(&store, "Finaldocumenttextneedle").len(), 1);
    let mut options = root.scan_options;
    options.documents = DocumentBudget::LowerMemory;
    store.set_scan_options(&root.id, options).unwrap();
    let limited = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(limited.progress.failed, 0);
    assert_eq!(
        store.catalogue(&CatalogQuery::default()).unwrap().files[0].extraction,
        ExtractionState::MetadataOnly
    );
    assert_eq!(
        search(&store, "Finaldocumenttextneedle"),
        [] as [IndexPassage; 0]
    );
    assert!(
        limited
            .issues
            .iter()
            .any(|note| note.contains("Folder settings"))
    );
    options.documents = DocumentBudget::Standard;
    store.set_scan_options(&root.id, options).unwrap();
    assert_eq!(
        store
            .scan(&root.id, &Observer::default())
            .unwrap()
            .progress
            .reused,
        1
    );
    assert_eq!(search(&store, "Finaldocumenttextneedle").len(), 1);
}

#[test]
fn indexed_archives_keep_members_beyond_one_hundred() {
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for index in 0..150 {
        archive
            .start_file(
                format!("note{index}.txt"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive
            .write_all(format!("Archivemember{index}needle is readable source.").as_bytes())
            .unwrap();
    }
    let bytes = archive.finish().unwrap().into_inner();
    assert!(crate::prepare_bytes("attachment", "many.zip", &bytes).is_err());
    let fixture = Fixture::new();
    std::fs::write(fixture.source().join("many.zip"), bytes).unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    assert_eq!(
        store
            .scan(&root.id, &Observer::default())
            .unwrap()
            .progress
            .failed,
        0
    );
    assert_eq!(search(&store, "Archivemember0needle").len(), 1);
    assert_eq!(search(&store, "Archivemember149needle").len(), 1);
}

#[test]
fn cancellation_while_reading_a_large_document_keeps_the_previous_version() {
    // Readers poll cancellation every 64 KiB block; trip after ~64 polls (≈4 MiB) so the
    // request lands mid-file, as a person pressing Stop would.
    struct CancelOnRead(AtomicBool, AtomicU64);
    impl ScanObserver for CancelOnRead {
        fn cancellation(&self) -> Option<ScanCancellation> {
            if self.0.load(Ordering::Relaxed) && self.1.fetch_add(1, Ordering::Relaxed) >= 64 {
                Some(ScanCancellation::Requested)
            } else {
                None
            }
        }
        fn progress(&self, _: &IndexedRoot) {}
    }
    let fixture = Fixture::new();
    let path = fixture.source().join("large.pdf");
    std::fs::write(
        &path,
        document(1, |_| "Beforelargecancelneedle remains searchable.".into()),
    )
    .unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    std::fs::write(
        &path,
        document_with_padding(
            1,
            |_| "Afterlargecancelneedle is new content.".into(),
            21 * 1024 * 1024,
        ),
    )
    .unwrap();
    let cancelled = store
        .scan(
            &root.id,
            &CancelOnRead(AtomicBool::new(true), AtomicU64::new(0)),
        )
        .unwrap();
    assert_eq!(cancelled.status, IndexStatus::Cancelled);
    assert_eq!(cancelled.progress.failed, 0);
    assert!(cancelled.progress.bytes_read < 21 * 1024 * 1024);
    assert_eq!(search(&store, "Beforelargecancelneedle").len(), 1);
    assert_eq!(
        search(&store, "Afterlargecancelneedle"),
        [] as [IndexPassage; 0]
    );
    store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(search(&store, "Afterlargecancelneedle").len(), 1);
}

#[test]
fn indexed_scanned_pdfs_keep_metadata_without_the_twelve_picture_attachment_error() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.source().join("scan.pdf"),
        document(20, |_| String::new()),
    )
    .unwrap();
    std::fs::write(
        fixture.source().join("short.pdf"),
        document(1, |_| "shortneedle".into()),
    )
    .unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let scanned = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(scanned.progress.failed, 0, "{:?}", scanned.issues);
    assert_eq!(search(&store, "shortneedle").len(), 1);
    let inventory = store.catalogue(&CatalogQuery::default()).unwrap();
    let scan = inventory
        .files
        .iter()
        .find(|f| f.path == "scan.pdf")
        .unwrap();
    assert_eq!(scan.extraction, ExtractionState::MetadataOnly);
    assert_eq!(scan.first_passage, None);
    assert!(scan.note.as_ref().unwrap().contains("text layer"));
}
