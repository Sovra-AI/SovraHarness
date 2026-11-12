//! Large text sources are indexed with fixed buffers, never attachment-sized excerpts.

use super::*;
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;
use std::io::{Seek as _, SeekFrom};

mod legacy;
mod unicode;
use unicode::{Decoder, Encoding};

const BUFFER_BYTES: usize = 64 * 1024;
const PREFIX_BYTES: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
struct Source {
    digest: String,
    encoding: Encoding,
    inferred: bool,
}

impl Scan<'_> {
    /// Validate and hash before publishing anything. A second hash fences changes
    /// between passes; the transaction rolls back every chunk on cancellation.
    pub(super) fn stream_text(
        &mut self,
        file: &mut cap_std::fs::File,
        path: &str,
        before: &Metadata,
    ) -> Result<bool, IndexError> {
        let mut visible = false;
        let mut first = stream(file, before.len(), self.root, self.observer, |text| {
            visible |= !text.trim().is_empty();
            Ok(())
        })?;
        if first.is_none()
            && !self.observer.cancelled()
            && let Some(encoding) = legacy::detect(file, before.len(), self.root, self.observer)?
        {
            file.seek(SeekFrom::Start(0)).map_err(|e| io_error(&e))?;
            visible = false;
            first = stream_encoded(
                file,
                before.len(),
                self.root,
                self.observer,
                Some(encoding),
                |text| {
                    visible |= !text.trim().is_empty();
                    Ok(())
                },
            )?;
        }
        if self.observer.cancelled() {
            return Ok(true);
        }
        let Some(source) = first.filter(|_| visible) else {
            file.seek(SeekFrom::Start(0)).map_err(|e| io_error(&e))?;
            return Ok(false);
        };
        let content_hash = &source.digest;
        let physical = identity(before);
        let file_id = self.file_id(path, &physical)?;
        let revision = "stream-text-v4";
        let version = digest(format!("{revision}\0{file_id}\0{content_hash}").as_bytes());
        let known: bool = self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM index_versions WHERE id=?1)",
                [&version],
                |row| row.get(0),
            )
            .map_err(|e| db_error(&e))?;
        let tx = self.db.transaction().map_err(|e| db_error(&e))?;
        tx.execute("INSERT INTO index_files(id,root_id,path,identity,current_version,generation,available) VALUES(?1,?2,?3,?4,?5,?6,1) ON CONFLICT(id) DO UPDATE SET path=excluded.path,identity=excluded.identity,current_version=excluded.current_version,generation=excluded.generation,available=1",
            params![file_id, self.root.id, path, physical, version, self.generation]).map_err(|e| db_error(&e))?;
        let note = format!(
            "Complete {} source text indexed in blocks. {}Markup and structured files are searchable as source, without rendering or executing them.",
            source.encoding.label(),
            if source.inferred {
                "Encoding inferred from byte patterns; check the saved text if characters look wrong. "
            } else {
                ""
            }
        );
        if !known {
            let modified = before
                .modified()
                .ok()
                .and_then(|t| t.into_std().duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|t| i64::try_from(t.as_secs()).ok());
            tx.execute("INSERT INTO index_versions(id,file_id,digest,mime,note,observed,modified,extractor_revision,generated_by) VALUES(?1,?2,?3,'text/plain',?4,?5,?6,?7,?8)",
                params![version, file_id, content_hash, note, i64::try_from(now()).expect("Unix seconds fit i64"), modified, revision, self.activity]).map_err(|e| db_error(&e))?;
            file.seek(SeekFrom::Start(0)).map_err(|e| io_error(&e))?;
            let mut ordinal = 0_i64;
            let mut line = 1_u32;
            let mut carry = None;
            let legacy = match source.encoding {
                Encoding::Legacy(encoding) => Some(encoding),
                Encoding::Utf8 | Encoding::Utf16 { .. } | Encoding::Utf32 { .. } => None,
            };
            let second = stream_encoded(
                file,
                before.len(),
                self.root,
                self.observer,
                legacy,
                |text| {
                    for chunk in chunks(text) {
                        let stored = sovra_core::file_index::contextual_passage(
                            path,
                            text,
                            chunk.start_line,
                            chunk.body,
                            carry.as_deref(),
                        );
                        let chunk_id = digest(stored.as_bytes());
                        tx.execute("INSERT INTO index_chunks(id,body) VALUES(?1,?2) ON CONFLICT(id) DO NOTHING", params![chunk_id, stored]).map_err(|e| db_error(&e))?;
                        tx.execute("INSERT INTO index_version_chunks(version_id,ordinal,chunk_id,start_line,end_line) VALUES(?1,?2,?3,?4,?5)",
                        params![version, ordinal, chunk_id, line.saturating_add(chunk.start_line - 1), line.saturating_add(chunk.end_line - 1)]).map_err(|e| db_error(&e))?;
                        ordinal += 1;
                    }
                    if let Some(section) = sovra_core::file_index::carried_section(text) {
                        carry = Some(section);
                    }
                    line = line.saturating_add(
                        u32::try_from(text.bytes().filter(|b| *b == b'\n').count())
                            .expect("bounded block"),
                    );
                    Ok(())
                },
            )?;
            if self.observer.cancelled() {
                return Ok(true);
            }
            if second.as_ref() != Some(&source) {
                return Err(changed());
            }
        }
        let after = file.metadata().map_err(|e| io_error(&e))?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err(changed());
        }
        if self.observer.cancelled() {
            return Ok(true);
        }
        save_metadata(
            &tx,
            &file_id,
            path,
            before,
            ExtractionState::Text,
            Some(&note),
        )?;
        super::super::history::observe(&tx, &file_id, self.activity)?;
        tx.commit().map_err(|e| db_error(&e))?;
        if known {
            self.root.progress.reused += 1;
        } else {
            self.root.progress.indexed += 1;
        }
        Ok(true)
    }
}

fn changed() -> IndexError {
    IndexError("The file changed while being read. Refresh the folder to retry.".into())
}

/// None means non-text, invalid/incomplete Unicode or cancellation. Source bytes
/// are hashed before decoding, so changes in encoding are distinct saved versions.
fn stream(
    file: &mut impl std::io::Read,
    length: u64,
    root: &mut IndexedRoot,
    observer: &dyn ScanObserver,
    visit: impl FnMut(&str) -> Result<(), IndexError>,
) -> Result<Option<Source>, IndexError> {
    stream_encoded(file, length, root, observer, None, visit)
}

fn stream_encoded(
    file: &mut impl std::io::Read,
    length: u64,
    root: &mut IndexedRoot,
    observer: &dyn ScanObserver,
    encoding: Option<&'static encoding_rs::Encoding>,
    mut visit: impl FnMut(&str) -> Result<(), IndexError>,
) -> Result<Option<Source>, IndexError> {
    if observer.cancelled() {
        return Ok(None);
    }
    let mut buffer = vec![0_u8; BUFFER_BYTES + 3];
    let prefix = usize::try_from(length.min(PREFIX_BYTES as u64)).expect("bounded prefix");
    file.read_exact(&mut buffer[..prefix])
        .map_err(|e| io_error(&e))?;
    root.progress.bytes_read = root.progress.bytes_read.saturating_add(prefix as u64);
    if buffer[..prefix].starts_with(b"%PDF-") {
        return Ok(None);
    }
    let mut hash = Sha256::new();
    hash.update(&buffer[..prefix]);
    let (mut decoder, bom) = encoding.map_or_else(
        || Decoder::start(&buffer[..prefix]),
        |encoding| (Decoder::legacy(encoding), 0),
    );
    let mut escape = legacy::EscapeHint::default();
    buffer.copy_within(bom..prefix, 0);
    let mut used = prefix - bom;
    let mut remaining = length - prefix as u64;
    let mut output = String::with_capacity(BUFFER_BYTES * 2);
    let mut blocks = 0_u64;
    loop {
        if observer.cancelled() {
            return Ok(None);
        }
        if bom == 0 && decoder.encoding == Encoding::Utf8 && escape.feed(&buffer[..used]) {
            return Ok(None);
        }
        let Some(valid) = decoder.decode(&buffer[..used], &mut output, remaining == 0) else {
            return Ok(None);
        };
        if !crate::text::printable(&output) {
            return Ok(None);
        }
        if !output.is_empty() {
            visit(&output)?;
        }
        let carry = used - valid;
        if remaining == 0 {
            if carry != 0 || !decoder.finished() {
                return Ok(None);
            }
            break;
        }
        buffer.copy_within(valid..used, 0);
        let count = usize::try_from(remaining.min(BUFFER_BYTES as u64)).expect("bounded read");
        let read = file
            .read(&mut buffer[carry..carry + count])
            .map_err(|e| io_error(&e))?;
        if read == 0 {
            return Err(changed());
        }
        hash.update(&buffer[carry..carry + read]);
        root.progress.bytes_read = root.progress.bytes_read.saturating_add(read as u64);
        remaining -= read as u64;
        used = carry + read;
        blocks += 1;
        if blocks.is_multiple_of(16) {
            observer.progress(root);
        }
    }
    if file.read(&mut buffer[..1]).map_err(|e| io_error(&e))? != 0 {
        return Err(changed());
    }
    let mut encoded = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(encoded, "{byte:02x}").expect("writing to a string");
    }
    Ok(Some(Source {
        digest: encoded,
        encoding: decoder.encoding,
        inferred: decoder.inferred,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::file_index::{IndexProgress, IndexStatus, ScanCancellation};

    struct Observer;
    impl ScanObserver for Observer {
        fn cancellation(&self) -> Option<ScanCancellation> {
            None
        }
        fn progress(&self, _: &IndexedRoot) {}
    }

    fn root() -> IndexedRoot {
        IndexedRoot {
            id: "test".into(),
            path: "/test".into(),
            status: IndexStatus::Scanning,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: None,
            auto_refresh: false,
            scan_options: sovra_core::file_index::ScanOptions::default(),
        }
    }

    #[test]
    fn legacy_decoders_preserve_split_characters_and_reject_incomplete_final_sequences() {
        struct Fragments<'a>(&'a [u8], usize);
        impl std::io::Read for Fragments<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let count = out.len().min(self.0.len()).min(self.1);
                out[..count].copy_from_slice(&self.0[..count]);
                self.0 = &self.0[count..];
                Ok(count)
            }
        }
        for (encoding, line) in [
            (encoding_rs::WINDOWS_1252, "Café français — coût €93.\r\n"),
            (
                encoding_rs::SHIFT_JIS,
                "日本語の請求書、価格を確認します。\r\n",
            ),
            (
                encoding_rs::ISO_2022_JP,
                "日本語の請求書、価格を確認します。\r\n",
            ),
            (encoding_rs::GBK, "这是一份中文发票。检查鲜花的价格。\r\n"),
        ] {
            let text = format!("{}{}", "header ".repeat(600), line.repeat(300));
            let (bytes, _, errors) = encoding.encode(&text);
            assert!(!errors);
            for fragment in [1, 3, BUFFER_BYTES] {
                let mut seen = String::new();
                let source = stream_encoded(
                    &mut Fragments(&bytes, fragment),
                    bytes.len() as u64,
                    &mut root(),
                    &Observer,
                    Some(encoding),
                    |part| {
                        assert!(part.len() <= BUFFER_BYTES * 4 + 32);
                        seen.push_str(part);
                        Ok(())
                    },
                )
                .unwrap()
                .unwrap();
                assert_eq!(seen, text, "{}, fragment={fragment}", encoding.name());
                assert_eq!(source.digest, digest(&bytes));
                assert_eq!(source.encoding, Encoding::Legacy(encoding));
                assert!(source.inferred);
            }
        }
        for (encoding, ending) in [
            (encoding_rs::SHIFT_JIS, b"\x82".as_slice()),
            (encoding_rs::ISO_2022_JP, b"\x1b$".as_slice()),
            (encoding_rs::GBK, b"\x81".as_slice()),
        ] {
            let mut bytes = b"valid header ".repeat(400);
            bytes.extend_from_slice(ending);
            assert!(
                stream_encoded(
                    &mut Fragments(&bytes, 1),
                    bytes.len() as u64,
                    &mut root(),
                    &Observer,
                    Some(encoding),
                    |_| Ok(())
                )
                .unwrap()
                .is_none()
            );
        }
    }

    #[test]
    fn japanese_escape_detection_survives_prefix_and_block_boundaries() {
        for prefix in [
            PREFIX_BYTES - 1,
            PREFIX_BYTES - 2,
            PREFIX_BYTES + BUFFER_BYTES - 1,
        ] {
            let text = format!("{}{}", "a".repeat(prefix), "日本語の請求書。".repeat(200));
            let (bytes, _, errors) = encoding_rs::ISO_2022_JP.encode(&text);
            assert!(!errors);
            assert!(
                stream(
                    &mut bytes.as_ref(),
                    bytes.len() as u64,
                    &mut root(),
                    &Observer,
                    |_| Ok(())
                )
                .unwrap()
                .is_none(),
                "escape bytes must not be published as plain ASCII"
            );
            let inferred = legacy::detect(
                &mut std::io::Cursor::new(&bytes),
                bytes.len() as u64,
                &mut root(),
                &Observer,
            )
            .unwrap();
            assert_eq!(inferred, Some(encoding_rs::ISO_2022_JP));
        }
    }

    #[test]
    fn legacy_detection_does_not_reinterpret_binary_or_broken_explicit_unicode() {
        for bytes in [
            b"%PDF-1.7\n".as_slice(),
            b"valid\0binary",
            b"\xff\xfea",
            b"\xef\xbb\xbf\xff",
        ] {
            assert!(
                legacy::detect(
                    &mut std::io::Cursor::new(bytes),
                    bytes.len() as u64,
                    &mut root(),
                    &Observer
                )
                .unwrap()
                .is_none()
            );
        }
    }

    #[test]
    fn unicode_units_and_surrogate_pairs_survive_fragmented_reads_without_replacement() {
        struct Fragments<'a>(&'a [u8], usize);
        impl std::io::Read for Fragments<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let count = out.len().min(self.0.len()).min(self.1);
                out[..count].copy_from_slice(&self.0[..count]);
                self.0 = &self.0[count..];
                Ok(count)
            }
        }
        let text = format!(
            "{}🦀京都\n{}",
            "a".repeat(PREFIX_BYTES / 2 - 2),
            "京都🦀\r\n".repeat(900)
        );
        for encoding in [
            Encoding::Utf8,
            Encoding::Utf16 { little: true },
            Encoding::Utf16 { little: false },
            Encoding::Utf32 { little: true },
            Encoding::Utf32 { little: false },
        ] {
            let full = format!("\u{feff}{text}");
            let bytes: Vec<u8> = match encoding {
                Encoding::Utf8 => full.into_bytes(),
                Encoding::Utf16 { little } => full
                    .encode_utf16()
                    .flat_map(|unit| {
                        if little {
                            unit.to_le_bytes()
                        } else {
                            unit.to_be_bytes()
                        }
                    })
                    .collect(),
                Encoding::Utf32 { little } => full
                    .chars()
                    .flat_map(|ch| {
                        if little {
                            u32::from(ch).to_le_bytes()
                        } else {
                            u32::from(ch).to_be_bytes()
                        }
                    })
                    .collect(),
                Encoding::Legacy(_) => unreachable!("this fixture contains only Unicode encodings"),
            };
            for fragment in [1, 3, BUFFER_BYTES] {
                let mut seen = String::new();
                let mut progress = root();
                let source = stream(
                    &mut Fragments(&bytes, fragment),
                    bytes.len() as u64,
                    &mut progress,
                    &Observer,
                    |part| {
                        assert!(part.len() <= BUFFER_BYTES * 2);
                        seen.push_str(part);
                        Ok(())
                    },
                )
                .unwrap()
                .unwrap();
                assert_eq!(seen, text, "{encoding:?}, fragment={fragment}");
                assert_eq!(source.digest, digest(&bytes));
                assert_eq!(source.encoding, encoding);
                assert!(!source.inferred);
                assert_eq!(progress.progress.bytes_read, bytes.len() as u64);
            }
        }
    }

    #[test]
    fn malformed_wide_text_is_not_published_as_replacement_characters() {
        for bytes in [
            vec![0xff, 0xfe, 0x00, 0xd8],
            vec![0xff, 0xfe, 0x00, 0xdc],
            vec![0xff, 0xfe, 0x00, 0xd8, b'a', 0],
            vec![0xff, 0xfe, b'a'],
            vec![0xfe, 0xff, 0xdc, 0x00],
            vec![0xff, 0xfe, 0, 0, 0, 0, 0x11, 0],
            vec![0xff, 0xfe, 0, 0, 0, 0xd8, 0, 0],
            vec![0, 0, 0xfe, 0xff, 0, 0, 0],
        ] {
            assert_eq!(
                stream(
                    &mut bytes.as_slice(),
                    bytes.len() as u64,
                    &mut root(),
                    &Observer,
                    |_| Ok(())
                )
                .unwrap(),
                None,
                "{bytes:?}"
            );
        }
    }

    #[test]
    fn stream_preserves_split_unicode_and_long_lines_and_rejects_binary_or_changed_lengths() {
        let text = format!(
            "{}京都☕\n{}",
            "a".repeat(BUFFER_BYTES - 1),
            "b".repeat(BUFFER_BYTES * 2)
        );
        let mut seen = String::new();
        let actual = stream(
            &mut std::io::Cursor::new(text.as_bytes()),
            text.len() as u64,
            &mut root(),
            &Observer,
            |part| {
                assert!(part.len() <= BUFFER_BYTES + 3);
                seen.push_str(part);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            actual.map(|source| source.digest),
            Some(digest(text.as_bytes()))
        );
        assert_eq!(seen, text);
        for bytes in [
            b"%PDF-1.7\n".as_slice(),
            b"valid\0binary",
            b"valid\xffinvalid",
            b"truncated\xe4\xba",
        ] {
            assert_eq!(
                stream(
                    &mut std::io::Cursor::new(bytes),
                    bytes.len() as u64,
                    &mut root(),
                    &Observer,
                    |_| Ok(())
                )
                .unwrap(),
                None
            );
        }
        for length in [2, 4] {
            assert!(
                stream(
                    &mut std::io::Cursor::new(b"abc"),
                    length,
                    &mut root(),
                    &Observer,
                    |_| Ok(())
                )
                .is_err()
            );
        }
    }
}
