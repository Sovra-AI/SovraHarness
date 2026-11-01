//! Stage 2: readers. Several threads take opened files from the walker, read them within
//! the folder's document budget, hash them and run the (CPU-heavy) extractors. They never
//! write: results go to the single writer, because SQLite has one writer at a time and
//! batching there is what makes the database side fast.
//!
//! Each reader holds a read-only connection (WAL allows concurrent readers) to skip
//! extraction when an identical version is already saved. A byte reservation travels with
//! each result until the writer has saved it, sized to the larger of the source and the
//! extracted text, so everything in flight (readers, channel, batch) stays under one cap.

use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use cap_std::fs::Metadata;
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, params};
use sovra_core::file_index::{
    DocumentBudget, ExtractionState, IndexError, IndexedRoot, NoticeKind, ScanCancellation,
    ScanCancellationWatch, ScanObserver,
};

use super::{Job, Outcome, ReadFile, Shared, Tally, identity};
use crate::index::{digest, io_error};

/// What every reader needs to know about the folder being scanned.
pub(super) struct ReadContext {
    pub root_id: String,
    pub root_path: std::path::PathBuf,
    pub generation: i64,
    pub budget: DocumentBudget,
    pub database: std::path::PathBuf,
}

const POLL: Duration = Duration::from_millis(100);

/// Reader thread body: runs until the walk is finished and drained, or the scan stops.
pub(super) fn work(
    context: &ReadContext,
    shared: &Shared<'_>,
    jobs: &Mutex<Receiver<Job>>,
    out: &SyncSender<Outcome>,
) {
    // Without a reader connection every file is simply extracted; correctness is kept by
    // the writer, which re-checks before saving.
    let db = open_reader(&context.database).ok();
    loop {
        if shared.stopped() {
            return;
        }
        let job = {
            let Ok(receiver) = jobs.lock() else { return };
            match receiver.recv_timeout(POLL) {
                Ok(job) => job,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        if let Some(outcome) = read(context, shared, db.as_ref(), job)
            && out.send(outcome).is_err()
        {
            return;
        }
    }
}

fn open_reader(path: &Path) -> rusqlite::Result<Connection> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    Ok(db)
}

/// `None` means the scan stopped mid-file: nothing is reported, the old version stays.
fn read(
    context: &ReadContext,
    shared: &Shared<'_>,
    db: Option<&Connection>,
    job: Job,
) -> Option<Outcome> {
    if let Some(digest) = db.and_then(|db| unchanged(context, db, &job.path, &job.metadata)) {
        return Some(Outcome::Read(Box::new(ReadFile {
            name: job.name,
            path: job.path,
            metadata: job.metadata,
            content_hash: digest,
            prepared: None,
            bytes: None,
            _reservation: None,
        })));
    }
    // Legacy single-byte text can triple when decoded to UTF-8, so large *text* goes to
    // the streaming reader (on the writer) before the attachment decoder's budget applies.
    // Large PDFs, office files, media and archives stay here, in parallel.
    let mut job = job;
    if job.metadata.len() >= (sovra_core::files::MAX_DOCUMENT_BYTES / 3) as u64
        && looks_like_text(&mut job)
    {
        return Some(Outcome::Stream(job));
    }
    let source_limit = context.budget.source_bytes();
    if job.metadata.len() > source_limit {
        let note = format!(
            "This file exceeds the folder's {} MiB document reading budget. Its filename and size are searchable; its contents have not been read. Open Folder settings → Document reading to choose a larger budget, or split the document. Streaming source text has no file-size cutoff.",
            source_limit / (1024 * 1024)
        );
        return Some(Outcome::Metadata {
            path: job.path,
            metadata: job.metadata,
            state: ExtractionState::MetadataOnly,
            note,
            notice: NoticeKind::Unreadable,
            tally: Tally::Skipped,
        });
    }
    let path = job.path.clone();
    let metadata = job.metadata.clone();
    match read_and_prepare(context, shared, db, job) {
        Ok(read) => read.map(|read| Outcome::Read(Box::new(read))),
        // Cancellation is not a failed read: keep the previous version.
        Err(_) if shared.stopped() => None,
        Err(error) => Some(Outcome::Metadata {
            path,
            metadata,
            state: ExtractionState::Failed,
            note: error.to_string(),
            notice: NoticeKind::Failed,
            tally: Tally::Failed,
        }),
    }
}

/// The saved digest when a file provably has not changed since it was last read, so it
/// need not be read again (the incremental check git, Baloo and Spotlight rely on).
///
/// Same path, filesystem identity, size and modification second, a non-failed reading by
/// the current extractor with the folder's current document budget — and, as git's
/// "racy timestamp" guard, a modification second strictly before the last observation, so
/// an edit made in the same second as the previous scan is never missed.
fn unchanged(
    context: &ReadContext,
    db: &Connection,
    path: &str,
    before: &Metadata,
) -> Option<String> {
    let modified = before
        .modified()
        .ok()
        .and_then(|t| t.into_std().duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|t| i64::try_from(t.as_secs()).ok())?;
    let size = i64::try_from(before.len()).ok()?;
    let (file_id, version, digest): (String, String, String) = db
        .query_row(
            "SELECT f.id, f.current_version, v.digest FROM index_files f
             JOIN index_versions v ON v.id = f.current_version
             WHERE f.root_id=?1 AND f.path=?2 AND f.identity=?3 AND f.available=1
               AND f.byte_size=?4 AND f.modified=?5 AND f.observed>?5
               AND f.extraction<>'failed' AND v.extractor_revision=?6",
            params![
                context.root_id,
                path,
                identity(before),
                size,
                modified,
                crate::index::history::EXTRACTOR_REVISION
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .ok()
        .flatten()?;
    // A changed document budget yields a different version: read it again.
    (super::version_id(context.budget, &file_id, &digest) == version).then_some(digest)
}

/// Lets extractors notice a writer failure as well as the person pressing Stop, so a
/// failed scan does not keep decoding files nobody will save.
struct StopAware<'a, 's>(&'a Shared<'s>);

impl ScanObserver for StopAware<'_, '_> {
    fn cancellation(&self) -> Option<ScanCancellation> {
        if self.0.abort.load(Relaxed) {
            Some(ScanCancellation::Interrupted)
        } else {
            self.0.observer.cancellation()
        }
    }

    fn progress(&self, _: &IndexedRoot) {}

    fn watch_cancellation(
        &self,
        signal: Arc<dyn Fn() + Send + Sync>,
    ) -> Option<Box<dyn ScanCancellationWatch>> {
        self.0.observer.watch_cancellation(signal)
    }
}

/// Sniff the first block: text (or unknown bytes the streaming reader may decode) streams;
/// recognised binary formats do not. The file is rewound either way.
fn looks_like_text(job: &mut Job) -> bool {
    let mut prefix = vec![0_u8; 8 * 1024];
    let read = job.file.read(&mut prefix).unwrap_or(0);
    prefix.truncate(read);
    if job.file.seek(SeekFrom::Start(0)).is_err() {
        return true;
    }
    let mime = crate::detection::mime(&job.name, &prefix);
    mime.starts_with("text/")
        || mime == "application/octet-stream"
        || crate::detection::formatted_text(&mime, &prefix)
}

fn read_and_prepare(
    context: &ReadContext,
    shared: &Shared<'_>,
    db: Option<&Connection>,
    job: Job,
) -> Result<Option<ReadFile>, IndexError> {
    let Job {
        mut file,
        name,
        path,
        metadata: before,
    } = job;
    let source_limit = context.budget.source_bytes();
    let Some(mut reservation) = shared.reserve(before.len()) else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut reader = file.by_ref().take(source_limit + 1);
    loop {
        if shared.stopped() {
            return Ok(None);
        }
        let count = reader.read(&mut buffer).map_err(|e| io_error(&e))?;
        if count == 0 {
            break;
        }
        bytes.try_reserve(count).map_err(|_| IndexError("Not enough memory to read this document. Choose a lower document budget or retry after closing other apps.".into()))?;
        bytes.extend_from_slice(&buffer[..count]);
        shared.bytes_read.fetch_add(count as u64, Relaxed);
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
    let content_hash = digest(&bytes);
    let physical = identity(&before);
    // A reader-side lookup failure (busy database, checkpoint) only costs an extraction;
    // it must never turn a readable file into a failed one.
    let file_id = db.and_then(|db| {
        super::file_id(
            db,
            &context.root_id,
            &context.root_path,
            context.generation,
            &path,
            &physical,
        )
        .ok()
    });
    let known = match (db, &file_id) {
        (Some(db), Some(file_id)) => db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM index_versions WHERE id=?1)",
                [super::version_id(context.budget, file_id, &content_hash)],
                |r| r.get(0),
            )
            .unwrap_or(false),
        _ => false,
    };
    if shared.stopped() {
        return Ok(None);
    }
    let prepared = if known {
        None
    } else {
        let label = file_id.as_deref().unwrap_or(&path);
        Some(super::prepare(
            label,
            &name,
            &bytes,
            context.budget,
            Some(&StopAware(shared)),
        )?)
    };
    // Extractors may return partial output when cancelled; never let it replace a version.
    if shared.stopped() {
        return Ok(None);
    }
    let output = prepared
        .as_ref()
        .map_or(0, |p| p.content.text.len() as u64)
        .max(bytes.len() as u64);
    reservation.grow_to(output);
    Ok(Some(ReadFile {
        name,
        path,
        metadata: before,
        content_hash,
        // Only kept when extraction was skipped, so the writer can extract after all if
        // its authoritative view disagrees that this version is already saved.
        bytes: prepared.is_none().then_some(bytes),
        prepared,
        _reservation: Some(reservation),
    }))
}

/// Caps source bytes held by all readers at once. A single file larger than the cap may
/// still proceed alone, so no file is starved.
#[derive(Debug)]
pub(super) struct ByteBudget {
    /// Signed: output growth may overdraw it (see [`Reservation::grow_to`]).
    free: Mutex<i64>,
    freed: std::sync::Condvar,
    capacity: u64,
}

impl ByteBudget {
    pub(super) fn new(capacity: u64) -> Self {
        Self {
            free: Mutex::new(signed(capacity)),
            freed: std::sync::Condvar::new(),
            capacity,
        }
    }

    fn lock(&self) -> Option<MutexGuard<'_, i64>> {
        self.free.lock().ok()
    }
}

/// Bytes held against the budget; released when dropped (after the writer saves).
pub(super) struct Reservation {
    budget: Arc<ByteBudget>,
    bytes: u64,
}

impl Reservation {
    /// Charge extracted output that outgrew the source. Never waits — readers waiting on
    /// each other could deadlock — so the budget may briefly go into overdraft, which
    /// holds back new files until the writer has saved enough.
    fn grow_to(&mut self, total: u64) {
        let total = total.min(self.budget.capacity);
        if total <= self.bytes {
            return;
        }
        if let Some(mut free) = self.budget.lock() {
            *free -= signed(total - self.bytes);
            self.bytes = total;
        }
    }
}

fn signed(bytes: u64) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(mut free) = self.budget.lock() {
            *free += signed(self.bytes);
        }
        self.budget.freed.notify_all();
    }
}

impl Shared<'_> {
    /// Wait for room for `bytes`; `None` if the scan stops while waiting.
    pub(super) fn reserve(&self, bytes: u64) -> Option<Reservation> {
        let budget = &self.budget;
        let want = bytes.min(budget.capacity);
        let mut free = budget.lock()?;
        while *free < signed(want) {
            if self.stopped() {
                return None;
            }
            free = budget.freed.wait_timeout(free, POLL).ok()?.0;
        }
        *free -= signed(want);
        drop(free);
        Some(Reservation {
            budget: Arc::clone(budget),
            bytes: want,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::file_index::{IndexedRoot, ScanCancellation, ScanObserver};

    struct Observer;
    impl ScanObserver for Observer {
        fn cancellation(&self) -> Option<ScanCancellation> {
            None
        }
        fn progress(&self, _: &IndexedRoot) {}
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test must hold a reservation long enough to observe blocking"
    )]
    fn byte_budget_blocks_until_room_and_lets_an_oversized_file_through_alone() {
        let budget = ByteBudget::new(100);
        let observer = Observer;
        let shared = Shared::new(&observer, budget);
        let first = shared.reserve(60).expect("room");
        let big = std::thread::scope(|scope| {
            let waiter = scope.spawn(|| shared.reserve(1_000).map(|r| r.bytes));
            std::thread::sleep(Duration::from_millis(150));
            assert!(!waiter.is_finished(), "must wait while 60 bytes are held");
            drop(first);
            waiter.join().expect("waiter thread")
        });
        assert_eq!(big, Some(100), "oversized files take the whole budget");
        assert_eq!(
            *shared.budget.lock().expect("lock"),
            100,
            "released on drop"
        );
    }

    #[test]
    fn output_growth_overdraws_without_waiting_and_holds_back_new_files() {
        let observer = Observer;
        let shared = Shared::new(&observer, ByteBudget::new(100));
        let mut first = shared.reserve(10).expect("room");
        first.grow_to(150);
        assert_eq!(first.bytes, 100, "growth is capped at the budget");
        assert_eq!(*shared.budget.lock().expect("lock"), 0);
        let mut second = shared.reserve(0).expect("zero-byte files never wait");
        second.grow_to(30);
        assert_eq!(
            *shared.budget.lock().expect("lock"),
            -30,
            "overdraft, no deadlock"
        );
        drop((first, second));
        assert_eq!(*shared.budget.lock().expect("lock"), 100);
    }
}
