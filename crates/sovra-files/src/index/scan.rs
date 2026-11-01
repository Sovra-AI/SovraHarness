//! Reconcile a cancellable walk with bounded per-file work; only a completed walk can retire missing paths.
//!
//! The scan is a three-stage pipeline joined by bounded channels:
//!
//! 1. [`walk`] — one capability-safe walker finds and opens files (cheap, sequential).
//! 2. [`read`] — `readers()` threads read, hash and extract in parallel (the expensive part).
//! 3. the writer (this thread) saves results to SQLite in short batched transactions and
//!    publishes throttled progress, including what kinds of files have been found.
//!
//! Bounded channels give backpressure: a slow disk or a slow extractor never piles work
//! up in memory, and open file handles are capped. Cancellation is checked at every stage.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cap_std::fs::{Dir, Metadata};
use rusqlite::{Connection, OptionalExtension as _, params};
use sovra_core::file_index::live::RateEstimator;
use sovra_core::file_index::ontology;
use sovra_core::file_index::{
    DocumentBudget, ExtractionState, IndexError, IndexedRoot, MAX_INDEX_ISSUES, NoticeKind,
    ScanObserver,
};

use super::{db_error, digest, io_error, now, save_root};

mod read;
mod stream_text;
mod walk;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Completion {
    Complete,
    Incomplete,
}

/// Outcomes per transaction. Each commit is followed by a progress event, so what the
/// report shows always matches the counts on screen, and a cancelled scan keeps nearly
/// everything it finished.
const BATCH_FILES: usize = 32;
/// A quiet stretch (one slow file, or the walk still counting) still refreshes ~10×/s.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// Persist the folder's progress (for crash recovery) at most once a second.
const SAVE_INTERVAL: Duration = Duration::from_secs(1);
/// Source bytes all readers may hold at once.
const IN_FLIGHT_BYTES: u64 = 512 * 1024 * 1024;

/// Readers: leave one core for the UI and runtime; past eight, extraction contends for
/// memory bandwidth and the disk more than it gains.
fn readers() -> usize {
    std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .saturating_sub(1)
        .clamp(1, 8)
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
        database: storage.join("files.sqlite3"),
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

/// An opened regular file found by the walker, waiting for a reader.
pub(super) struct Job {
    file: cap_std::fs::File,
    name: String,
    path: String,
    metadata: Metadata,
}

/// Which counter a walk- or metadata-level outcome advances.
#[derive(Clone, Copy)]
pub(super) enum Tally {
    Skipped,
    Failed,
}

/// Everything the writer is told. Only the writer touches the database for writing.
pub(super) enum Outcome {
    /// A walk-level notice about an entry that is not a discovered file.
    Notice {
        kind: NoticeKind,
        path: String,
        message: String,
        tally: Tally,
    },
    /// A discovered file kept as metadata only (too large, or it failed to read).
    Metadata {
        path: String,
        metadata: Metadata,
        state: ExtractionState,
        note: String,
        notice: NoticeKind,
        tally: Tally,
    },
    /// A discovered file a reader finished.
    Read(Box<ReadFile>),
    /// A large file for the sequential streaming text reader.
    Stream(Job),
}

pub(super) struct ReadFile {
    name: String,
    path: String,
    metadata: Metadata,
    content_hash: String,
    /// `None` when a matching saved version made extraction unnecessary.
    prepared: Option<crate::PreparedFile>,
    /// Source bytes, kept only when extraction was skipped (see [`Writer::read`]).
    bytes: Option<Vec<u8>>,
    /// Released once the writer has saved this result.
    _reservation: Option<read::Reservation>,
}

/// Counters and signals shared by all three stages. Atomics, not events: the writer
/// samples them on a timer, so a million tiny files never become a million UI updates.
pub(super) struct Shared<'a> {
    observer: &'a dyn ScanObserver,
    abort: AtomicBool,
    visited: AtomicU64,
    discovered: AtomicU64,
    bytes_read: AtomicU64,
    walk_done: AtomicBool,
    /// Files found by the listing-only count; an early total for the progress bar.
    counted: AtomicU64,
    count_done: AtomicBool,
    budget: Arc<read::ByteBudget>,
}

impl<'a> Shared<'a> {
    #[cfg(test)]
    fn new(observer: &'a dyn ScanObserver, budget: read::ByteBudget) -> Self {
        Self::continuing(
            observer,
            budget,
            &sovra_core::file_index::IndexProgress::default(),
        )
    }

    /// Counters continue from `progress`, so a resumed folder never appears to go backwards.
    fn continuing(
        observer: &'a dyn ScanObserver,
        budget: read::ByteBudget,
        progress: &sovra_core::file_index::IndexProgress,
    ) -> Self {
        Self {
            observer,
            abort: AtomicBool::new(false),
            visited: AtomicU64::new(progress.visited),
            discovered: AtomicU64::new(progress.discovered),
            bytes_read: AtomicU64::new(progress.bytes_read),
            walk_done: AtomicBool::new(false),
            counted: AtomicU64::new(0),
            count_done: AtomicBool::new(false),
            budget: Arc::new(budget),
        }
    }

    fn stopped(&self) -> bool {
        self.abort.load(Relaxed) || self.observer.cancelled()
    }
}

struct Scan<'a> {
    activity: i64,
    db: &'a mut Connection,
    root: &'a mut IndexedRoot,
    observer: &'a dyn ScanObserver,
    generation: i64,
    excluded: Option<PathBuf>,
    notices: Vec<super::report::PendingNotice>,
    database: PathBuf,
}

/// Per-scan writer bookkeeping that is not part of the persisted progress.
struct Clock {
    started: Instant,
    rate: RateEstimator,
    last_progress: Instant,
    last_save: Instant,
}

impl Scan<'_> {
    fn notice(&mut self, kind: NoticeKind, path: &str, message: &str) -> Result<(), IndexError> {
        note(self.root, &mut self.notices, kind, path, message);
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
        let readers = readers();
        let budget = read::ByteBudget::new(
            IN_FLIGHT_BYTES.max(self.root.scan_options.documents.source_bytes()),
        );
        let shared = Shared::continuing(self.observer, budget, &self.root.progress);
        let (job_tx, job_rx) = sync_channel::<Job>(readers * 2);
        // Shared so the walker's sends fail once every reader has gone.
        let job_rx = Arc::new(Mutex::new(job_rx));
        let (out_tx, out_rx) = sync_channel::<Outcome>(readers * 4);
        let options = self.root.scan_options;
        let excluded = self.excluded.clone();
        let context = read::ReadContext {
            root_id: self.root.id.clone(),
            root_path: self.root.path.clone(),
            generation: self.generation,
            budget: self.root.scan_options.documents,
            database: self.database.clone(),
        };
        let counter_dir = dir.try_clone().ok();
        let count_excluded = self.excluded.clone();
        std::thread::scope(|scope| {
            if let Some(counter_dir) = counter_dir {
                let shared = &shared;
                scope.spawn(move || {
                    walk::count(counter_dir, options, count_excluded.as_deref(), shared);
                });
            }
            let walker = {
                let (shared, out) = (&shared, out_tx.clone());
                scope.spawn(move || {
                    walk::walk(dir, options, excluded.as_deref(), shared, &job_tx, &out)
                })
            };
            let workers: Vec<_> = (0..readers)
                .map(|_| {
                    let (context, shared) = (&context, &shared);
                    let (jobs, out) = (Arc::clone(&job_rx), out_tx.clone());
                    scope.spawn(move || read::work(context, shared, &jobs, &out))
                })
                .collect();
            drop((out_tx, job_rx));
            let written = self.drain(&out_rx, &shared);
            if written.is_err() {
                shared.abort.store(true, Relaxed);
            }
            // Dropping the receiver unblocks any stage waiting to send.
            drop(out_rx);
            let unread = walker.join().map_err(|_| {
                IndexError("The folder walk stopped unexpectedly. Refresh to retry.".into())
            });
            // Joined explicitly so a reader crash is an ordinary, recoverable scan error.
            let readers_ok = workers.into_iter().all(|worker| worker.join().is_ok());
            written?;
            let unread = unread?;
            if !readers_ok {
                return Err(IndexError(
                    "Reading stopped unexpectedly. Files already saved were kept; refresh to retry."
                        .into(),
                ));
            }
            self.publish(&shared, None, true);
            if self.observer.cancelled() {
                return Ok(Completion::Incomplete);
            }
            // Only a walk that reached its end may retire unseen files.
            if unread == 0 && shared.walk_done.load(Relaxed) {
                Ok(Completion::Complete)
            } else {
                // A partial walk cannot prove a previously indexed path is gone.
                // Keep the aggregate guidance out of the persisted failure rows:
                // each affected location has already been counted and reported.
                issue(
                    self.root,
                    if unread > 0 {
                        format!(
                            "Scan incomplete: {unread} folders or entries could not be read. Accessible files were updated. Older entries were kept because this scan cannot confirm which files are missing. Check the scan report, fix access, then refresh to retry."
                        )
                    } else {
                        "Scan stopped before checking every folder. Files already read were saved and older entries were kept. Refresh to finish.".into()
                    },
                );
                Ok(Completion::Incomplete)
            }
        })
    }

    /// The writer loop: batch outcomes into short transactions and publish progress on a
    /// timer until every walker and reader has finished (or the scan stops).
    fn drain(
        &mut self,
        outcomes: &Receiver<Outcome>,
        shared: &Shared<'_>,
    ) -> Result<(), IndexError> {
        let mut clock = Clock {
            started: Instant::now(),
            rate: RateEstimator::default(),
            last_progress: Instant::now(),
            last_save: Instant::now(),
        };
        let mut batch: Vec<Outcome> = Vec::with_capacity(BATCH_FILES);
        loop {
            let finished = match outcomes.recv_timeout(PROGRESS_INTERVAL) {
                Ok(Outcome::Stream(job)) => {
                    // Streaming writes its own transaction: settle the batch first.
                    self.commit(&mut batch)?;
                    self.stream(job, shared)?;
                    false
                }
                Ok(outcome) => {
                    batch.push(outcome);
                    false
                }
                Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => true,
            };
            let stopped = shared.stopped();
            if finished
                || stopped
                || batch.len() >= BATCH_FILES
                || clock.last_progress.elapsed() >= PROGRESS_INTERVAL
            {
                // Finished work is kept even when the scan is cancelled.
                self.commit(&mut batch)?;
                self.flush_notices()?;
                if finished || stopped {
                    return Ok(());
                }
                self.publish(shared, Some(&mut clock), false);
            }
        }
    }

    /// Copy shared counters into the folder's progress and tell the observer.
    fn publish(&mut self, shared: &Shared<'_>, clock: Option<&mut Clock>, force: bool) {
        let progress = &mut self.root.progress;
        progress.visited = shared.visited.load(Relaxed);
        // The early count gives the total; the reading walk is the authority once done
        // (files may appear or vanish mid-scan, so never show fewer than were found).
        let dispatched = shared.discovered.load(Relaxed);
        let walked = shared.walk_done.load(Relaxed);
        progress.discovered = if walked {
            dispatched
        } else {
            dispatched.max(shared.counted.load(Relaxed))
        };
        progress.walk_complete = walked || shared.count_done.load(Relaxed);
        progress.bytes_read = shared.bytes_read.load(Relaxed);
        if let Some(clock) = clock {
            let elapsed = u64::try_from(clock.started.elapsed().as_millis()).unwrap_or(u64::MAX);
            clock.rate.observe(elapsed, progress.handled);
            progress.elapsed_ms = elapsed;
            progress.files_per_minute = clock.rate.files_per_minute(elapsed, progress.handled);
            clock.last_progress = Instant::now();
            if clock.last_save.elapsed() >= SAVE_INTERVAL {
                // Best effort: a failed snapshot only affects crash-recovery display.
                let _ = save_root(self.db, self.root);
                clock.last_save = Instant::now();
            }
        }
        if force || !shared.stopped() {
            self.observer.progress(self.root);
        }
    }

    fn commit(&mut self, batch: &mut Vec<Outcome>) -> Result<(), IndexError> {
        if batch.is_empty() {
            return Ok(());
        }
        let Self {
            db,
            root,
            notices,
            generation,
            activity,
            ..
        } = self;
        let writer = Writer {
            root_id: root.id.clone(),
            root_path: root.path.clone(),
            generation: *generation,
            activity: *activity,
        };
        // Counters, issues and notices describe saved rows: if the batch rolls back, so do they.
        let (progress, issues, pending) = (root.progress.clone(), root.issues.len(), notices.len());
        let saved = db.transaction().map_err(|e| db_error(&e)).and_then(|tx| {
            for outcome in batch.drain(..) {
                writer.apply(&tx, root, notices, outcome)?;
            }
            tx.commit().map_err(|e| db_error(&e))
        });
        if let Err(error) = saved {
            batch.clear();
            root.progress = progress;
            root.issues.truncate(issues);
            notices.truncate(pending);
            return Err(error);
        }
        if self.notices.len() >= 64 {
            self.flush_notices()?;
        }
        Ok(())
    }

    /// Large files: the bounded streaming text reader, else the ordinary path, on the
    /// writer thread because both write as they read.
    fn stream(&mut self, job: Job, shared: &Shared<'_>) -> Result<(), IndexError> {
        let Job {
            mut file,
            name,
            path,
            metadata,
        } = job;
        let bytes_before = self.root.progress.bytes_read;
        let result = self.file(&mut file, &name, &path, &metadata);
        let streamed = self.root.progress.bytes_read.saturating_sub(bytes_before);
        shared.bytes_read.fetch_add(streamed, Relaxed);
        if let Err(error) = result {
            // Cancellation is not a failed replacement: retain the previous version.
            if self.observer.cancelled() {
                return Ok(());
            }
            self.root.progress.failed += 1;
            self.notice(NoticeKind::Failed, &path, &error.to_string())?;
            let writer = Writer::of(self);
            let tx = self.db.transaction().map_err(|e| db_error(&e))?;
            writer.metadata_only(
                &tx,
                &path,
                &metadata,
                ExtractionState::Failed,
                &error.to_string(),
            )?;
            tx.commit().map_err(|e| db_error(&e))?;
        }
        // A file cancelled mid-stream was not saved, so it has not settled.
        if !self.observer.cancelled() {
            settle(self.root, &path, None);
        }
        Ok(())
    }

    fn file(
        &mut self,
        file: &mut cap_std::fs::File,
        name: &str,
        path: &str,
        before: &Metadata,
    ) -> Result<(), IndexError> {
        if self.stream_text(file, path, before)? {
            return Ok(());
        }
        let budget = self.root.scan_options.documents;
        let source_limit = budget.source_bytes();
        if before.len() > source_limit {
            let note = format!(
                "This file exceeds the folder's {} MiB document reading budget. Its filename and size are searchable; its contents have not been read. Open Folder settings → Document reading to choose a larger budget, or split the document. Streaming source text has no file-size cutoff.",
                source_limit / (1024 * 1024)
            );
            let writer = Writer::of(self);
            let tx = self.db.transaction().map_err(|e| db_error(&e))?;
            writer.metadata_only(&tx, path, before, ExtractionState::MetadataOnly, &note)?;
            tx.commit().map_err(|e| db_error(&e))?;
            self.root.progress.skipped += 1;
            self.notice(NoticeKind::Unreadable, path, &note)?;
            return Ok(());
        }
        let mut bytes = Vec::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        let mut reader = file.by_ref().take(source_limit + 1);
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
        let read = ReadFile {
            name: name.to_owned(),
            path: path.to_owned(),
            metadata: before.clone(),
            content_hash: digest(&bytes),
            prepared: Some(prepare(path, name, &bytes, budget, Some(self.observer))?),
            bytes: None,
            _reservation: None,
        };
        if self.observer.cancelled() {
            return Ok(());
        }
        let writer = Writer::of(self);
        let Self {
            db, root, notices, ..
        } = self;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        writer.read(&tx, root, notices, read)?;
        tx.commit().map_err(|e| db_error(&e))
    }

    fn file_id(&self, path: &str, physical: &str) -> Result<String, IndexError> {
        file_id(
            self.db,
            &self.root.id,
            &self.root.path,
            self.generation,
            path,
            physical,
        )
    }
}

/// The writer's view of one scan: everything needed to save an outcome in a transaction.
struct Writer {
    root_id: String,
    root_path: PathBuf,
    generation: i64,
    activity: i64,
}

impl Writer {
    fn of(scan: &Scan<'_>) -> Self {
        Self {
            root_id: scan.root.id.clone(),
            root_path: scan.root.path.clone(),
            generation: scan.generation,
            activity: scan.activity,
        }
    }

    fn apply(
        &self,
        tx: &Connection,
        root: &mut IndexedRoot,
        notices: &mut Vec<super::report::PendingNotice>,
        outcome: Outcome,
    ) -> Result<(), IndexError> {
        match outcome {
            Outcome::Notice {
                kind,
                path,
                message,
                tally,
            } => {
                count(root, tally);
                note(root, notices, kind, &path, &message);
            }
            Outcome::Metadata {
                path,
                metadata,
                state,
                note: detail,
                notice,
                tally,
            } => {
                count(root, tally);
                note(root, notices, notice, &path, &detail);
                self.metadata_only(tx, &path, &metadata, state, &detail)?;
                settle(root, &path, None);
            }
            Outcome::Read(read) => {
                let path = read.path.clone();
                let metadata = read.metadata.clone();
                // Each file is atomic inside the batch: a failure rolls back only its rows.
                tx.execute_batch("SAVEPOINT scan_file")
                    .map_err(|e| db_error(&e))?;
                match self.read(tx, root, notices, *read) {
                    Ok(media_type) => {
                        tx.execute_batch("RELEASE scan_file")
                            .map_err(|e| db_error(&e))?;
                        settle(root, &path, media_type.as_deref());
                    }
                    Err(error) => {
                        tx.execute_batch("ROLLBACK TO scan_file; RELEASE scan_file")
                            .map_err(|e| db_error(&e))?;
                        root.progress.failed += 1;
                        note(root, notices, NoticeKind::Failed, &path, &error.0);
                        // Like any failed read: keep its metadata in this generation so a
                        // completed walk never mistakes a present file for a removed one.
                        self.metadata_only(
                            tx,
                            &path,
                            &metadata,
                            ExtractionState::Failed,
                            &error.0,
                        )?;
                        settle(root, &path, None);
                    }
                }
            }
            Outcome::Stream(_) => unreachable!("streams are handled outside batches"),
        }
        Ok(())
    }

    fn metadata_only(
        &self,
        tx: &Connection,
        path: &str,
        metadata: &Metadata,
        state: ExtractionState,
        note: &str,
    ) -> Result<(), IndexError> {
        let physical = identity(metadata);
        let id = file_id(
            tx,
            &self.root_id,
            &self.root_path,
            self.generation,
            path,
            &physical,
        )?;
        tx.execute("INSERT INTO index_files(id,root_id,path,identity,current_version,generation,available) VALUES(?1,?2,?3,?4,NULL,?5,1) ON CONFLICT(id) DO UPDATE SET path=excluded.path,identity=excluded.identity,current_version=NULL,generation=excluded.generation,available=1",
            params![id,self.root_id,path,physical,self.generation]).map_err(|e| db_error(&e))?;
        save_metadata(tx, &id, path, metadata, state, Some(note))?;
        super::history::observe(tx, &id, self.activity)
    }

    /// Save a read file; returns its media type for classification. The writer's view
    /// of the database is authoritative: a reader that skipped extraction because a
    /// version looked known is re-checked here.
    fn read(
        &self,
        tx: &Connection,
        root: &mut IndexedRoot,
        notices: &mut Vec<super::report::PendingNotice>,
        read: ReadFile,
    ) -> Result<Option<String>, IndexError> {
        let ReadFile {
            name,
            path,
            metadata: before,
            content_hash,
            prepared,
            bytes,
            _reservation,
        } = read;
        let budget = root.scan_options.documents;
        let physical = identity(&before);
        let file_id = file_id(
            tx,
            &self.root_id,
            &self.root_path,
            self.generation,
            &path,
            &physical,
        )?;
        let version = version_id(budget, &file_id, &content_hash);
        let known: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM index_versions WHERE id=?1)",
                [&version],
                |r| r.get(0),
            )
            .map_err(|error| db_error(&error))?;
        let prepared = match (known, prepared) {
            (true, _) => None,
            (false, Some(prepared)) => Some(prepared),
            // The reader judged from committed state (e.g. before a rename in this batch)
            // that the version was saved; the writer disagrees, so it extracts after all.
            (false, None) => match bytes {
                Some(bytes) => Some(prepare(&path, &name, &bytes, budget, None)?),
                None => {
                    return Err(IndexError(
                        "This file changed while being read. Refresh the folder to retry.".into(),
                    ));
                }
            },
        };
        let modified = before
            .modified()
            .ok()
            .and_then(|t| t.into_std().duration_since(std::time::UNIX_EPOCH).ok())
            .map(|t| t.as_secs());
        tx.execute("INSERT INTO index_files(id,root_id,path,identity,current_version,generation,available) VALUES(?1,?2,?3,?4,?5,?6,1) ON CONFLICT(id) DO UPDATE SET path=excluded.path,identity=excluded.identity,current_version=excluded.current_version,generation=excluded.generation,available=1",
            params![file_id, self.root_id, path, physical, version, self.generation]).map_err(|error| db_error(&error))?;
        let (state, detail, media_type) = if let Some(prepared) = prepared {
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
            (state, note, Some(prepared.attachment.mime_type))
        } else {
            let (has_text, note, partial, mime): (bool, Option<String>, bool, Option<String>) = tx.query_row("SELECT EXISTS(SELECT 1 FROM index_version_chunks WHERE version_id=?1),note,partial,mime FROM index_versions WHERE id=?1", [&version], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e| db_error(&e))?;
            (
                if has_text && partial {
                    ExtractionState::PartialText
                } else if has_text {
                    ExtractionState::Text
                } else {
                    ExtractionState::MetadataOnly
                },
                note,
                mime,
            )
        };
        save_metadata(tx, &file_id, &path, &before, state, detail.as_deref())?;
        super::history::observe(tx, &file_id, self.activity)?;
        if state == ExtractionState::MetadataOnly {
            note(
                root,
                notices,
                NoticeKind::Unreadable,
                &path,
                detail
                    .as_deref()
                    .unwrap_or("No searchable text was extracted."),
            );
        }
        if state == ExtractionState::PartialText {
            note(
                root,
                notices,
                NoticeKind::Partial,
                &path,
                detail
                    .as_deref()
                    .unwrap_or("Some content could not be read. Check the original file."),
            );
        }
        if known {
            root.progress.reused += 1;
        } else {
            root.progress.indexed += 1;
        }
        Ok(media_type)
    }
}

/// Extract with panic isolation: a decoder bug on one damaged file becomes that file's
/// "couldn't be read" notice instead of ending the scan of the rest.
fn prepare(
    label: &str,
    name: &str,
    bytes: &[u8],
    budget: DocumentBudget,
    observer: Option<&dyn ScanObserver>,
) -> Result<crate::PreparedFile, IndexError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::prepare_indexed_bytes(label, name, bytes, budget, observer)
    }))
    .map_err(|_| {
        IndexError(
            "This file could not be read safely. It may be damaged; open it in its own app to check."
                .into(),
        )
    })?
    .map_err(|e| IndexError(e.to_string()))
}

fn note(
    root: &mut IndexedRoot,
    notices: &mut Vec<super::report::PendingNotice>,
    kind: NoticeKind,
    path: &str,
    message: &str,
) {
    issue(root, format!("{path}: {message}"));
    notices.push(super::report::PendingNotice {
        kind,
        path: path.into(),
        message: message.into(),
    });
}

fn count(root: &mut IndexedRoot, tally: Tally) {
    match tally {
        Tally::Skipped => root.progress.skipped += 1,
        Tally::Failed => root.progress.failed += 1,
    }
}

/// A discovered file's outcome is final: count it, classify it, show it as "now reading".
fn settle(root: &mut IndexedRoot, path: &str, media_type: Option<&str>) {
    let progress = &mut root.progress;
    progress.handled += 1;
    *progress
        .kinds
        .entry(ontology::classify(media_type, path))
        .or_default() += 1;
    if let Some(area) = ontology::life_area(path) {
        *progress.areas.entry(area).or_default() += 1;
    }
    progress.current = Some(path.to_owned());
}

/// Decoder changes create new snapshots on refresh, never relabel old citations or
/// reuse text produced with outdated format/size handling.
fn version_id(budget: DocumentBudget, file_id: &str, content_hash: &str) -> String {
    digest(
        format!(
            "{}\0{budget:?}\0{file_id}\0{content_hash}",
            super::history::EXTRACTOR_REVISION
        )
        .as_bytes(),
    )
}

fn file_id(
    db: &Connection,
    root_id: &str,
    root_path: &Path,
    generation: i64,
    path: &str,
    physical: &str,
) -> Result<String, IndexError> {
    if let Some(id) = db
        .query_row(
            "SELECT id FROM index_files WHERE root_id=?1 AND path=?2",
            params![root_id, path],
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
        let mut query = db
            .prepare("SELECT id,path FROM index_files WHERE root_id=?1 AND identity=?2 LIMIT 2")
            .map_err(|error| db_error(&error))?;
        let candidates = query
            .query_map(params![root_id, physical], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|error| db_error(&error))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| db_error(&error))?;
        if let [(id, old_path)] = candidates.as_slice() {
            let root = Dir::open_ambient_dir(root_path, cap_std::ambient_authority())
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
        format!("{root_id}\0{path}\0{physical}\0{generation}").as_bytes(),
    ))
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

fn escaped_path(path: &Path) -> String {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .copied()
        .flat_map(std::ascii::escape_default)
        .map(char::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn escaped_paths_preserve_distinct_non_unicode_names_without_filesystem_support() {
        use std::os::unix::ffi::OsStringExt as _;
        let paths = [0xfe, 0xff].map(|byte| {
            let name = std::ffi::OsString::from_vec(vec![b'n', byte]);
            escaped_path(Path::new(&name))
        });
        assert_eq!(paths, ["n\\xfe", "n\\xff"]);
    }

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
