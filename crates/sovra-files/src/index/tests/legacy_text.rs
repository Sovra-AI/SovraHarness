//! Older text encodings must not inherit attachment-sized excerpts.

use super::*;
use encoding_rs::{GBK, ISO_2022_JP, SHIFT_JIS, WINDOWS_1252};

#[test]
fn large_legacy_sources_keep_the_last_passage_and_label_the_inferred_encoding() {
    for (encoding, line, minimum) in [
        (
            WINDOWS_1252,
            "Café français, crème et gâteau — coût €93.\r\n",
            usize::try_from(sovra_core::files::MAX_FILE_BYTES).unwrap() + 1,
        ),
        (
            SHIFT_JIS,
            "日本語の請求書。庭の花とお茶の価格を確認します。\r\n",
            3 * 1024 * 1024,
        ),
        (
            ISO_2022_JP,
            "日本語の請求書。庭の花とお茶の価格を確認します。\r\n",
            3 * 1024 * 1024,
        ),
        (
            GBK,
            "这是一份中文发票。我们需要检查花园里的鲜花和茶叶价格。\r\n",
            3 * 1024 * 1024,
        ),
    ] {
        let fixture = Fixture::new();
        // Do not infer UTF-8 merely because a long header is ASCII.
        let mut bytes = b"Plain header before the document.\r\n".repeat(4096);
        let (encoded, _, errors) = encoding.encode(line);
        assert!(!errors);
        let copies = minimum / encoded.len() + 1;
        bytes.extend(encoded.repeat(copies));
        bytes.extend(b"\r\nlegacyendingneedle\r\n");
        std::fs::write(fixture.source().join("source.txt"), &bytes).unwrap();
        let store = fixture.store();
        let root = store.add_root(&fixture.source()).unwrap();
        let scanned = store.scan(&root.id, &Observer::default()).unwrap();
        assert_eq!(
            scanned.progress.failed,
            0,
            "{}: {:?}",
            encoding.name(),
            scanned.issues
        );
        let hits = search(&store, "legacyendingneedle");
        assert_eq!(hits.len(), 1, "{}: {:?}", encoding.name(), scanned.issues);
        let db = store.connect().unwrap();
        let note: String = db
            .query_row("SELECT note FROM index_versions", [], |row| row.get(0))
            .unwrap();
        assert!(note.contains(encoding.name()), "{note}");
        assert!(note.contains("inferred"), "{note}");
        let first_body: String = db
            .query_row(
                "SELECT body FROM index_chunks WHERE instr(body,?1)>0 LIMIT 1",
                [line.trim()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(first_body.contains(line.trim()));
        drop(db);
        let reopened = fixture.store();
        assert_eq!(search(&reopened, "legacyendingneedle").len(), 1);
        assert_eq!(
            reopened
                .scan(&root.id, &Observer::default())
                .unwrap()
                .progress
                .reused,
            1
        );
    }
}

#[test]
fn legacy_expansion_enters_streaming_before_the_attachment_output_limit() {
    let fixture = Fixture::new();
    let line = "ใบแจ้งหนี้ภาษาไทย ตรวจสอบราคาดอกไม้และชา\r\n";
    let (encoded_line, _, errors) = encoding_rs::WINDOWS_874.encode(line);
    assert!(!errors);
    let copies = (sovra_core::files::MAX_DOCUMENT_BYTES / 2 - 128) / encoded_line.len();
    let text = format!("{}legacyexpansionneedle", line.repeat(copies));
    let (bytes, _, errors) = encoding_rs::WINDOWS_874.encode(&text);
    assert!(!errors);
    assert!(bytes.len() < sovra_core::files::MAX_DOCUMENT_BYTES / 2);
    assert!(text.len() > sovra_core::files::MAX_DOCUMENT_BYTES);
    std::fs::write(fixture.source().join("expanded.txt"), bytes).unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    let scanned = store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(
        search(&store, "legacyexpansionneedle").len(),
        1,
        "{:?}",
        scanned.issues
    );
    let note: String = store
        .connect()
        .unwrap()
        .query_row("SELECT note FROM index_versions", [], |row| row.get(0))
        .unwrap();
    assert!(note.contains("indexed in blocks"), "{note}");
    assert!(note.contains("windows-874"), "{note}");
    let matched: bool = store
        .connect()
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM index_chunks WHERE instr(body,?1)>0)",
            [line.trim()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(matched);
}

#[test]
fn cancelling_legacy_detection_validation_or_writes_keeps_the_previous_snapshot() {
    struct CancelAfter(u64, AtomicBool);
    impl ScanObserver for CancelAfter {
        fn cancellation(&self) -> Option<ScanCancellation> {
            self.1
                .load(Ordering::Relaxed)
                .then_some(ScanCancellation::Requested)
        }
        fn progress(&self, root: &IndexedRoot) {
            if root.progress.bytes_read > self.0 {
                self.1.store(true, Ordering::Relaxed);
            }
        }
    }
    let line = "Café français, crème et gâteau — coût €93.\r\n";
    let replacement = format!("{}legacyreplacementneedle", line.repeat(100_000));
    let (bytes, _, errors) = WINDOWS_1252.encode(&replacement);
    assert!(!errors);
    let length = bytes.len() as u64;
    for threshold in [length / 2, length + length / 2, 2 * length + length / 2] {
        let fixture = Fixture::new();
        fixture.write(
            "notes.txt",
            "legacyoriginalneedle stays until a full replacement.",
        );
        let store = fixture.store();
        let root = store.add_root(&fixture.source()).unwrap();
        store.scan(&root.id, &Observer::default()).unwrap();
        std::fs::write(fixture.source().join("notes.txt"), &bytes).unwrap();
        let scanned = store
            .scan(&root.id, &CancelAfter(threshold, AtomicBool::new(false)))
            .unwrap();
        assert_eq!(scanned.status, IndexStatus::Cancelled);
        assert_eq!(search(&store, "legacyoriginalneedle").len(), 1);
        assert_eq!(
            search(&store, "legacyreplacementneedle"),
            [] as [IndexPassage; 0]
        );
        let versions: i64 = store
            .connect()
            .unwrap()
            .query_row("SELECT count(*) FROM index_versions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(versions, 1);
        store.scan(&root.id, &Observer::default()).unwrap();
        assert_eq!(search(&store, "legacyreplacementneedle").len(), 1);
    }
}
