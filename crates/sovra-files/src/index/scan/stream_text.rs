//! Large UTF-8 sources are indexed with fixed buffers, never attachment-sized excerpts.

use super::*;
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;
use std::io::{Seek as _, SeekFrom};

const BUFFER_BYTES: usize = 64 * 1024;

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
        let first = stream(file, before.len(), self.root, self.observer, |text| {
            visible |= !text.trim().is_empty();
            Ok(())
        })?;
        if self.observer.cancelled() {
            return Ok(true);
        }
        let Some(content_hash) = first.filter(|_| visible) else {
            file.seek(SeekFrom::Start(0)).map_err(|e| io_error(&e))?;
            return Ok(false);
        };
        let physical = identity(before);
        let file_id = self.file_id(path, &physical)?;
        let revision = "stream-utf8-v1";
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
        let note = "Complete UTF-8 source text indexed in blocks. Markup and structured files are searchable as source, without rendering or executing them.";
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
            let second = stream(file, before.len(), self.root, self.observer, |text| {
                for chunk in chunks(text) {
                    let chunk_id = digest(chunk.body.as_bytes());
                    tx.execute("INSERT INTO index_chunks(id,body) VALUES(?1,?2) ON CONFLICT(id) DO NOTHING", params![chunk_id, chunk.body]).map_err(|e| db_error(&e))?;
                    tx.execute("INSERT INTO index_version_chunks(version_id,ordinal,chunk_id,start_line,end_line) VALUES(?1,?2,?3,?4,?5)",
                        params![version, ordinal, chunk_id, line.saturating_add(chunk.start_line - 1), line.saturating_add(chunk.end_line - 1)]).map_err(|e| db_error(&e))?;
                    ordinal += 1;
                }
                line = line.saturating_add(
                    u32::try_from(text.bytes().filter(|b| *b == b'\n').count())
                        .expect("bounded block"),
                );
                Ok(())
            })?;
            if self.observer.cancelled() {
                return Ok(true);
            }
            if second.as_ref() != Some(&content_hash) {
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
            Some(note),
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

/// None means non-text, a truncated UTF-8 sequence or cancellation. No decoder
/// allocates for the whole source, including sources with a single enormous line.
fn stream(
    file: &mut impl std::io::Read,
    length: u64,
    root: &mut IndexedRoot,
    observer: &dyn ScanObserver,
    mut visit: impl FnMut(&str) -> Result<(), IndexError>,
) -> Result<Option<String>, IndexError> {
    let mut buffer = vec![0_u8; BUFFER_BYTES + 3];
    let mut carry = 0;
    let mut remaining = length;
    let mut hash = Sha256::new();
    let mut blocks = 0_u64;
    while remaining > 0 {
        if observer.cancelled() {
            return Ok(None);
        }
        let count = usize::try_from(remaining.min(BUFFER_BYTES as u64)).expect("bounded read");
        let read = file
            .read(&mut buffer[carry..carry + count])
            .map_err(|e| io_error(&e))?;
        if read == 0 {
            return Err(changed());
        }
        if blocks == 0 && buffer[..read].starts_with(b"%PDF-") {
            return Ok(None);
        }
        hash.update(&buffer[carry..carry + read]);
        root.progress.bytes_read = root.progress.bytes_read.saturating_add(read as u64);
        remaining -= read as u64;
        let used = carry + read;
        let valid = match std::str::from_utf8(&buffer[..used]) {
            Ok(_) => used,
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => return Ok(None),
        };
        let text = std::str::from_utf8(&buffer[..valid]).expect("validated UTF-8 prefix");
        if text.chars().any(|ch| {
            ch.is_control()
                && !matches!(
                    ch,
                    '\n' | '\r' | '\t' | '\u{000c}' | '\u{001b}' | '\u{0008}'
                )
        }) {
            return Ok(None);
        }
        visit(text)?;
        buffer.copy_within(valid..used, 0);
        carry = used - valid;
        blocks += 1;
        if blocks.is_multiple_of(16) {
            observer.progress(root);
        }
    }
    if carry != 0 {
        return Ok(None);
    }
    if file.read(&mut buffer[..1]).map_err(|e| io_error(&e))? != 0 {
        return Err(changed());
    }
    let mut encoded = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(encoded, "{byte:02x}").expect("writing to a string");
    }
    Ok(Some(encoded))
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
        assert_eq!(actual, Some(digest(text.as_bytes())));
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
