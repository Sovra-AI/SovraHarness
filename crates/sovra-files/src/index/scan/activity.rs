//! Send elapsed time and actual in-flight paths even while the writer is inside a slow operation.
use super::{
    Arc, AtomicU64, Duration, IndexedRoot, Instant, Mutex, RecvTimeoutError, Relaxed, ScanObserver,
};
use sovra_core::file_index::{
    ScanCancellation, ScanCancellationWatch,
    live::{ScanActivity, ScanPhase},
};
use std::sync::{MutexGuard, mpsc};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Work {
    phase: ScanPhase,
    path: Option<String>,
    since: Instant,
    passage: Option<u64>,
}

struct Clear<'a>(&'a Mutex<Vec<Option<Work>>>, usize);

impl Drop for Clear<'_> {
    fn drop(&mut self) {
        lock(self.0)[self.1] = None;
    }
}

struct Snapshot {
    saved: IndexedRoot,
    changed: Instant,
    revision: u64,
    last_sent: Instant,
}

pub(super) struct Live<'a> {
    observer: &'a dyn ScanObserver,
    started: Instant,
    snapshot: Mutex<Snapshot>,
    // Slot zero is the one writer. Remaining slots belong to the parallel readers.
    slots: Mutex<Vec<Option<Work>>>,
    revision: AtomicU64,
}

impl<'a> Live<'a> {
    pub(super) fn new(observer: &'a dyn ScanObserver, root: &IndexedRoot, readers: usize) -> Self {
        let started = Instant::now();
        Self {
            observer,
            started,
            snapshot: Mutex::new(Snapshot {
                saved: root.clone(),
                changed: started,
                revision: 0,
                last_sent: started,
            }),
            slots: Mutex::new((0..=readers).map(|_| None).collect()),
            revision: AtomicU64::new(0),
        }
    }

    pub(super) fn during<T>(&self, operation: impl FnOnce() -> T) -> T {
        let (done, stop) = mpsc::channel::<()>();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                while matches!(
                    stop.recv_timeout(Duration::from_secs(1)),
                    Err(RecvTimeoutError::Timeout)
                ) {
                    let mut state = lock(&self.snapshot);
                    if state.last_sent.elapsed() >= Duration::from_millis(900) && !self.cancelled()
                    {
                        self.send(&mut state);
                    }
                }
            });
            // On unwind the sender also drops before the scoped thread is joined.
            let _done = done;
            operation()
        })
    }

    pub(super) fn working(
        &self,
        slot: usize,
        phase: ScanPhase,
        path: Option<&str>,
    ) -> impl Drop + use<'_> {
        lock(&self.slots)[slot] = Some(Work {
            phase,
            path: path.map(str::to_owned),
            since: Instant::now(),
            passage: None,
        });
        self.revision.fetch_add(1, Relaxed);
        Clear(&self.slots, slot)
    }

    pub(super) fn saving_passage(&self, number: u64) {
        if let Some(work) = &mut lock(&self.slots)[0] {
            work.passage = Some(number);
            self.revision.fetch_add(1, Relaxed);
        }
    }

    fn send(&self, state: &mut Snapshot) {
        let now = Instant::now();
        let revision = self.revision.load(Relaxed);
        if state.revision != revision {
            state.changed = now;
            state.revision = revision;
        }
        let work = lock(&self.slots);
        let current = work[0]
            .as_ref()
            .or_else(|| work.iter().flatten().min_by_key(|w| w.since));
        let mut root = state.saved.clone();
        root.progress.activity = Some(ScanActivity {
            phase: current.map_or(ScanPhase::Discovering, |w| w.phase),
            path: current.and_then(|w| w.path.clone()),
            passage: current.and_then(|w| w.passage),
            seconds_without_change: now.duration_since(state.changed).as_secs(),
        });
        drop(work);
        root.progress.elapsed_ms =
            u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        state.last_sent = now;
        self.observer.progress(&root);
    }
}

impl ScanObserver for Live<'_> {
    fn cancellation(&self) -> Option<ScanCancellation> {
        self.observer.cancellation()
    }
    fn watch_cancellation(
        &self,
        signal: Arc<dyn Fn() + Send + Sync>,
    ) -> Option<Box<dyn ScanCancellationWatch>> {
        self.observer.watch_cancellation(signal)
    }
    fn progress(&self, root: &IndexedRoot) {
        let mut state = lock(&self.snapshot);
        let old = &state.saved.progress;
        let new = &root.progress;
        if (old.handled, old.visited, old.discovered, old.bytes_read)
            != (new.handled, new.visited, new.discovered, new.bytes_read)
        {
            state.changed = Instant::now();
        }
        state.saved.clone_from(root);
        self.send(&mut state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::file_index::{IndexProgress, IndexStatus, ScanOptions};

    struct Observe(mpsc::Sender<IndexedRoot>);
    impl ScanObserver for Observe {
        fn cancellation(&self) -> Option<ScanCancellation> {
            None
        }
        fn progress(&self, root: &IndexedRoot) {
            self.0.send(root.clone()).unwrap();
        }
    }

    #[test]
    fn long_write_reports_the_active_path_and_passage_before_any_file_finishes() {
        let root = IndexedRoot {
            id: "root".into(),
            path: "/fixture".into(),
            status: IndexStatus::Scanning,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: None,
            auto_refresh: true,
            scan_options: ScanOptions::default(),
        };
        let (tx, rx) = mpsc::channel();
        let observe = Observe(tx);
        let live = Live::new(&observe, &root, 1);
        live.during(|| {
            let _reader = live.working(1, ScanPhase::Reading, Some("other.txt"));
            let _writer = live.working(0, ScanPhase::Saving, Some("books/long.txt"));
            live.saving_passage(19);
            let event = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("independent progress while writer is busy");
            let a = event.progress.activity.unwrap();
            assert_eq!(a.phase, ScanPhase::Saving);
            assert_eq!(a.path.as_deref(), Some("books/long.txt"));
            assert_eq!(a.passage, Some(19));
            assert!(event.progress.elapsed_ms >= 900);
            assert_eq!(event.progress.handled, 0);
        });
        assert!(rx.try_recv().is_err());
    }
}
