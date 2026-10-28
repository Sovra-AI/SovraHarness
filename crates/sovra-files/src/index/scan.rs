//! Reconcile a cancellable walk with bounded per-file work; only a completed walk can retire missing paths.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsSyncExt as _};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use rusqlite::{Connection, OptionalExtension as _, params};
use sovra_core::file_index::{
    ExtractionState, IndexError, IndexedRoot, MAX_INDEX_ISSUES, NoticeKind, ScanObserver,
};

use super::{db_error, digest, io_error, now, save_root};

mod stream_text;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Completion {
    Complete,
    Incomplete,
}

pub(super) fn run(
    db: &mut Connection,
    root: &mut IndexedRoot,
    storage: &Path,
    observer: &dyn ScanObserver,
    activity: i64,
) -> Result<Completion, IndexError> {
    let dir = Dir::open_ambient_dir(&root.path, cap_std::ambient_authority())
        .map_err(|e| io_error(&e))?;
    let expected: String = db
        .query_row(
            "SELECT identity FROM index_roots WHERE id=?1",
            [&root.id],
            |r| r.get(0),
        )
        .map_err(|error| db_error(&error))?;
    if identity(&dir.dir_metadata().map_err(|e| io_error(&e))?) != expected {
        return Err(IndexError(
            "This folder was replaced or moved. Remove its old index and choose the folder again."
                .into(),
        ));
    }
    db.execute(
        "UPDATE index_roots SET generation=generation+1 WHERE id=?1",
        [&root.id],
    )
    .map_err(|error| db_error(&error))?;
    let generation: i64 = db
        .query_row(
            "SELECT generation FROM index_roots WHERE id=?1",
            [&root.id],
            |r| r.get(0),
        )
        .map_err(|error| db_error(&error))?;
    let excluded = std::fs::canonicalize(storage)
        .ok()
        .and_then(|p| p.strip_prefix(&root.path).ok().map(Path::to_owned));
    let mut scan = Scan {
        activity,
        db,
        root,
        observer,
        generation,
        excluded,
        notices: Vec::new(),
    };
    let walked = scan.walk(dir);
    scan.flush_notices()?;
    let completion = walked?;
    if observer.cancelled() || completion == Completion::Incomplete {
        return Ok(completion);
    }
    // Failed files retain observed metadata but revoke readable text. A traversal
    // error returns before this point, preserving the unvisited subtree.
    let tx = scan.db.transaction().map_err(|e| db_error(&e))?;
    let missing = {
        let mut statement = tx
            .prepare(
                "SELECT id FROM index_files WHERE root_id=?1 AND generation<>?2 AND available=1",
            )
            .map_err(|e| db_error(&e))?;
        statement
            .query_map(params![scan.root.id, generation], |r| r.get::<_, String>(0))
            .map_err(|e| db_error(&e))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| db_error(&e))?
    };
    for id in &missing {
        if observer.cancelled() {
            return Ok(Completion::Incomplete);
        }
        tx.execute(
            "UPDATE index_files SET available=0,observed=?2 WHERE id=?1",
            params![id, i64::try_from(now()).expect("Unix seconds fit i64")],
        )
        .map_err(|e| db_error(&e))?;
        super::history::observe(&tx, id, activity)?;
    }
    tx.commit().map_err(|e| db_error(&e))?;
    scan.root.progress.removed = missing.len() as u64;
    Ok(Completion::Complete)
}

struct Scan<'a> {
    activity: i64,
    db: &'a mut Connection,
    root: &'a mut IndexedRoot,
    observer: &'a dyn ScanObserver,
    generation: i64,
    excluded: Option<PathBuf>,
    notices: Vec<super::report::PendingNotice>,
}

impl Scan<'_> {
    fn notice(&mut self, kind: NoticeKind, path: &str, message: &str) -> Result<(), IndexError> {
        issue(self.root, format!("{path}: {message}"));
        self.notices.push(super::report::PendingNotice {
            kind,
            path: path.into(),
            message: message.into(),
        });
        if self.notices.len() >= 64 {
            self.flush_notices()?;
        }
        Ok(())
    }

    fn flush_notices(&mut self) -> Result<(), IndexError> {
        super::report::append(self.db, &self.root.id, &self.notices)?;
        self.notices.clear();
        Ok(())
    }

    fn walk(&mut self, dir: Dir) -> Result<Completion, IndexError> {
        // An explicit stack avoids a call-stack depth cutoff. Each directory is
        // held as a capability; only the current branch has open handles.
        let mut stack = vec![(
            dir.entries().map_err(|e| io_error(&e))?,
            dir,
            PathBuf::new(),
        )];
        let mut unread_entries = 0_u64;
        while let Some((entries, dir, relative)) = stack.last_mut() {
            let Some(entry) = entries.next() else {
                stack.pop();
                continue;
            };
            if self.root.progress.visited > 0 && self.root.progress.visited.is_multiple_of(32) {
                self.flush_notices()?;
                save_root(self.db, self.root)?;
                self.observer.progress(self.root);
            }
            if self.observer.cancelled() {
                return Ok(Completion::Incomplete);
            }
            self.root.progress.visited += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    self.traversal_issue(relative, &error)?;
                    unread_entries += 1;
                    // An iterator error leaves the rest of this directory
                    // uncertain. Continue from its parent rather than polling
                    // a broken iterator or abandoning other branches.
                    stack.pop();
                    continue;
                }
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                self.root.progress.skipped += 1;
                let escaped: String = relative
                    .join(&name)
                    .as_os_str()
                    .as_encoded_bytes()
                    .iter()
                    .copied()
                    .flat_map(std::ascii::escape_default)
                    .map(char::from)
                    .collect();
                self.notice(NoticeKind::Skipped, &escaped, "Filename is not valid Unicode; its path is shown with escapes. Rename it to include it in the index.")?;
                continue;
            };
            let path = relative.join(name);
            let excluded = if self.excluded.as_ref().is_some_and(|p| path == *p) {
                Some("Sovra's own index data")
            } else {
                self.root.scan_options.exclusion_reason(name)
            };
            if let Some(reason) = excluded {
                self.root.progress.skipped += 1;
                self.notice(NoticeKind::Skipped, &path.to_string_lossy(), reason)?;
                continue;
            }
            let metadata = match dir.symlink_metadata(name) {
                Ok(metadata) => metadata,
                Err(error) => {
                    self.traversal_issue(&path, &error)?;
                    unread_entries += 1;
                    continue;
                }
            };
            if metadata.is_symlink() {
                self.root.progress.skipped += 1;
                self.notice(
                    NoticeKind::Skipped,
                    &path.to_string_lossy(),
                    "Symbolic link; add its destination folder directly to include it.",
                )?;
            } else if metadata.is_dir() {
                // Every component is opened relative to a held capability and
                // rejects symlinks, including a link swapped in during traversal.
                match dir
                    .open_dir_nofollow(name)
                    .and_then(|child| child.entries().map(|entries| (entries, child)))
                {
                    Ok((entries, child)) => stack.push((entries, child, path)),
                    Err(error) => {
                        self.traversal_issue(&path, &error)?;
                        unread_entries += 1;
                    }
                }
            } else if metadata.is_file() {
                let relative = path
                    .to_str()
                    .ok_or_else(|| IndexError("Invalid filename.".into()))?
                    .replace(std::path::MAIN_SEPARATOR, "/");
                if let Err(error) = self.file(dir, name, &relative) {
                    // Cancellation is not a failed replacement: retain the
                    // previous version and never retire the unvisited remainder.
                    if self.observer.cancelled() {
                        return Ok(Completion::Incomplete);
                    }
                    self.root.progress.failed += 1;
                    self.notice(NoticeKind::Failed, &relative, &error.to_string())?;
                    self.metadata_only(
                        &relative,
                        &metadata,
                        ExtractionState::Failed,
                        &error.to_string(),
                    )?;
                }
            } else {
                self.root.progress.skipped += 1;
                self.notice(
                    NoticeKind::Skipped,
                    &path.to_string_lossy(),
                    "Not a regular file or folder.",
                )?;
            }
        }
        if unread_entries == 0 {
            Ok(Completion::Complete)
        } else {
            // A partial walk cannot prove a previously indexed path is gone.
            // Keep the aggregate guidance out of the persisted failure rows:
            // each affected location has already been counted and reported.
            issue(
                self.root,
                format!(
                    "Scan incomplete: {unread_entries} folders or entries could not be read. Accessible files were updated. Older entries were kept because this scan cannot confirm which files are missing. Check the scan report, fix access, then refresh to retry."
                ),
            );
            Ok(Completion::Incomplete)
        }
    }

    fn traversal_issue(&mut self, path: &Path, error: &std::io::Error) -> Result<(), IndexError> {
        let reason = match error.kind() {
            std::io::ErrorKind::PermissionDenied => "Access denied. Check folder permissions and allow Sovra to read it, then refresh to retry.".into(),
            std::io::ErrorKind::NotFound => "This entry disappeared while scanning. Refresh to check the folder again.".into(),
            kind => format!("This folder or entry could not be read ({kind}). Check access and availability, then refresh to retry."),
        };
        self.root.progress.failed += 1;
        let label = if path.as_os_str().is_empty() {
            ".".into()
        } else {
            path.to_string_lossy()
        };
        self.notice(NoticeKind::Failed, &label, &reason)
    }

    fn metadata_only(
        &mut self,
        path: &str,
        metadata: &Metadata,
        state: ExtractionState,
        note: &str,
    ) -> Result<(), IndexError> {
        let physical = identity(metadata);
        let id = self.file_id(path, &physical)?;
        let tx = self.db.transaction().map_err(|e| db_error(&e))?;
        tx.execute("INSERT INTO index_files(id,root_id,path,identity,current_version,generation,available) VALUES(?1,?2,?3,?4,NULL,?5,1) ON CONFLICT(id) DO UPDATE SET path=excluded.path,identity=excluded.identity,current_version=NULL,generation=excluded.generation,available=1",
            params![id,self.root.id,path,physical,self.generation]).map_err(|e| db_error(&e))?;
        save_metadata(&tx, &id, path, metadata, state, Some(note))?;
        super::history::observe(&tx, &id, self.activity)?;
        tx.commit().map_err(|e| db_error(&e))
    }

    fn file(&mut self, dir: &Dir, name: &str, path: &str) -> Result<(), IndexError> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No).nonblock(true);
        let mut file = dir.open_with(name, &options).map_err(|e| io_error(&e))?;
        let before = file.metadata().map_err(|e| io_error(&e))?;
        if !before.is_file() {
            return Err(IndexError("Only regular files can be indexed.".into()));
        }
        // Legacy single-byte text can triple when decoded to UTF-8. Enter the bounded reader
        // before the attachment decoder's output budget could reject it.
        if before.len() >= (sovra_core::files::MAX_DOCUMENT_BYTES / 3) as u64
            && self.stream_text(&mut file, path, &before)?
        {
            return Ok(());
        }
        let budget = self.root.scan_options.documents;
        let source_limit = budget.source_bytes();
        if before.len() > source_limit {
            let note = format!(
                "This file exceeds the folder's {} MiB document reading budget. Its filename and size are searchable; its contents have not been read. Open Folder settings → Document reading to choose a larger budget, or split the document. Streaming source text has no file-size cutoff.",
                source_limit / (1024 * 1024)
            );
            self.metadata_only(path, &before, ExtractionState::MetadataOnly, &note)?;
            self.root.progress.skipped += 1;
            self.notice(NoticeKind::Unreadable, path, &note)?;
            return Ok(());
        }
        let mut bytes = Vec::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        let mut reader = file.by_ref().take(source_limit + 1);
        let mut since_progress = 0;
        loop {
            if self.observer.cancelled() {
                return Ok(());
            }
            let count = reader.read(&mut buffer).map_err(|e| io_error(&e))?;
            if count == 0 {
                break;
            }
            bytes.try_reserve(count).map_err(|_| IndexError("Not enough memory to read this document. Choose a lower document budget or retry after closing other apps.".into()))?;
            bytes.extend_from_slice(&buffer[..count]);
            self.root.progress.bytes_read += count as u64;
            since_progress += count;
            if since_progress >= 4 * 1024 * 1024 {
                self.observer.progress(self.root);
                since_progress = 0;
            }
        }
        let after = file.metadata().map_err(|e| io_error(&e))?;
        if bytes.len() as u64 > source_limit
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
        {
            return Err(IndexError(
                "The file changed while being read. Refresh the folder to retry.".into(),
            ));
        }
        if self.observer.cancelled() {
            return Ok(());
        }
        let content_hash = digest(&bytes);
        let physical = identity(&before);
        let file_id = self.file_id(path, &physical)?;
        // Decoder changes create new snapshots on refresh, never relabel old
        // citations or reuse text produced with outdated format/size handling.
        let version = digest(
            format!(
                "{}\0{budget:?}\0{file_id}\0{content_hash}",
                super::history::EXTRACTOR_REVISION
            )
            .as_bytes(),
        );
        let known: bool = self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM index_versions WHERE id=?1)",
                [&version],
                |r| r.get(0),
            )
            .map_err(|error| db_error(&error))?;
        let prepared = if known {
            None
        } else {
            Some(
                crate::prepare_indexed_bytes(&file_id, name, &bytes, budget, Some(self.observer))
                    .map_err(|e| IndexError(e.to_string()))?,
            )
        };
        if self.observer.cancelled() {
            return Ok(());
        }
        let modified = before
            .modified()
            .ok()
            .and_then(|t| t.into_std().duration_since(std::time::UNIX_EPOCH).ok())
            .map(|t| t.as_secs());
        let tx = self.db.transaction().map_err(|error| db_error(&error))?;
        tx.execute("INSERT INTO index_files(id,root_id,path,identity,current_version,generation,available) VALUES(?1,?2,?3,?4,?5,?6,1) ON CONFLICT(id) DO UPDATE SET path=excluded.path,identity=excluded.identity,current_version=excluded.current_version,generation=excluded.generation,available=1",
            params![file_id, self.root.id, path, physical, version, self.generation]).map_err(|error| db_error(&error))?;
        let (state, detail) = if let Some(prepared) = prepared {
            let partial = prepared.partial;
            let prepared = prepared.content;
            let state = if prepared.raw.is_some() || prepared.text.trim().is_empty() {
                ExtractionState::MetadataOnly
            } else if partial {
                ExtractionState::PartialText
            } else {
                ExtractionState::Text
            };
            let note = if state == ExtractionState::MetadataOnly {
                prepared.attachment.note.or_else(|| Some("No searchable text was extracted. Attach this file in chat for inspection with a suitable model.".into()))
            } else {
                prepared.attachment.note
            };
            tx.execute("INSERT INTO index_versions(id,file_id,digest,mime,note,observed,modified,extractor_revision,generated_by,partial) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![version, file_id, content_hash, prepared.attachment.mime_type, note, i64::try_from(now()).expect("current Unix time fits i64"), modified.and_then(|v| i64::try_from(v).ok()),super::history::EXTRACTOR_REVISION,self.activity,partial]).map_err(|error| db_error(&error))?;
            let source_text =
                if matches!(state, ExtractionState::Text | ExtractionState::PartialText) {
                    prepared.text.as_str()
                } else {
                    ""
                };
            for (ordinal, chunk) in chunks(source_text).into_iter().enumerate() {
                let chunk_id = digest(chunk.body.as_bytes());
                tx.execute(
                    "INSERT INTO index_chunks(id,body) VALUES(?1,?2) ON CONFLICT(id) DO NOTHING",
                    params![chunk_id, chunk.body],
                )
                .map_err(|error| db_error(&error))?;
                tx.execute("INSERT INTO index_version_chunks(version_id,ordinal,chunk_id,start_line,end_line) VALUES(?1,?2,?3,?4,?5)",
                    params![version, i64::try_from(ordinal).expect("bounded text chunk count"), chunk_id, chunk.start_line, chunk.end_line]).map_err(|error| db_error(&error))?;
            }
            (state, note)
        } else {
            let (has_text,note,partial): (bool,Option<String>,bool) = tx.query_row("SELECT EXISTS(SELECT 1 FROM index_version_chunks WHERE version_id=?1),note,partial FROM index_versions WHERE id=?1", [&version], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e| db_error(&e))?;
            (
                if has_text && partial {
                    ExtractionState::PartialText
                } else if has_text {
                    ExtractionState::Text
                } else {
                    ExtractionState::MetadataOnly
                },
                note,
            )
        };
        save_metadata(&tx, &file_id, path, &before, state, detail.as_deref())?;
        super::history::observe(&tx, &file_id, self.activity)?;
        tx.commit().map_err(|error| db_error(&error))?;
        if state == ExtractionState::MetadataOnly {
            self.notice(
                NoticeKind::Unreadable,
                path,
                detail
                    .as_deref()
                    .unwrap_or("No searchable text was extracted."),
            )?;
        }
        if state == ExtractionState::PartialText {
            self.notice(
                NoticeKind::Partial,
                path,
                detail
                    .as_deref()
                    .unwrap_or("Some content could not be read. Check the original file."),
            )?;
        }
        if known {
            self.root.progress.reused += 1;
        } else {
            self.root.progress.indexed += 1;
        }
        Ok(())
    }

    fn file_id(&self, path: &str, physical: &str) -> Result<String, IndexError> {
        if let Some(id) = self
            .db
            .query_row(
                "SELECT id FROM index_files WHERE root_id=?1 AND path=?2",
                params![self.root.id, path],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| db_error(&error))?
        {
            return Ok(id);
        }
        // A unique filesystem identity permits rename preservation. Hard links
        // keep separate path entries; ambiguous identities never merge records.
        if !physical.is_empty() {
            let mut query = self
                .db
                .prepare("SELECT id,path FROM index_files WHERE root_id=?1 AND identity=?2 LIMIT 2")
                .map_err(|error| db_error(&error))?;
            let candidates = query
                .query_map(params![self.root.id, physical], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|error| db_error(&error))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| db_error(&error))?;
            if let [(id, old_path)] = candidates.as_slice() {
                let root = Dir::open_ambient_dir(&self.root.path, cap_std::ambient_authority())
                    .map_err(|e| io_error(&e))?;
                if root
                    .symlink_metadata(old_path)
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                {
                    return Ok(id.clone());
                }
            }
        }
        Ok(digest(
            format!("{}\0{path}\0{physical}\0{}", self.root.id, self.generation).as_bytes(),
        ))
    }
}

fn save_metadata(
    db: &Connection,
    id: &str,
    path: &str,
    metadata: &Metadata,
    state: ExtractionState,
    note: Option<&str>,
) -> Result<(), IndexError> {
    let (name, directory, extension) = super::catalogue::path_parts(path);
    let timestamp = |time: std::io::Result<cap_std::time::SystemTime>| {
        time.ok()
            .and_then(|time| time.into_std().duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|duration| i64::try_from(duration.as_secs()).ok())
    };
    let state = match state {
        ExtractionState::Text => "text",
        ExtractionState::PartialText => "partial_text",
        ExtractionState::MetadataOnly => "metadata_only",
        ExtractionState::Failed => "failed",
    };
    db.execute("UPDATE index_files SET filename=?2,directory=?3,extension=?4,byte_size=?5,observed=?6,modified=?7,created=?8,extraction=?9,detail=?10 WHERE id=?1",
        params![id,name,directory,extension,i64::try_from(metadata.len()).ok(), i64::try_from(now()).expect("Unix time fits i64"),timestamp(metadata.modified()),timestamp(metadata.created()),state,note]).map_err(|e| db_error(&e))?;
    Ok(())
}

pub(super) fn identity(metadata: &Metadata) -> String {
    use cap_fs_ext::MetadataExt as _;
    format!(
        "{}:{}:{:?}",
        metadata.dev(),
        metadata.ino(),
        metadata.created().ok()
    )
}

pub(super) fn issue(root: &mut IndexedRoot, message: String) {
    if root.issues.len() < MAX_INDEX_ISSUES {
        root.issues.push(message);
    }
}

struct TextChunk<'a> {
    body: &'a str,
    start_line: u32,
    end_line: u32,
}

fn chunks(text: &str) -> Vec<TextChunk<'_>> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut line = 1_u32;
    for chunk in fastcdc::v2020::FastCDC::new(text.as_bytes(), 512, 2048, 8192) {
        let end = text.floor_char_boundary(chunk.offset + chunk.length);
        if end <= start {
            continue;
        }
        let body = &text[start..end];
        let newlines =
            u32::try_from(body.bytes().filter(|b| *b == b'\n').count()).unwrap_or(u32::MAX);
        result.push(TextChunk {
            body,
            start_line: line,
            end_line: line
                .saturating_add(newlines)
                .saturating_sub(u32::from(body.ends_with('\n'))),
        });
        line = line.saturating_add(newlines);
        start = end;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_chunks_preserve_utf8_and_reuse_unmodified_regions_after_an_insertion() {
        use std::fmt::Write as _;
        let mut text = String::new();
        for n in 0_u64..10_000 {
            let _ = writeln!(text, "Line {n}: flowering cây 🌳 {}", n * 15_485_863);
        }
        let before = chunks(&text);
        assert_eq!(before.iter().map(|c| c.body).collect::<String>(), text);
        let changed = format!("A new opening paragraph.\n{text}");
        let after = chunks(&changed);
        assert_eq!(after.iter().map(|c| c.body).collect::<String>(), changed);
        let reused = after
            .iter()
            .filter(|chunk| before.iter().any(|old| old.body == chunk.body))
            .count();
        assert!(
            reused > before.len() / 2,
            "{reused} reused of {}",
            before.len()
        );
        assert_eq!(after[0].start_line, 1);
        assert_eq!(after.last().unwrap().end_line, 10_001);
    }
}
