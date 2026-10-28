//! Reconcile a cancellable walk with bounded per-file work; only a completed walk can retire missing paths.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsSyncExt as _};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use rusqlite::{Connection, OptionalExtension as _, params};
use sovra_core::file_index::{
    ExtractionState, IndexError, IndexedRoot, MAX_INDEX_ISSUES, ScanObserver,
};
use sovra_core::files::MAX_FILE_BYTES;

use super::{db_error, digest, io_error, now, save_root};

mod stream_text;

pub(super) fn run(
    db: &mut Connection,
    root: &mut IndexedRoot,
    storage: &Path,
    observer: &dyn ScanObserver,
    activity: i64,
) -> Result<(), IndexError> {
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
    };
    scan.walk(dir)?;
    if observer.cancelled() {
        return Ok(());
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
            return Ok(());
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
    Ok(())
}

struct Scan<'a> {
    activity: i64,
    db: &'a mut Connection,
    root: &'a mut IndexedRoot,
    observer: &'a dyn ScanObserver,
    generation: i64,
    excluded: Option<PathBuf>,
}

impl Scan<'_> {
    fn walk(&mut self, dir: Dir) -> Result<(), IndexError> {
        // An explicit stack avoids a call-stack depth cutoff. Each directory is
        // held as a capability; only the current branch has open handles.
        let mut stack = vec![(
            dir.entries().map_err(|e| io_error(&e))?,
            dir,
            PathBuf::new(),
        )];
        while let Some((entries, dir, relative)) = stack.last_mut() {
            let Some(entry) = entries.next() else {
                stack.pop();
                continue;
            };
            if self.observer.cancelled() {
                return Ok(());
            }
            self.root.progress.visited += 1;
            let entry = entry.map_err(|e| io_error(&e))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                self.root.progress.skipped += 1;
                issue(
                    self.root,
                    "A filename is not valid Unicode and was skipped.".into(),
                );
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
                issue(self.root, format!("{}: {reason}.", path.display()));
                continue;
            }
            let metadata = dir.symlink_metadata(name).map_err(|e| io_error(&e))?;
            if metadata.is_symlink() {
                self.root.progress.skipped += 1;
                issue(
                    self.root,
                    format!(
                        "{}: symbolic link; add its destination folder directly to include it.",
                        path.display()
                    ),
                );
            } else if metadata.is_dir() {
                // Every component is opened relative to a held capability and
                // rejects symlinks, including a link swapped in during traversal.
                let child = dir.open_dir_nofollow(name).map_err(|e| io_error(&e))?;
                stack.push((child.entries().map_err(|e| io_error(&e))?, child, path));
            } else if metadata.is_file() {
                let relative = path
                    .to_str()
                    .ok_or_else(|| IndexError("Invalid filename.".into()))?
                    .replace(std::path::MAIN_SEPARATOR, "/");
                if let Err(error) = self.file(dir, name, &relative) {
                    self.root.progress.failed += 1;
                    issue(self.root, format!("{relative}: {error}"));
                    self.metadata_only(
                        &relative,
                        &metadata,
                        ExtractionState::Failed,
                        &error.to_string(),
                    )?;
                }
            } else {
                self.root.progress.skipped += 1;
                issue(
                    self.root,
                    format!("{}: not a regular file or folder.", path.display()),
                );
            }
            if self.root.progress.visited.is_multiple_of(32) {
                save_root(self.db, self.root)?;
                self.observer.progress(self.root);
            }
        }
        Ok(())
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
        if before.len() >= sovra_core::files::MAX_DOCUMENT_BYTES as u64
            && self.stream_text(&mut file, path, &before)?
        {
            return Ok(());
        }
        if before.len() > MAX_FILE_BYTES {
            let note = "This file is not streamable UTF-8 text and exceeds the 20 MB document-decoder budget. Its filename and size are searchable; its contents have not been read into the index.";
            self.metadata_only(path, &before, ExtractionState::MetadataOnly, note)?;
            self.root.progress.skipped += 1;
            issue(self.root, format!("{path}: {note}"));
            return Ok(());
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| io_error(&e))?;
        self.root.progress.bytes_read += bytes.len() as u64;
        let after = file.metadata().map_err(|e| io_error(&e))?;
        if bytes.len() as u64 > MAX_FILE_BYTES
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
        // v2 excludes byte-inspection descriptions and whitespace from source
        // passages. Older snapshots are re-extracted on refresh, never relabelled.
        let version = digest(
            format!(
                "{}\0{file_id}\0{content_hash}",
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
                crate::prepare_bytes(&file_id, name, &bytes)
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
            let state = if prepared.raw.is_some() || prepared.text.trim().is_empty() {
                ExtractionState::MetadataOnly
            } else {
                ExtractionState::Text
            };
            let note = if state == ExtractionState::MetadataOnly {
                prepared.attachment.note.or_else(|| Some("No searchable text was extracted. Attach this file in chat for inspection with a suitable model.".into()))
            } else {
                prepared.attachment.note
            };
            tx.execute("INSERT INTO index_versions(id,file_id,digest,mime,note,observed,modified,extractor_revision,generated_by) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![version, file_id, content_hash, prepared.attachment.mime_type, note, i64::try_from(now()).expect("current Unix time fits i64"), modified.and_then(|v| i64::try_from(v).ok()),super::history::EXTRACTOR_REVISION,self.activity]).map_err(|error| db_error(&error))?;
            let source_text = if state == ExtractionState::Text {
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
            let (has_text,note): (bool,Option<String>) = tx.query_row("SELECT EXISTS(SELECT 1 FROM index_version_chunks WHERE version_id=?1),note FROM index_versions WHERE id=?1", [&version], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e| db_error(&e))?;
            (
                if has_text {
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
