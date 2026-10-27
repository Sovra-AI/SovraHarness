//! Own native guards and deterministic refresh schedules. Native callbacks only
//! set bounded signals; slow registration/disposal stays on blocking workers.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use async_channel::Sender;
use sovra_core::{
    Event,
    file_index::{
        FileIndex, IndexChanges, IndexEvent, IndexWatch, IndexedRoot, MonitorMode, MonitorStatus,
        RefreshSchedule,
    },
};

#[derive(Default)]
struct Signal {
    dirty: AtomicBool,
    error: Mutex<Option<String>>,
}
impl IndexChanges for Signal {
    fn changed(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }
    fn failed(&self, message: String) {
        *self
            .error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message);
        self.changed();
    }
}

struct Scope {
    guard: Option<Box<dyn IndexWatch>>,
    signal: Arc<Signal>,
    schedule: RefreshSchedule,
}
impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(guard) = self.guard.take() {
            tokio::task::spawn_blocking(move || drop(guard));
        }
    }
}

pub(super) struct Monitors {
    scopes: BTreeMap<String, Scope>,
    paused: BTreeSet<String>,
    clock: Instant,
    events: Sender<Event>,
}

impl Monitors {
    pub(super) fn new(events: Sender<Event>) -> Self {
        Self {
            scopes: BTreeMap::new(),
            paused: BTreeSet::new(),
            clock: Instant::now(),
            events,
        }
    }

    fn now(&self) -> u64 {
        u64::try_from(self.clock.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    pub(super) fn set_enabled(&mut self, id: &str, enabled: bool) {
        if enabled {
            self.paused.remove(id);
        } else {
            // Honour Pause even if a full disk prevents saving its preference.
            self.paused.insert(id.into());
            self.scopes.remove(id);
            self.publish(id, MonitorMode::Paused, None);
        }
    }

    pub(super) async fn sync(
        &mut self,
        store: &Arc<dyn FileIndex>,
        roots: &[IndexedRoot],
        scanned: Option<&str>,
    ) {
        self.paused
            .retain(|id| roots.iter().any(|root| root.id == *id));
        self.scopes
            .retain(|id, _| roots.iter().any(|root| root.id == *id && root.auto_refresh));
        for root in roots {
            if !root.auto_refresh || self.paused.contains(&root.id) {
                self.publish(&root.id, MonitorMode::Paused, None);
                continue;
            }
            if self.scopes.contains_key(&root.id) && scanned != Some(root.id.as_str()) {
                continue;
            }
            let now = self.now();
            let mut scope = self.scopes.remove(&root.id).unwrap_or_else(|| Scope {
                guard: None,
                signal: Arc::new(Signal::default()),
                schedule: RefreshSchedule::new(now, scanned != Some(root.id.as_str())),
            });
            if scanned == Some(root.id.as_str()) {
                // Includes the first scan immediately after a folder is added.
                scope.schedule.started(now);
            }
            let work_store = store.clone();
            let work_root = root.clone();
            let signal = scope.signal.clone();
            let old = scope.guard.take();
            // Install the replacement before dropping the old guard. A scan may
            // have discovered directories that did not exist at registration.
            let result = tokio::task::spawn_blocking(move || {
                let result = work_store.watch(&work_root, signal);
                drop(old);
                result
            })
            .await;
            match result {
                Ok(Ok(guard)) => {
                    scope.guard = Some(guard);
                    self.publish(&root.id, MonitorMode::Native, None);
                }
                Ok(Err(error)) => {
                    self.publish(&root.id, MonitorMode::Periodic, Some(error.to_string()));
                }
                Err(_) => self.publish(
                    &root.id,
                    MonitorMode::Periodic,
                    Some(
                        "Folder notifications could not start. Periodic checks remain active."
                            .into(),
                    ),
                ),
            }
            self.scopes.insert(root.id.clone(), scope);
        }
    }

    pub(super) fn started(&mut self, id: &str) {
        let now = self.now();
        if let Some(scope) = self.scopes.get_mut(id) {
            scope.signal.dirty.swap(false, Ordering::Relaxed);
            scope.schedule.started(now);
        }
    }

    pub(super) fn due(&mut self) -> Option<String> {
        let now = self.now();
        let mut statuses = vec![];
        let mut due = None;
        for (id, scope) in &mut self.scopes {
            if scope.signal.dirty.swap(false, Ordering::Relaxed) {
                scope.schedule.changed(now);
            }
            if let Some(error) = scope
                .signal
                .error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                if let Some(guard) = scope.guard.take() {
                    tokio::task::spawn_blocking(move || drop(guard));
                }
                statuses.push((id.clone(), error));
            }
            if due.is_none() && scope.schedule.due(now) {
                due = Some(id.clone());
            }
        }
        for (id, error) in statuses {
            self.publish(&id, MonitorMode::Periodic, Some(error));
        }
        due
    }

    fn publish(&self, root_id: &str, mode: MonitorMode, detail: Option<String>) {
        let _ = self
            .events
            .try_send(Event::FileIndex(IndexEvent::Monitoring {
                root_id: root_id.into(),
                status: MonitorStatus { mode, detail },
            }));
    }
}
