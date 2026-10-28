//! User-granted folder scopes and bounded retrieval over indexed source versions.
//! Paths are display metadata; model reads use opaque version/chunk references.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
pub mod clustering;
mod options;
pub use options::{DocumentBudget, ScanOptions};

pub mod graph;
mod history;
mod refresh;
pub mod semantic;
pub mod summary;
pub use history::{
    Confidence, EvidenceBasis, EvidenceTier, FileHistory, FileHistoryQuery, FileObservation,
    HistoricalPassage, HistoryPassageRef, ObservedPresence, ScanActivity, SourceRepresentation,
    WorldValidity,
};
pub use refresh::RefreshSchedule;
mod browser;
mod catalogue;
pub use browser::BrowserState;
mod report;
pub use catalogue::{
    CatalogCoverage, CatalogCursor, CatalogEntry, CatalogFilter, CatalogGroup, CatalogGroupBy,
    CatalogPage, CatalogQuery, ExtractionState,
};
pub use report::{NoticeCursor, NoticeKind, NoticePage, NoticeQuery, ReportState, ScanNotice};

pub const MAX_INDEX_ROOTS: usize = 16;
pub const MAX_INDEX_ENTRIES: u64 = 100_000;
pub const MAX_INDEX_ISSUES: usize = 50;
pub const MAX_SEARCH_RESULTS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct IndexError(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexStatus {
    Pending,
    Scanning,
    Ready,
    Cancelled,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexProgress {
    pub visited: u64,
    pub indexed: u64,
    pub reused: u64,
    pub skipped: u64,
    pub failed: u64,
    pub removed: u64,
    pub bytes_read: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedRoot {
    pub id: String,
    pub path: PathBuf,
    pub status: IndexStatus,
    pub progress: IndexProgress,
    pub issues: Vec<String>,
    pub completed_at: Option<u64>,
    /// A persisted user preference, independent of the latest scan outcome.
    #[serde(default)]
    pub auto_refresh: bool,
    #[serde(default)]
    pub scan_options: ScanOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexQuery {
    pub text: String,
    /// Empty means every currently granted folder.
    pub roots: Vec<String>,
    pub path_prefix: Option<String>,
    pub limit: usize,
}

impl IndexQuery {
    pub fn validate(&self) -> Result<(), IndexError> {
        validate_search_text(&self.text)?;
        if self.limit == 0 || self.limit > MAX_SEARCH_RESULTS || self.roots.len() > MAX_INDEX_ROOTS
        {
            return Err(IndexError(
                "This search requests too many results or folders.".into(),
            ));
        }
        if let Some(prefix) = &self.path_prefix {
            validate_relative_path(prefix)?;
        }
        if self.roots.iter().any(|id| id.is_empty() || id.len() > 120) {
            return Err(IndexError("Invalid indexed folder reference.".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassageRef {
    pub root_id: String,
    pub file_id: String,
    pub version_id: String,
    pub ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexPassage {
    pub reference: PassageRef,
    pub path: String,
    pub text: String,
    pub start_line: u32,
    pub end_line: u32,
    pub mime_type: String,
    /// Extraction caveats travel with every citation.
    pub note: Option<String>,
    pub observed_at: u64,
    pub modified_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexCommand {
    Report {
        request_id: u64,
        query: NoticeQuery,
    },
    Browse {
        request_id: u64,
        query: CatalogQuery,
    },
    Preview {
        request_id: u64,
        reference: PassageRef,
    },
    ListKnowledge,
    ReviewKnowledge {
        proposal_id: u64,
        approve: bool,
    },
    List,
    AddRoot {
        path: PathBuf,
    },
    Refresh {
        root_id: String,
    },
    Cancel {
        root_id: String,
    },
    SetAutoRefresh {
        root_id: String,
        enabled: bool,
    },
    SetScanOptions {
        root_id: String,
        options: ScanOptions,
    },
    ConfigureSemantic {
        root_id: String,
        target: Option<semantic::EmbeddingTarget>,
    },
    Remove {
        root_id: String,
    },
    Search {
        request_id: u64,
        query: IndexQuery,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexEvent {
    Reported {
        request_id: u64,
        result: Result<NoticePage, IndexError>,
    },
    Browsed {
        request_id: u64,
        result: Result<CatalogPage, IndexError>,
    },
    Previewed {
        request_id: u64,
        result: Result<IndexPassage, IndexError>,
    },
    KnowledgePending(Vec<graph::KnowledgeProposal>),
    KnowledgeReviewed {
        proposal_id: u64,
        accepted: bool,
    },
    Semantic(Vec<semantic::SemanticStatus>),
    SearchNotice {
        request_id: u64,
        message: String,
    },
    Roots(Vec<IndexedRoot>),
    Progress(IndexedRoot),
    Monitoring {
        root_id: String,
        status: MonitorStatus,
    },
    Results {
        request_id: u64,
        passages: Vec<IndexPassage>,
    },
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorMode {
    Paused,
    Native,
    Periodic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorStatus {
    pub mode: MonitorMode,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexState {
    pub report: ReportState,
    pub browser: BrowserState,
    pub roots: Vec<IndexedRoot>,
    pub passages: Vec<IndexPassage>,
    pub request_id: u64,
    pub searching: bool,
    pub notice: Option<String>,
    pub monitors: std::collections::BTreeMap<String, MonitorStatus>,
    pub semantic: Vec<semantic::SemanticStatus>,
    pub knowledge_pending: Vec<graph::KnowledgeProposal>,
    pub knowledge_reviewing: std::collections::BTreeSet<u64>,
    knowledge_reviewed: std::collections::BTreeSet<u64>,
}

impl IndexState {
    /// A review starts only from a visible proposal and cannot be submitted twice.
    pub fn begin_review(&mut self, id: u64) -> bool {
        self.knowledge_pending.iter().any(|p| p.id == id) && self.knowledge_reviewing.insert(id)
    }

    pub fn begin_search(&mut self) -> u64 {
        self.request_id = self.request_id.saturating_add(1);
        self.searching = true;
        self.notice = None;
        self.request_id
    }

    pub fn apply(&mut self, event: &IndexEvent) {
        match event {
            IndexEvent::KnowledgePending(proposals) => {
                self.knowledge_pending = proposals
                    .iter()
                    .filter(|p| {
                        !self.knowledge_reviewed.contains(&p.id)
                            && p.state == graph::ProposalState::Pending
                            && p.required_roots
                                .iter()
                                .all(|id| self.roots.iter().any(|r| r.id == *id))
                    })
                    .cloned()
                    .collect();
            }
            IndexEvent::KnowledgeReviewed {
                proposal_id,
                accepted,
            } => {
                self.knowledge_pending.retain(|p| p.id != *proposal_id);
                self.knowledge_reviewing.remove(proposal_id);
                self.knowledge_reviewed.insert(*proposal_id);
                self.notice = Some(if *accepted {"Saved the reviewed inferences. Their sources and uncertainty remain attached."} else {"Rejected this knowledge proposal."}.into());
            }
            IndexEvent::Semantic(status) => self.semantic.clone_from(status),
            IndexEvent::SearchNotice {
                request_id,
                message,
            } => {
                if *request_id == self.request_id {
                    self.notice = Some(message.clone());
                }
            }
            IndexEvent::Roots(roots) => {
                if self.roots.iter().any(|old| {
                    !roots
                        .iter()
                        .any(|new| new.id == old.id && new.scan_options == old.scan_options)
                }) {
                    self.browser.invalidate();
                    self.report.invalidate();
                    self.passages.clear();
                }
                self.roots.clone_from(roots);
                self.monitors
                    .retain(|id, _| roots.iter().any(|root| root.id == *id));
                self.semantic
                    .retain(|s| roots.iter().any(|r| r.id == s.setting.root_id));
                self.passages
                    .retain(|p| roots.iter().any(|r| r.id == p.reference.root_id));
                self.knowledge_pending.retain(|p| {
                    p.required_roots
                        .iter()
                        .all(|id| roots.iter().any(|r| r.id == *id))
                });
                self.knowledge_reviewing
                    .retain(|id| self.knowledge_pending.iter().any(|p| p.id == *id));
            }
            IndexEvent::Progress(root) => {
                if let Some(current) = self.roots.iter_mut().find(|r| r.id == root.id) {
                    if current.scan_options != root.scan_options {
                        self.browser.invalidate();
                        self.report.invalidate();
                        self.passages.retain(|p| p.reference.root_id != root.id);
                    }
                    current.clone_from(root);
                } else {
                    self.roots.push(root.clone());
                }
                self.roots.sort_by(|a, b| a.path.cmp(&b.path));
            }
            IndexEvent::Monitoring { root_id, status } => {
                self.monitors.insert(root_id.clone(), status.clone());
            }
            IndexEvent::Results {
                request_id,
                passages,
            } => {
                if *request_id == self.request_id {
                    self.passages = passages
                        .iter()
                        .filter(|passage| {
                            self.roots
                                .iter()
                                .any(|root| root.id == passage.reference.root_id)
                        })
                        .cloned()
                        .collect();
                    self.searching = false;
                }
            }
            IndexEvent::Error(message) => {
                self.knowledge_reviewing.clear();
                self.notice = Some(message.clone());
                self.searching = false;
            }
            IndexEvent::Browsed { request_id, result } => {
                self.browser.finish_browse(*request_id, result);
            }
            IndexEvent::Reported { request_id, result } => self.report.finish(*request_id, result),
            IndexEvent::Previewed { request_id, result } => {
                self.browser.finish_preview(*request_id, result);
            }
        }
    }
}

/// A blocking scanner calls this between bounded units of work. Cancellation must
/// not interpret the unvisited remainder of a folder as deleted files.
pub trait ScanObserver: Send + Sync {
    fn cancellation(&self) -> Option<ScanCancellation>;
    fn cancelled(&self) -> bool {
        self.cancellation().is_some()
    }
    fn progress(&self, root: &IndexedRoot);

    /// Register a decoder's cooperative cancellation signal. The owner invokes
    /// it when this scan is cancelled/interrupted; dropping the returned guard
    /// releases the registration. The signal must be nonblocking. Synchronous
    /// observers may have no watcher.
    fn watch_cancellation(
        &self,
        signal: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Option<Box<dyn ScanCancellationWatch>> {
        if self.cancelled() {
            signal();
        }
        None
    }
}

/// Owned cancellation registration; releasing it must not block the UI/runtime.
pub trait ScanCancellationWatch: Send {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanCancellation {
    Requested,
    /// Shutdown or reconfiguration preserves the automatic-update preference.
    Interrupted,
}

pub trait FileIndex: Send + Sync + std::fmt::Debug {
    fn report(&self, _: &NoticeQuery) -> Result<NoticePage, IndexError> {
        Err(IndexError(
            "Full scan reports are unavailable in this session.".into(),
        ))
    }
    fn summaries(&self) -> Option<&dyn summary::SummaryIndex> {
        None
    }
    fn knowledge(&self) -> Option<&dyn graph::FileKnowledge> {
        None
    }
    fn semantic(&self) -> Option<&dyn semantic::SemanticIndex> {
        None
    }
    fn list(&self) -> Result<Vec<IndexedRoot>, IndexError>;
    fn recover(&self) -> Result<(), IndexError>;
    fn add_root(&self, path: &Path) -> Result<IndexedRoot, IndexError>;
    fn scan(&self, root_id: &str, observer: &dyn ScanObserver) -> Result<IndexedRoot, IndexError>;
    fn set_auto_refresh(&self, root_id: &str, enabled: bool) -> Result<(), IndexError>;
    fn set_scan_options(&self, _: &str, _: ScanOptions) -> Result<(), IndexError> {
        Err(IndexError(
            "Folder inclusion settings are unavailable in this session.".into(),
        ))
    }
    /// Installs bounded native watches. Runtime performs periodic reconciliation
    /// even when this succeeds; callbacks may only mark a scope dirty.
    fn watch(
        &self,
        root: &IndexedRoot,
        changes: std::sync::Arc<dyn IndexChanges>,
    ) -> Result<Box<dyn IndexWatch>, IndexError>;
    /// Removes the derived index and its access grant, never source files.
    fn remove(&self, root_id: &str) -> Result<(), IndexError>;
    fn search(&self, query: &IndexQuery) -> Result<Vec<IndexPassage>, IndexError>;
    fn read(&self, reference: &PassageRef) -> Result<IndexPassage, IndexError>;
    fn catalogue(&self, query: &CatalogQuery) -> Result<CatalogPage, IndexError>;
    fn history(&self, query: &FileHistoryQuery) -> Result<FileHistory, IndexError>;
    fn read_history(&self, reference: &HistoryPassageRef) -> Result<HistoricalPassage, IndexError>;
}

/// Dropping this owned guard releases native watches. Both creation and disposal
/// may block and belong on a runtime blocking worker.
pub trait IndexWatch: Send {}

pub trait IndexChanges: Send + Sync {
    fn changed(&self);
    fn failed(&self, message: String);
}

/// The runtime schedules model retrieval on blocking workers; adapters never
/// expose filesystem handles or arbitrary path reads to the agent.
#[async_trait::async_trait]
pub trait IndexedFiles: Send + Sync + std::fmt::Debug {
    async fn propose_knowledge(
        &self,
        _: graph::GraphDraft,
        _: graph::GraphOrigin,
    ) -> Result<graph::KnowledgeProposal, IndexError> {
        Err(IndexError(
            "File knowledge is unavailable in this session.".into(),
        ))
    }
    async fn knowledge(&self, _: graph::GraphQuery) -> Result<graph::GraphPage, IndexError> {
        Err(IndexError(
            "File knowledge is unavailable in this session.".into(),
        ))
    }
    async fn knowledge_evidence(&self, _: u64) -> Result<graph::GraphEvidence, IndexError> {
        Err(IndexError(
            "File knowledge is unavailable in this session.".into(),
        ))
    }
    async fn search_report(&self, query: IndexQuery) -> Result<semantic::SearchReport, IndexError> {
        Ok(semantic::SearchReport {
            passages: self.search(query).await?,
            notices: vec![],
        })
    }
    async fn history(&self, query: FileHistoryQuery) -> Result<FileHistory, IndexError>;
    async fn read_history(
        &self,
        reference: HistoryPassageRef,
    ) -> Result<HistoricalPassage, IndexError>;
    async fn folders(&self) -> Result<Vec<IndexedRoot>, IndexError>;
    async fn catalogue(&self, query: CatalogQuery) -> Result<CatalogPage, IndexError>;
    async fn search(&self, query: IndexQuery) -> Result<Vec<IndexPassage>, IndexError>;
    async fn read(&self, reference: PassageRef) -> Result<IndexPassage, IndexError>;
}

fn validate_search_text(text: &str) -> Result<(), IndexError> {
    if text.trim().is_empty() || text.len() > 2048 || text.split_whitespace().count() > 32 {
        return Err(IndexError(
            "Use 1–32 search terms, up to 2,048 bytes. Split longer queries into focused searches."
                .into(),
        ));
    }
    Ok(())
}

pub fn validate_relative_path(path: &str) -> Result<(), IndexError> {
    if path.is_empty()
        || path.len() > 4096
        || path.contains(['\\', '\0'])
        || !Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(IndexError(
            "Use a relative path inside an indexed folder.".into(),
        ));
    }
    Ok(())
}

/// Deliberately conservative defaults: generated content and hidden credentials
/// are never traversed just because their parent directory was selected.
pub fn excluded_name(name: &str) -> bool {
    ScanOptions::default().exclusion_reason(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synchronous_cancellation_watch_signals_only_a_cancelled_scan() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        struct Observer(Option<ScanCancellation>);
        impl ScanObserver for Observer {
            fn cancellation(&self) -> Option<ScanCancellation> {
                self.0
            }
            fn progress(&self, _: &IndexedRoot) {}
        }
        for cancellation in [
            None,
            Some(ScanCancellation::Requested),
            Some(ScanCancellation::Interrupted),
        ] {
            let called = Arc::new(AtomicBool::new(false));
            let signal = called.clone();
            let observer = Observer(cancellation);
            assert!(
                observer
                    .watch_cancellation(Arc::new(move || signal.store(true, Ordering::Relaxed)))
                    .is_none()
            );
            assert_eq!(called.load(Ordering::Relaxed), cancellation.is_some());
            assert_eq!(observer.cancelled(), cancellation.is_some());
        }
    }

    #[test]
    fn queries_and_paths_are_bounded_and_cannot_escape_a_scope() {
        for path in [
            "",
            "/etc/passwd",
            "../outside",
            "a/../../b",
            "a\\b",
            "a\0b",
            ".",
        ] {
            assert!(validate_relative_path(path).is_err(), "{path:?}");
        }
        assert!(validate_relative_path("notes/project report.md").is_ok());
        let mut query = IndexQuery {
            text: "report".into(),
            roots: vec![],
            path_prefix: None,
            limit: 10,
        };
        assert!(query.validate().is_ok());
        query.limit = MAX_SEARCH_RESULTS + 1;
        assert!(query.validate().is_err());
        query.limit = 10;
        query.text.clear();
        assert!(query.validate().is_err());
        query.text = "word ".repeat(32);
        assert!(query.validate().is_ok());
        query.text.push_str("extra");
        assert!(query.validate().is_err());
        for name in [".env", ".git", "target", "node_modules", "DEV-CREDENTIALS"] {
            assert!(excluded_name(name));
        }
        assert!(!excluded_name("project-notes"));
    }

    #[test]
    fn stale_search_responses_cannot_replace_a_newer_query() {
        let mut state = IndexState::default();
        let old = state.begin_search();
        let current = state.begin_search();
        state.apply(&IndexEvent::Results {
            request_id: old,
            passages: vec![],
        });
        assert!(state.searching);
        state.apply(&IndexEvent::Results {
            request_id: current,
            passages: vec![],
        });
        assert!(!state.searching);
        state.apply(&IndexEvent::Error("Retry".into()));
        assert_eq!(state.notice.as_deref(), Some("Retry"));
    }

    #[test]
    fn late_search_cannot_restore_passages_from_a_removed_scope() {
        let mut state = IndexState::default();
        state.apply(&IndexEvent::Roots(vec![IndexedRoot {
            id: "scope".into(),
            path: "/selected".into(),
            status: IndexStatus::Ready,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: None,
            auto_refresh: false,
            scan_options: ScanOptions::default(),
        }]));
        let request_id = state.begin_search();
        let passage = IndexPassage {
            reference: PassageRef {
                root_id: "scope".into(),
                file_id: "file".into(),
                version_id: "version".into(),
                ordinal: 0,
            },
            path: "notes.txt".into(),
            text: "source".into(),
            start_line: 1,
            end_line: 1,
            mime_type: "text/plain".into(),
            note: None,
            observed_at: 1,
            modified_at: None,
        };
        state.apply(&IndexEvent::Results {
            request_id,
            passages: vec![passage.clone()],
        });
        assert_eq!(state.passages.len(), 1);
        let request_id = state.begin_search();
        state.apply(&IndexEvent::Roots(vec![]));
        state.apply(&IndexEvent::Results {
            request_id,
            passages: vec![passage],
        });
        assert_eq!(state.passages, [] as [IndexPassage; 0]);
        assert!(!state.searching);
    }
}
