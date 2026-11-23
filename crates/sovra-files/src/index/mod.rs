//! Persistent, local, read-only folder ingestion. SQLite owns the derived index;
//! capability directory handles bound source reads to explicitly granted roots.

mod answers;
mod automatic;
mod catalogue;
mod derived;
mod graph;
mod history;
mod matching;
mod report;
mod scan;
mod semantic;
mod summary;
#[cfg(test)]
mod tests;
mod watch;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cap_std::fs::Dir;
use rusqlite::{Connection, OptionalExtension as _, params};
use sha2::{Digest as _, Sha256};
use sovra_core::file_index::{
    CatalogPage, CatalogQuery, FileIndex, IndexError, IndexPassage, IndexProgress, IndexQuery,
    IndexStatus, IndexedRoot, PassageRef, ScanCancellation, ScanObserver,
};

/// Batch writers reuse compiled SQL as each file passes through the same few
/// catalogue, history and chunk updates. Statements return to the connection
/// cache before the next transaction commits.
trait BatchWrites {
    fn execute_cached(&self, sql: &str, args: impl rusqlite::Params) -> rusqlite::Result<usize>;
}
impl BatchWrites for Connection {
    fn execute_cached(&self, sql: &str, args: impl rusqlite::Params) -> rusqlite::Result<usize> {
        self.prepare_cached(sql)?.execute(args)
    }
}

/// The count and notification walks need only a type hint. Some filesystems
/// omit it from directory entries, so fall back to no-follow entry metadata.
fn entry_type(entry: &cap_std::fs::DirEntry) -> std::io::Result<cap_std::fs::FileType> {
    let kind = entry.file_type()?;
    if kind.is_file() || kind.is_dir() || kind.is_symlink() {
        Ok(kind)
    } else {
        entry.metadata().map(|metadata| metadata.file_type())
    }
}

const SCHEMA_VERSION: i64 = 16;

#[derive(Debug)]
pub struct LocalFileIndex {
    directory: PathBuf,
    writes: Mutex<()>,
}

impl LocalFileIndex {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            writes: Mutex::new(()),
        }
    }

    fn connect(&self) -> Result<Connection, IndexError> {
        if std::fs::symlink_metadata(&self.directory).is_ok_and(|m| m.is_symlink()) {
            return Err(IndexError(
                "The index storage folder cannot be a symbolic link.".into(),
            ));
        }
        crate::private_directory(&self.directory).map_err(|e| IndexError(e.to_string()))?;
        let path = self.directory.join("files.sqlite3");
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_file() || meta.is_symlink() => {
                return Err(IndexError(
                    "The index database must be a regular file.".into(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt as _;
                    options.mode(0o600);
                }
                match options.open(&path) {
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(io_error(&error)),
                }
            }
            Err(error) => return Err(io_error(&error)),
            Ok(_) => {}
        }
        let mut db = Connection::open(path).map_err(|error| db_error(&error))?;
        matching::register(&db).map_err(|error| db_error(&error))?;
        db.busy_timeout(Duration::from_secs(5))
            .map_err(|error| db_error(&error))?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| db_error(&error))?;
        if version > SCHEMA_VERSION {
            return Err(IndexError(
                "This file index was created by a newer Sovra version.".into(),
            ));
        }
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF;")
            .map_err(|error| db_error(&error))?;
        if version < SCHEMA_VERSION {
            if version == 0 {
                db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
                    .map_err(|e| db_error(&e))?;
            }
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| db_error(&e))?;
            // Another reader may have initialized this database while this
            // connection waited for the migration lock.
            let current: i64 = tx
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .map_err(|e| db_error(&e))?;
            if current > SCHEMA_VERSION {
                return Err(IndexError(
                    "This file index was upgraded by a newer Sovra version.".into(),
                ));
            }
            if current == 0 {
                tx.execute_batch(include_str!("schema.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            if current < 2 {
                tx.execute_batch(include_str!("catalogue_v2.sql"))
                    .map_err(|e| db_error(&e))?;
                let mut statement = tx
                    .prepare("SELECT id,path FROM index_files")
                    .map_err(|e| db_error(&e))?;
                let rows = statement
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(|e| db_error(&e))?;
                for row in rows {
                    let (id, path) = row.map_err(|e| db_error(&e))?;
                    let (name, directory, extension) = catalogue::path_parts(&path);
                    tx.execute(
                        "UPDATE index_files SET filename=?2,directory=?3,extension=?4 WHERE id=?1",
                        params![id, name, directory, extension],
                    )
                    .map_err(|e| db_error(&e))?;
                }
            }
            if current < 3 {
                tx.execute_batch("CREATE INDEX IF NOT EXISTS file_catalogue_version ON index_files(current_version) WHERE available=1; PRAGMA user_version=3;").map_err(|e| db_error(&e))?;
            }
            if current < 4 {
                history::migrate(&tx)?;
            }
            if current < 5 {
                tx.execute_batch(include_str!("history_indexes_v5.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            if current < 6 {
                tx.execute_batch(include_str!("semantic_v6.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            if current < 7 {
                tx.execute_batch(include_str!("knowledge_v7.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            if current < 8 {
                tx.execute_batch(include_str!("summary_v8.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            if current < 9 {
                tx.execute_batch(include_str!("report_v9.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            if current < 10 {
                tx.execute_batch(include_str!("partial_v10.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            if current < 11 {
                tx.execute_batch(include_str!("knowledge_v11.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            if current < 12 {
                tx.execute_batch(include_str!("word_forms_v12.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            if current < 13 {
                tx.execute_batch(include_str!("answers_v13.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            if current < 14 {
                tx.execute_batch(include_str!("mask_v14.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            if current < 15 {
                tx.execute_batch("CREATE INDEX IF NOT EXISTS knowledge_property_id ON index_knowledge_relations(json_extract(kind,'$.property.id')); PRAGMA user_version=15;")
                    .map_err(|e| db_error(&e))?;
            }
            if current < 16 {
                tx.execute_batch(include_str!("automatic_v16.sql"))
                    .map_err(|e| db_error(&e))?;
            }
            tx.commit().map_err(|e| db_error(&e))?;
        }
        Ok(db)
    }
}

impl FileIndex for LocalFileIndex {
    fn table_wording(&self, reference: &PassageRef) -> Result<Option<String>, IndexError> {
        let db = self.connect()?;
        let mut stmt = db.prepare("SELECT c.body FROM index_files f JOIN index_version_chunks vc ON vc.version_id=f.current_version JOIN index_chunks c ON c.id=vc.chunk_id WHERE f.root_id=?1 AND f.id=?2 AND f.current_version=?3 AND f.available=1 ORDER BY vc.ordinal").map_err(|e|db_error(&e))?;
        let mut rows = stmt
            .query(params![
                reference.root_id,
                reference.file_id,
                reference.version_id
            ])
            .map_err(|e| db_error(&e))?;
        let mut whole = String::new();
        while let Some(row) = rows.next().map_err(|e| db_error(&e))? {
            let text: String = row.get(0).map_err(|e| db_error(&e))?;
            let body = sovra_core::file_index::passage_wording(&text);
            if whole.len() + body.len() > sovra_core::knowledge_analysis::MAX_ANALYSIS_BYTES {
                return Ok(None);
            }
            whole.push_str(body);
        }
        Ok((!whole.is_empty()).then_some(whole))
    }
    fn binary_mask(
        &self,
        r: &PassageRef,
    ) -> Result<Option<sovra_core::knowledge_analysis::mask::Mask>, IndexError> {
        let db = self.connect()?;
        let row:Option<(u16,u16,Vec<u8>)>=db.query_row("SELECT m.width,m.height,m.bits FROM index_masks m JOIN index_files f ON f.current_version=m.version_id JOIN index_version_chunks vc ON vc.version_id=m.version_id WHERE f.root_id=?1 AND f.id=?2 AND f.current_version=?3 AND f.available=1 AND vc.ordinal=?4",params![r.root_id,r.file_id,r.version_id,r.ordinal],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|e|db_error(&e))?;
        row.map(|(w, h, b)| {
            sovra_core::knowledge_analysis::mask::Mask::unpack(w.into(), h.into(), &b)
                .ok_or_else(|| IndexError("The saved binary image is incomplete.".into()))
        })
        .transpose()
    }
    fn save_answer(
        &self,
        reference: &PassageRef,
    ) -> Result<sovra_core::file_index::DatedAnswer, IndexError> {
        self.store_answer(reference)
    }
    fn saved_answers(
        &self,
        before: Option<u64>,
    ) -> Result<sovra_core::file_index::SavedAnswerPage, IndexError> {
        self.answer_page(before)
    }
    fn report(
        &self,
        query: &sovra_core::file_index::NoticeQuery,
    ) -> Result<sovra_core::file_index::NoticePage, IndexError> {
        report::query(&mut self.connect()?, query)
    }
    fn summaries(&self) -> Option<&dyn sovra_core::file_index::summary::SummaryIndex> {
        Some(self)
    }
    fn automatic(&self) -> Option<&dyn sovra_core::file_index::automatic::AutomaticIndex> {
        Some(self)
    }
    fn knowledge(&self) -> Option<&dyn sovra_core::file_index::graph::FileKnowledge> {
        Some(self)
    }
    fn semantic(&self) -> Option<&dyn sovra_core::file_index::semantic::SemanticIndex> {
        Some(self)
    }
    fn history(
        &self,
        query: &sovra_core::file_index::FileHistoryQuery,
    ) -> Result<sovra_core::file_index::FileHistory, IndexError> {
        history::query(&mut self.connect()?, query)
    }
    fn read_history(
        &self,
        reference: &sovra_core::file_index::HistoryPassageRef,
    ) -> Result<sovra_core::file_index::HistoricalPassage, IndexError> {
        history::read(&mut self.connect()?, reference)
    }
    fn catalogue(&self, query: &CatalogQuery) -> Result<CatalogPage, IndexError> {
        catalogue::query(&mut self.connect()?, query)
    }
    fn list(&self) -> Result<Vec<IndexedRoot>, IndexError> {
        let db = self.connect()?;
        let mut query = db
            .prepare("SELECT details FROM index_roots ORDER BY path")
            .map_err(|error| db_error(&error))?;
        query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| db_error(&error))?
            .map(|row| decode(&row.map_err(|error| db_error(&error))?))
            .collect()
    }

    fn recover(&self) -> Result<(), IndexError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let db = self.connect()?;
        history::recover(&db)?;
        for mut root in self.list()? {
            if root.status == IndexStatus::Scanning {
                root.status = IndexStatus::Interrupted;
                root.issues.push("The previous scan stopped before completion. Refresh this folder to reconcile changes.".into());
                save_root(&db, &root)?;
            }
        }
        Ok(())
    }

    fn add_root(&self, path: &Path) -> Result<IndexedRoot, IndexError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let path = std::fs::canonicalize(path).map_err(|e| io_error(&e))?;
        let display = path
            .to_str()
            .ok_or_else(|| IndexError("Choose a folder with a Unicode path.".into()))?;
        let dir =
            Dir::open_ambient_dir(&path, cap_std::ambient_authority()).map_err(|e| io_error(&e))?;
        let identity = scan::identity(&dir.dir_metadata().map_err(|e| io_error(&e))?);
        let db = self.connect()?;
        if let Some(details) = db
            .query_row(
                "SELECT details FROM index_roots WHERE path=?1",
                [display],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| db_error(&error))?
        {
            return decode(&details);
        }
        let root = IndexedRoot {
            id: digest(display.as_bytes()),
            path: path.clone(),
            status: IndexStatus::Pending,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: None,
            auto_refresh: true,
            scan_options: sovra_core::file_index::ScanOptions::default(),
        };
        db.execute(
            "INSERT INTO index_roots(id,path,identity,details) VALUES(?1,?2,?3,?4)",
            params![root.id, display, identity, encode(&root)?],
        )
        .map_err(|error| db_error(&error))?;
        Ok(root)
    }

    fn scan(&self, root_id: &str, observer: &dyn ScanObserver) -> Result<IndexedRoot, IndexError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        let mut root = get_root(&db, root_id)?;
        root.status = IndexStatus::Scanning;
        root.progress = IndexProgress::default();
        root.issues.clear();
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        save_root(&tx, &root)?;
        let activity = history::begin(&tx, root_id)?;
        report::begin(&tx, root_id, activity)?;
        tx.commit().map_err(|e| db_error(&e))?;
        let started = std::time::Instant::now();
        observer.progress(&root);
        let result = scan::run(&mut db, &mut root, &self.directory, observer, activity);
        // One slow final file may finish between progress samples.
        root.progress.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match (observer.cancellation(), result) {
            (Some(ScanCancellation::Requested), _) => {
                root.status = IndexStatus::Cancelled;
                root.auto_refresh = false;
            }
            (Some(ScanCancellation::Interrupted), _) => root.status = IndexStatus::Interrupted,
            (None, Ok(scan::Completion::Complete)) => {
                root.status = IndexStatus::Ready;
                root.completed_at = Some(now());
            }
            (None, Ok(scan::Completion::Incomplete)) => root.status = IndexStatus::Failed,
            (None, Err(error)) => {
                root.status = IndexStatus::Failed;
                root.progress.failed += 1;
                report::append(
                    &mut db,
                    root_id,
                    &[report::PendingNotice {
                        kind: sovra_core::file_index::NoticeKind::Failed,
                        path: ".".into(),
                        message: error.to_string(),
                    }],
                )?;
                scan::issue(&mut root, error.to_string());
            }
        }
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        save_root(&tx, &root)?;
        history::finish(&tx, activity, root.status)?;
        tx.commit().map_err(|e| db_error(&e))?;
        observer.progress(&root);
        Ok(root)
    }

    fn set_auto_refresh(&self, root_id: &str, enabled: bool) -> Result<(), IndexError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let db = self.connect()?;
        let mut root = get_root(&db, root_id)?;
        root.auto_refresh = enabled;
        save_root(&db, &root)
    }

    fn set_scan_options(
        &self,
        root_id: &str,
        options: sovra_core::file_index::ScanOptions,
    ) -> Result<(), IndexError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        let mut root = get_root(&tx, root_id)?;
        if root.scan_options == options {
            return Ok(());
        }
        // Publish the new policy and remove excluded files from current retrieval
        // together; cancellation of the following scan cannot restore them.
        if root.scan_options.include_hidden != options.include_hidden
            || root.scan_options.include_generated != options.include_generated
        {
            let mut after = String::new();
            loop {
                let rows = {
                    let mut statement = tx.prepare("SELECT id,path FROM index_files WHERE root_id=?1 AND path>?2 AND available=1 ORDER BY path LIMIT 256").map_err(|e| db_error(&e))?;
                    statement
                        .query_map(params![root_id, after], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        })
                        .map_err(|e| db_error(&e))?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(|e| db_error(&e))?
                };
                if rows.is_empty() {
                    break;
                }
                for (id, path) in rows {
                    if path
                        .split('/')
                        .any(|name| options.exclusion_reason(name).is_some())
                    {
                        tx.execute("UPDATE index_files SET available=0 WHERE id=?1", [&id])
                            .map_err(|e| db_error(&e))?;
                    }
                    after = path;
                }
            }
        }
        root.scan_options = options;
        root.status = IndexStatus::Pending;
        save_root(&tx, &root)?;
        tx.commit().map_err(|e| db_error(&e))
    }

    fn watch(
        &self,
        root: &IndexedRoot,
        changes: std::sync::Arc<dyn sovra_core::file_index::IndexChanges>,
    ) -> Result<Box<dyn sovra_core::file_index::IndexWatch>, IndexError> {
        watch::start(root, &self.directory, changes)
    }

    fn remove(&self, root_id: &str) -> Result<(), IndexError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut db = self.connect()?;
        let tx = db.transaction().map_err(|error| db_error(&error))?;
        // A home-directory grant can hold hundreds of thousands of files and
        // millions of chunks. Per-file catalogue updates and per-chunk search
        // index deletes make that removal look stuck. Suspend those row
        // triggers, delete the grant, then rebuild search once. Grant-level
        // triggers stay so saved quotations, summaries and pending reviews
        // still follow the folder.
        let triggers = suspend_row_triggers(&tx)?;
        tx.execute("DELETE FROM index_files WHERE root_id=?1", [root_id])
            .map_err(|error| db_error(&error))?;
        tx.execute("DELETE FROM index_chunks WHERE NOT EXISTS (SELECT 1 FROM index_version_chunks vc WHERE vc.chunk_id=index_chunks.id)", []).map_err(|error| db_error(&error))?;
        tx.execute_batch(
            "INSERT INTO index_fts(index_fts) VALUES('rebuild');
             INSERT INTO index_word_forms(index_word_forms) VALUES('rebuild');",
        )
        .map_err(|error| db_error(&error))?;
        restore_row_triggers(&tx, &triggers)?;
        tx.execute("DELETE FROM index_roots WHERE id=?1", [root_id])
            .map_err(|error| db_error(&error))?;
        tx.commit().map_err(|error| db_error(&error))
    }

    fn search(&self, query: &IndexQuery) -> Result<Vec<IndexPassage>, IndexError> {
        self.search_language(query, sovra_core::knowledge_analysis::Language::English)
    }
    fn search_language(
        &self,
        query: &IndexQuery,
        language: sovra_core::knowledge_analysis::Language,
    ) -> Result<Vec<IndexPassage>, IndexError> {
        query.validate()?;
        // Literal phrases, never file- or model-provided SQL/FTS operators.
        let terms = sovra_core::file_index::lexical_match(&query.text);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let roots = serde_json::to_string(&query.roots)
            .map_err(|_| IndexError("Invalid folder filter.".into()))?;
        let prefix = query
            .path_prefix
            .as_deref()
            .unwrap_or("")
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
            + "%";
        let db = self.connect()?;
        // Identifiers, paths and explicitly quoted searches keep their literal tokens.
        let literal = query
            .text
            .chars()
            .any(|c| c.is_numeric() || matches!(c, '-' | '_' | '/' | '.' | '"' | ':' | '\\'));
        let (table, terms) = if literal {
            ("index_fts".to_owned(), terms)
        } else if language != sovra_core::knowledge_analysis::Language::English {
            let table = matching::language_index(&db, language).map_err(|e| db_error(&e))?;
            (
                table,
                sovra_core::file_index::lexical_match(&language.stem_text(&query.text)),
            )
        } else if query
            .text
            .chars()
            .any(|c| c.is_alphabetic() && !c.is_ascii())
        {
            ("index_fts".into(), terms)
        } else {
            ("index_word_forms".into(), terms)
        };
        let started = std::time::Instant::now();
        db.progress_handler(
            1000,
            Some(move || started.elapsed() > Duration::from_secs(5)),
        )
        .map_err(|error| db_error(&error))?;
        let sql = format!(
            "{PASSAGES} JOIN {table} ft ON ft.rowid=c.rowid WHERE {table} MATCH ?1 AND f.available=1 AND f.current_version=v.id AND (?2='[]' OR f.root_id IN (SELECT value FROM json_each(?2))) AND f.path LIKE ?3 ESCAPE '\\' ORDER BY bm25({table}),f.root_id,f.path,vc.ordinal LIMIT ?4"
        );
        let mut stmt = db.prepare(&sql).map_err(|error| db_error(&error))?;
        stmt.query_map(
            params![
                terms,
                roots,
                prefix,
                i64::try_from(query.limit).expect("validated result limit fits i64")
            ],
            passage,
        )
        .map_err(|error| db_error(&error))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| db_error(&error))
    }

    fn read(&self, reference: &PassageRef) -> Result<IndexPassage, IndexError> {
        let db = self.connect()?;
        let sql = format!(
            "{PASSAGES} WHERE f.root_id=?1 AND f.id=?2 AND v.id=?3 AND vc.ordinal=?4 AND f.available=1 AND f.current_version=v.id"
        );
        db.query_row(&sql, params![reference.root_id, reference.file_id, reference.version_id, reference.ordinal], passage)
            .optional().map_err(|error| db_error(&error))?.ok_or_else(|| IndexError("This indexed passage is no longer current or its folder was removed. Search again.".into()))
    }
}

const PASSAGES: &str = "SELECT f.root_id,f.id,v.id,vc.ordinal,f.path,c.body,vc.start_line,vc.end_line,v.mime,v.note,v.observed,v.modified FROM index_files f JOIN index_versions v ON v.file_id=f.id JOIN index_version_chunks vc ON vc.version_id=v.id JOIN index_chunks c ON c.id=vc.chunk_id";

fn passage(row: &rusqlite::Row<'_>) -> rusqlite::Result<IndexPassage> {
    Ok(IndexPassage {
        reference: PassageRef {
            root_id: row.get(0)?,
            file_id: row.get(1)?,
            version_id: row.get(2)?,
            ordinal: row.get(3)?,
        },
        path: row.get(4)?,
        text: row.get(5)?,
        start_line: row.get(6)?,
        end_line: row.get(7)?,
        mime_type: row.get(8)?,
        note: row.get(9)?,
        observed_at: u64::try_from(row.get::<_, i64>(10)?).unwrap_or_default(),
        modified_at: row
            .get::<_, Option<i64>>(11)?
            .and_then(|v| u64::try_from(v).ok()),
    })
}

fn get_root(db: &Connection, id: &str) -> Result<IndexedRoot, IndexError> {
    let value = db
        .query_row("SELECT details FROM index_roots WHERE id=?1", [id], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map_err(|error| db_error(&error))?
        .ok_or_else(|| IndexError("This folder is no longer indexed. Choose it again.".into()))?;
    decode(&value)
}

fn save_root(db: &Connection, root: &IndexedRoot) -> Result<(), IndexError> {
    db.execute(
        "UPDATE index_roots SET details=?2 WHERE id=?1",
        params![root.id, encode(root)?],
    )
    .map_err(|error| db_error(&error))?;
    Ok(())
}

fn encode(root: &IndexedRoot) -> Result<String, IndexError> {
    serde_json::to_string(root)
        .map_err(|_| IndexError("Could not save the folder's indexing status.".into()))
}

fn decode(value: &str) -> Result<IndexedRoot, IndexError> {
    serde_json::from_str(value).map_err(|_| {
        IndexError("The saved folder index is invalid. Remove and re-add this folder.".into())
    })
}

fn digest(bytes: &[u8]) -> String {
    {
        use std::fmt::Write as _;
        let mut encoded = String::with_capacity(64);
        for byte in Sha256::digest(bytes) {
            let _ = write!(encoded, "{byte:02x}");
        }
        encoded
    }
}

/// Row triggers whose per-record work dominates removal of a large folder.
/// Names are a fixed allow-list: SQLite cannot bind an identifier.
const ROW_TRIGGERS: &[&str] = &[
    "catalogue_delete",
    "summary_deleted_source",
    "index_chunks_delete",
    "word_forms_delete",
];

fn suspend_row_triggers(tx: &rusqlite::Transaction<'_>) -> Result<Vec<String>, IndexError> {
    let mut saved = Vec::new();
    for name in ROW_TRIGGERS {
        let sql: Option<String> = tx
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
                [*name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| db_error(&error))?;
        let Some(sql) = sql else {
            continue;
        };
        tx.execute(&format!("DROP TRIGGER {name}"), [])
            .map_err(|error| db_error(&error))?;
        saved.push(sql);
    }
    Ok(saved)
}

fn restore_row_triggers(
    tx: &rusqlite::Transaction<'_>,
    triggers: &[String],
) -> Result<(), IndexError> {
    for sql in triggers {
        tx.execute(sql, []).map_err(|error| db_error(&error))?;
    }
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn io_error(error: &std::io::Error) -> IndexError {
    IndexError(format!(
        "Folder indexing could not access a local file ({}). Check access and available disk space, then refresh.",
        error.kind()
    ))
}
fn db_error(error: &rusqlite::Error) -> IndexError {
    IndexError(format!(
        "The local file index could not be updated: {error}. Check available disk space and retry."
    ))
}
