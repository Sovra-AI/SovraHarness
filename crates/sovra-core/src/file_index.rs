//! User-granted folder scopes and bounded retrieval over indexed source versions.
//! Paths are display metadata; model reads use opaque version/chunk references.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
pub mod automatic;
pub mod clustering;
mod options;
pub use options::{DocumentBudget, ScanOptions};

mod dated;
pub mod graph;
mod knowledge_browser;
pub use dated::{DatedAnswer, DatedReading, SavedAnswerPage};
pub use knowledge_browser::KnowledgeBrowserState;
mod history;
pub mod live;
pub mod ontology;
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
    /// Regular files the walk has found so far; final once `walk_complete`.
    #[serde(default)]
    pub discovered: u64,
    /// Discovered files whose outcome (read, reused, skipped, failed) is settled.
    #[serde(default)]
    pub handled: u64,
    /// The walk finished, so `discovered` is the true total and progress is determinate.
    #[serde(default)]
    pub walk_complete: bool,
    /// Transient activity from the running scanner. Old snapshots need no migration.
    #[serde(skip)]
    pub activity: Option<live::ScanActivity>,
    /// Relative path of the most recently processed file.
    #[serde(default)]
    pub current: Option<String>,
    #[serde(default)]
    pub elapsed_ms: u64,
    /// Smoothed settle rate; 0 until enough samples make an estimate honest.
    #[serde(default)]
    pub files_per_minute: u64,
    /// What has been found so far, by [`ontology::FileKind`].
    #[serde(default)]
    pub kinds: std::collections::BTreeMap<ontology::FileKind, u64>,
    /// What it seems to be about, by [`ontology::LifeArea`] (from folder/file names).
    #[serde(default)]
    pub areas: std::collections::BTreeMap<ontology::LifeArea, u64>,
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
        if self.limit == 0 || self.limit > MAX_SEARCH_RESULTS {
            return Err(IndexError(
                "Choose between 1 and 40 results per search.".into(),
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
    SaveAnswer(PassageRef),
    ListAnswers {
        before: Option<u64>,
    },
    BrowseFindings {
        request_id: u64,
        kind: crate::knowledge_analysis::FindingKind,
        before: Option<i64>,
    },
    BrowseKnowledge {
        request_id: u64,
        query: graph::GraphQuery,
    },
    KnowledgeEvidence {
        request_id: u64,
        id: u64,
    },
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
    /// One picker selection is one queued operation, regardless of folder count.
    AddRoots {
        paths: Vec<PathBuf>,
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
    Findings {
        request_id: u64,
        result: Result<automatic::FindingPage, IndexError>,
    },
    Discoveries(Result<automatic::DiscoveryOverview, IndexError>),
    AnswerSaved(DatedAnswer),
    SavedAnswers {
        before: Option<u64>,
        page: SavedAnswerPage,
    },
    KnowledgeBrowsed {
        request_id: u64,
        result: Result<graph::GraphPage, IndexError>,
    },
    KnowledgeEvidence {
        request_id: u64,
        result: Result<Box<graph::GraphEvidence>, IndexError>,
    },
    FoldersAdded(FolderAddition),
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderAddition {
    pub added: Vec<PathBuf>,
    pub failed: Vec<FolderAddFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderAddFailure {
    pub path: PathBuf,
    pub reason: String,
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
    pub finding_browser: automatic::FindingBrowser,
    pub discoveries: Option<Result<automatic::DiscoveryOverview, IndexError>>,
    pub knowledge_browser: KnowledgeBrowserState,
    pub folder_addition: Option<FolderAddition>,
    pub report: ReportState,
    pub browser: BrowserState,
    pub roots: Vec<IndexedRoot>,
    pub passages: Vec<IndexPassage>,
    /// Wording a person saved from a passage. A later search does not replace these.
    pub saved_answers: Vec<DatedAnswer>,
    pub next_saved_before: Option<u64>,
    pub request_id: u64,
    pub searching: bool,
    pub search_completed: bool,
    pub notice: Option<String>,
    pub monitors: std::collections::BTreeMap<String, MonitorStatus>,
    pub semantic: Vec<semantic::SemanticStatus>,
    pub knowledge_pending: Vec<graph::KnowledgeProposal>,
    pub knowledge_review_notice: Option<String>,
    pub knowledge_reviewing: std::collections::BTreeSet<u64>,
    revoked_roots: std::collections::BTreeSet<String>,
    knowledge_reviewed: std::collections::BTreeSet<u64>,
}

impl IndexState {
    /// Summarizes observed scan state without treating a finished scan as complete coverage.
    pub fn capture_summary(&self) -> String {
        if self.roots.is_empty() {
            return "No folders connected.".into();
        }
        let mut reading = 0;
        let mut waiting = 0;
        let mut paused = 0;
        let mut checked = 0;
        let mut attention = 0;
        for root in &self.roots {
            match root.status {
                IndexStatus::Scanning => reading += 1,
                IndexStatus::Pending if root.auto_refresh => waiting += 1,
                IndexStatus::Pending => paused += 1,
                IndexStatus::Ready => checked += 1,
                IndexStatus::Cancelled | IndexStatus::Interrupted | IndexStatus::Failed => {}
            }
            if matches!(
                root.status,
                IndexStatus::Cancelled | IndexStatus::Interrupted | IndexStatus::Failed
            ) || !root.issues.is_empty()
                || root.progress.skipped > 0
                || root.progress.failed > 0
            {
                attention += 1;
            }
        }
        let count = self.roots.len();
        let noun = if count == 1 { "folder" } else { "folders" };
        let mut summary = format!("{count} {noun} connected.");
        for (count, description) in [
            (reading, "reading now"),
            (waiting, "waiting to be read"),
            (paused, "paused"),
            (checked, "checked"),
            (attention, "with items to review"),
        ] {
            if count > 0 {
                summary.push(' ');
                summary.push_str(&count.to_string());
                summary.push(' ');
                summary.push_str(description);
                summary.push('.');
            }
        }
        summary
    }

    /// A review starts only from a visible proposal and cannot be submitted twice.
    pub fn begin_review(&mut self, id: u64) -> bool {
        let started = self.knowledge_pending.iter().any(|p| p.id == id)
            && self.knowledge_reviewing.insert(id);
        if started {
            self.knowledge_review_notice = None;
        }
        started
    }

    pub fn begin_search(&mut self) -> u64 {
        self.request_id = self.request_id.saturating_add(1);
        self.searching = true;
        self.search_completed = false;
        self.notice = None;
        self.request_id
    }

    fn remember_saved_answer(&mut self, answer: &DatedAnswer, newest: bool) {
        if self
            .roots
            .iter()
            .any(|root| root.id == answer.reference().root_id)
            && !self
                .saved_answers
                .iter()
                .any(|a| a.reference() == answer.reference())
        {
            if newest {
                self.saved_answers.insert(0, answer.clone());
            } else {
                self.saved_answers.push(answer.clone());
            }
        }
    }

    pub fn apply(&mut self, event: &IndexEvent) {
        match event {
            IndexEvent::Findings { request_id, result } => {
                if self.finding_browser.request_id == *request_id
                    && self.finding_browser.kind.is_some()
                {
                    self.finding_browser.page = Some(result.clone());
                }
            }
            IndexEvent::Discoveries(result) => {
                if !result
                    .as_ref()
                    .is_ok_and(|r| r.roots.iter().any(|id| self.revoked_roots.contains(id)))
                {
                    self.discoveries = Some(result.clone());
                }
            }
            IndexEvent::KnowledgeBrowsed { request_id, result } => {
                self.knowledge_browser.finish(*request_id, result);
            }
            IndexEvent::KnowledgeEvidence { request_id, result } => {
                self.knowledge_browser.finish_evidence(*request_id, result);
            }
            IndexEvent::AnswerSaved(answer) => self.remember_saved_answer(answer, true),
            IndexEvent::SavedAnswers { before, page } => {
                if before.is_none() || self.next_saved_before == *before {
                    for answer in &page.answers {
                        self.remember_saved_answer(answer, false);
                    }
                    self.next_saved_before = page.next_before;
                }
            }
            IndexEvent::FoldersAdded(result) => {
                self.folder_addition = Some(result.clone());
            }
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
                self.knowledge_review_notice = Some(if *accepted {
                    "Relationship proposal saved. Its evidence is available in saved relationships."
                } else {
                    "Relationship proposal rejected."
                }.into());
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
                self.revoked_roots.extend(
                    self.roots
                        .iter()
                        .filter(|old| !roots.iter().any(|r| r.id == old.id))
                        .map(|r| r.id.clone()),
                );
                self.revoked_roots
                    .retain(|id| !roots.iter().any(|r| &r.id == id));
                if self
                    .discoveries
                    .as_ref()
                    .and_then(|d| d.as_ref().ok())
                    .is_some_and(|d| d.roots.iter().any(|id| self.revoked_roots.contains(id)))
                {
                    self.discoveries = None;
                }
                if self.roots.iter().any(|old| {
                    !roots
                        .iter()
                        .any(|new| new.id == old.id && new.scan_options == old.scan_options)
                }) {
                    self.finding_browser.close();
                    self.browser.invalidate();
                    self.knowledge_browser.invalidate();
                    self.report.invalidate();
                    self.passages.clear();
                    self.search_completed = false;
                }
                self.roots.clone_from(roots);
                self.monitors
                    .retain(|id, _| roots.iter().any(|root| root.id == *id));
                self.semantic
                    .retain(|s| roots.iter().any(|r| r.id == s.setting.root_id));
                self.passages
                    .retain(|p| roots.iter().any(|r| r.id == p.reference.root_id));
                self.saved_answers.retain(|answer| {
                    roots
                        .iter()
                        .any(|root| root.id == answer.reference().root_id)
                });
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
                        self.knowledge_browser.invalidate();
                        self.report.invalidate();
                        self.passages.retain(|p| p.reference.root_id != root.id);
                        self.search_completed = false;
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
                    self.search_completed = true;
                }
            }
            IndexEvent::Error(message) => {
                if !self.knowledge_reviewing.is_empty() {
                    self.knowledge_review_notice = Some(message.clone());
                }
                self.knowledge_reviewing.clear();
                self.notice = Some(message.clone());
                self.searching = false;
                self.search_completed = false;
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
    /// Whole extracted table when it fits one analysis, in saved chunk order.
    fn table_wording(&self, _: &PassageRef) -> Result<Option<String>, IndexError> {
        Ok(None)
    }
    fn binary_mask(
        &self,
        _reference: &PassageRef,
    ) -> Result<Option<crate::knowledge_analysis::mask::Mask>, IndexError> {
        Ok(None)
    }
    fn save_answer(&self, _: &PassageRef) -> Result<DatedAnswer, IndexError> {
        Err(IndexError("This index cannot save quoted answers.".into()))
    }
    fn saved_answers(&self, _: Option<u64>) -> Result<SavedAnswerPage, IndexError> {
        Ok(SavedAnswerPage::default())
    }
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
    fn automatic(&self) -> Option<&dyn automatic::AutomaticIndex> {
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
    fn search_language(
        &self,
        query: &IndexQuery,
        _language: crate::knowledge_analysis::Language,
    ) -> Result<Vec<IndexPassage>, IndexError> {
        self.search(query)
    }
    fn read(&self, reference: &PassageRef) -> Result<IndexPassage, IndexError>;
    /// Read a keyset page without recounting a large folder on every expansion.
    fn map_folder(
        &self,
        _root: &str,
        _after: &str,
        _history: bool,
    ) -> Result<crate::knowledge_map::MapPage, IndexError> {
        Err(IndexError("Folder map browsing is unavailable.".into()))
    }
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
    async fn discoveries(&self) -> Result<automatic::DiscoveryOverview, IndexError> {
        Err(IndexError(
            "Automatic file findings are unavailable in this session.".into(),
        ))
    }
    async fn findings(
        &self,
        _: automatic::FindingQuery,
    ) -> Result<automatic::FindingPage, IndexError> {
        Err(IndexError(
            "Automatic file findings are unavailable in this session.".into(),
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

/// Catalogue filters stay a short AND of literal keywords.
pub(crate) fn validate_keyword_text(text: &str) -> Result<(), IndexError> {
    if text.trim().is_empty() || text.len() > 2048 || text.split_whitespace().count() > 32 {
        return Err(IndexError(
            "Use 1–32 search terms, up to 2,048 bytes. Split longer queries into focused searches."
                .into(),
        ));
    }
    Ok(())
}

/// Content search accepts an ordinary question. Callers must not have to
/// shorten it to a list of at most 32 keywords.
fn validate_search_text(text: &str) -> Result<(), IndexError> {
    if text.trim().is_empty() || text.len() > 2048 {
        return Err(IndexError(
            "Use a question or a few words, up to 2,048 bytes.".into(),
        ));
    }
    Ok(())
}

/// Quoted FTS expression for one content question.
///
/// A short keyword list stays an AND of literal terms, so words such as `OR`
/// cannot change the query. A natural-language question drops function words
/// and matches any remaining term, ranked later by BM25. Terms are always
/// quoted, so the question cannot inject FTS operators.
pub fn lexical_match(text: &str) -> String {
    let raw: Vec<&str> = text.split_whitespace().collect();
    if raw.is_empty() {
        return String::new();
    }
    let meaningful = content_terms(text);
    let question = text.contains('?')
        || raw.len() > 32
        || (raw.len() >= 4 && raw.len() - meaningful.len() >= 2);
    let (terms, joiner) = if question && !meaningful.is_empty() {
        (meaningful, " OR ")
    } else {
        (raw.iter().map(|term| (*term).to_owned()).collect(), " AND ")
    };
    terms
        .into_iter()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(joiner)
}

/// Distinctive tokens in first-seen order, capped so a long question stays bounded.
pub fn content_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
        if token.is_empty() || is_stopword(token) {
            continue;
        }
        if terms
            .iter()
            .any(|seen: &String| seen.eq_ignore_ascii_case(token))
        {
            continue;
        }
        terms.push(token.to_owned());
    }
    if terms.len() <= 48 {
        return terms;
    }
    let mut identifiers: Vec<String> = terms
        .iter()
        .filter(|term| term.chars().any(|c| c.is_ascii_digit()))
        .cloned()
        .collect();
    identifiers.truncate(24);
    for term in terms {
        if identifiers.len() >= 48 {
            break;
        }
        if !term.chars().any(|c| c.is_ascii_digit()) {
            identifiers.push(term);
        }
    }
    identifiers
}

fn is_stopword(token: &str) -> bool {
    const STOP: &[&str] = &[
        "a", "about", "after", "also", "am", "an", "and", "any", "are", "as", "at", "be", "been",
        "before", "being", "but", "by", "can", "could", "did", "do", "does", "doing", "explain",
        "find", "for", "from", "here", "how", "i", "if", "in", "into", "is", "it", "its", "just",
        "many", "me", "much", "my", "no", "not", "of", "on", "only", "or", "our", "over", "please",
        "show", "so", "some", "such", "tell", "than", "that", "the", "then", "there", "these",
        "this", "those", "to", "very", "was", "we", "were", "what", "when", "where", "which",
        "who", "whom", "why", "will", "with", "would", "yes", "you", "your",
    ];
    let lower = token.to_ascii_lowercase();
    STOP.binary_search(&lower.as_str()).is_ok()
}

/// Stored chunk text: the relative path and the heading or nearby line, then the
/// extract. Lexical and vector indexes both read this same string.
pub fn contextual_passage(
    path: &str,
    source: &str,
    start_line: u32,
    body: &str,
    carried_section: Option<&str>,
) -> String {
    PassageContext::new(path, source, carried_section).passage(start_line, body)
}

/// Builds source prefixes in line order without re-reading earlier source text
/// for every chunk. A streamed block can inherit the preceding block's section.
#[derive(Debug)]
pub struct PassageContext<'a> {
    source: &'a str,
    lines: std::str::Lines<'a>,
    next_line: u32,
    heading: Option<&'a str>,
    nearby: Option<&'a str>,
    inherited: Option<&'a str>,
    path: &'a str,
}

impl<'a> PassageContext<'a> {
    pub fn new(path: &'a str, source: &'a str, inherited: Option<&'a str>) -> Self {
        Self {
            source,
            lines: source.lines(),
            next_line: 1,
            heading: None,
            nearby: None,
            inherited,
            path,
        }
    }

    fn advance(&mut self, line: u32) {
        if line < self.next_line {
            self.lines = self.source.lines();
            self.next_line = 1;
            self.heading = None;
            self.nearby = None;
        }
        while self.next_line < line {
            let Some(text) = self.lines.next() else { break };
            self.next_line += 1;
            let trimmed = text.trim();
            if let Some(title) = markdown_heading(trimmed) {
                self.heading = Some(title);
            } else if self.heading.is_none()
                && !trimmed.is_empty()
                && trimmed.chars().take(121).count() <= 120
            {
                self.nearby = Some(trimmed);
            }
        }
    }

    pub fn passage(&mut self, start_line: u32, body: &str) -> String {
        self.advance(start_line);
        format_passage(
            self.path,
            self.heading.or(self.nearby).or(self.inherited),
            body,
        )
    }

    pub fn last_section(mut self) -> Option<&'a str> {
        self.advance(u32::MAX);
        self.heading.or(self.nearby)
    }
}

fn format_passage(path: &str, section: Option<&str>, body: &str) -> String {
    let mut prefix = format!("Source: {}\n", clamp_chars(path, 180));
    if let Some(section) = section {
        let section = clamp_chars(section.trim(), 160);
        if !section.is_empty() {
            prefix.push_str("Section: ");
            prefix.push_str(&section);
            prefix.push('\n');
        }
    }
    prefix.push('\n');
    let mut text = String::with_capacity(prefix.len() + body.len());
    text.push_str(&prefix);
    text.push_str(body);
    let max = semantic::MAX_EMBEDDING_TEXT_BYTES;
    if text.len() > max {
        let room = max.saturating_sub(prefix.len());
        let end = body.floor_char_boundary(room.min(body.len()));
        text = prefix;
        text.push_str(&body[..end]);
    }
    text
}

/// Remove the context the index added, preserving all bytes of the source body.
pub fn passage_wording(text: &str) -> &str {
    if text.starts_with("Source: ")
        && let Some((_, body)) = text.split_once("\n\n")
    {
        return body;
    }
    text
}

/// Last heading (or nearby line) in a block, carried into the next streamed block.
pub fn carried_section(source: &str) -> Option<String> {
    PassageContext::new("", source, None)
        .last_section()
        .map(str::to_owned)
}

fn markdown_heading(line: &str) -> Option<&str> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ') {
        let title = line[hashes + 1..].trim();
        (!title.is_empty()).then_some(title)
    } else {
        None
    }
}

fn clamp_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let end = text
        .char_indices()
        .nth(max)
        .map_or(text.len(), |(index, _)| index);
    text[..end].to_owned()
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
    fn capture_summary_counts_observed_states_and_keeps_notices_visible() {
        let mut state = IndexState::default();
        assert_eq!(state.capture_summary(), "No folders connected.");
        let root = IndexedRoot {
            id: "root".into(),
            path: "/chosen".into(),
            status: IndexStatus::Ready,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: Some(1),
            auto_refresh: true,
            scan_options: ScanOptions::default(),
        };
        state.roots.push(root.clone());
        assert_eq!(state.capture_summary(), "1 folder connected. 1 checked.");
        state.roots[0].progress.skipped = 1;
        assert_eq!(
            state.capture_summary(),
            "1 folder connected. 1 checked. 1 with items to review."
        );
        for status in [
            IndexStatus::Scanning,
            IndexStatus::Pending,
            IndexStatus::Cancelled,
            IndexStatus::Interrupted,
            IndexStatus::Failed,
        ] {
            state.roots.push(IndexedRoot {
                status,
                ..root.clone()
            });
        }
        state.roots.push(IndexedRoot {
            status: IndexStatus::Pending,
            auto_refresh: false,
            ..root
        });
        assert_eq!(
            state.capture_summary(),
            "7 folders connected. 1 reading now. 1 waiting to be read. 1 paused. 1 checked. 4 with items to review."
        );
    }

    #[test]
    fn folder_addition_failures_survive_scan_and_search_updates() {
        let mut state = IndexState::default();
        let result = FolderAddition {
            added: vec!["/chosen".into()],
            failed: vec![FolderAddFailure {
                path: "/unavailable".into(),
                reason: "Permission denied".into(),
            }],
        };
        state.apply(&IndexEvent::FoldersAdded(result.clone()));
        state.apply(&IndexEvent::Roots(vec![]));
        state.begin_search();
        state.apply(&IndexEvent::Error("Unrelated search error".into()));
        assert_eq!(state.folder_addition, Some(result));
        state.apply(&IndexEvent::FoldersAdded(FolderAddition::default()));
        assert_eq!(state.folder_addition, Some(FolderAddition::default()));
    }

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
        query.text = "word ".repeat(40);
        assert!(
            query.validate().is_ok(),
            "a question longer than 32 words is accepted"
        );
        query.text = "x".repeat(2049);
        assert!(query.validate().is_err());
        let question = "Where is the allotment levy written?";
        assert!(lexical_match(question).contains(" OR "));
        assert!(content_terms(question).contains(&"allotment".to_owned()));
        assert!(!content_terms(question).iter().any(|term| term == "where"));
        assert_eq!(
            lexical_match("sunflower OR absent"),
            "\"sunflower\" AND \"OR\" AND \"absent\""
        );
        let source = "# Keeper note\n\nThe allotment levy is forty-two dinars.\n";
        let stored = contextual_passage(
            "aa-misc.txt",
            source,
            3,
            "The allotment levy is forty-two dinars.\n",
            None,
        );
        assert!(stored.contains("Source: aa-misc.txt"));
        assert!(stored.contains("Section: Keeper note"));
        assert!(stored.contains("forty-two dinars"));
        assert!(stored.len() <= semantic::MAX_EMBEDDING_TEXT_BYTES);
        for name in [".env", ".git", "target", "node_modules", "DEV-CREDENTIALS"] {
            assert!(excluded_name(name));
        }
        assert!(!excluded_name("project-notes"));
    }

    #[test]
    fn incremental_passage_context_keeps_citation_text_across_chunks_and_blocks() {
        let source = "nearby line

# First heading
other line
## 二番目
Body
";
        let mut context = PassageContext::new("folder/note.txt", source, Some("Previous block"));
        for (line, expected) in [
            (1, "Previous block"),
            (2, "nearby line"),
            (2, "nearby line"),
            (4, "First heading"),
            (5, "First heading"),
            (6, "二番目"),
            (3, "nearby line"),
        ] {
            assert_eq!(
                context.passage(line, "exact words"),
                format!(
                    "Source: folder/note.txt
Section: {expected}

exact words"
                )
            );
        }
        assert_eq!(context.last_section(), Some("二番目"));
        assert_eq!(
            PassageContext::new("x", "", Some("fallback")).last_section(),
            None
        );
    }

    #[test]
    fn stale_search_responses_cannot_replace_a_newer_query() {
        let mut state = IndexState::default();
        assert!(!state.search_completed);
        let old = state.begin_search();
        let current = state.begin_search();
        state.apply(&IndexEvent::Results {
            request_id: old,
            passages: vec![],
        });
        assert!(state.searching);
        assert!(!state.search_completed);
        state.apply(&IndexEvent::Results {
            request_id: current,
            passages: vec![],
        });
        assert!(!state.searching);
        assert!(state.search_completed);
        state.apply(&IndexEvent::Error("Retry".into()));
        assert_eq!(state.notice.as_deref(), Some("Retry"));
        assert!(!state.search_completed);
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

    #[test]
    fn a_saved_answer_keeps_its_wording_when_the_folder_search_changes() {
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
                version_id: "version-read".into(),
                ordinal: 0,
            },
            path: "keeper-note.txt".into(),
            text: "The allotment levy is forty-two dinars.".into(),
            start_line: 3,
            end_line: 4,
            mime_type: "text/plain".into(),
            note: None,
            observed_at: 1_700_000_000,
            modified_at: Some(1_600_000_000),
        };
        state.apply(&IndexEvent::Results {
            request_id,
            passages: vec![passage],
        });
        let saved = DatedAnswer::save(&state.passages[0]).unwrap();
        state.apply(&IndexEvent::AnswerSaved(saved.clone()));
        state.apply(&IndexEvent::AnswerSaved(saved.clone()));
        assert_eq!(state.saved_answers.len(), 1);
        let request_id = state.begin_search();
        state.apply(&IndexEvent::Results {
            request_id,
            passages: vec![IndexPassage {
                reference: PassageRef {
                    version_id: "version-later".into(),
                    ..saved.reference().clone()
                },
                path: "keeper-note.txt".into(),
                text: "The allotment levy is ninety-nine dinars.".into(),
                start_line: 3,
                end_line: 4,
                mime_type: "text/plain".into(),
                note: None,
                observed_at: 1_800_000_000,
                modified_at: Some(1_800_000_000),
            }],
        });
        let reading = state.saved_answers[0].reread();
        assert_eq!(reading.wording, "The allotment levy is forty-two dinars.");
        assert_eq!(reading.read_on, 1_700_000_000);
        assert_eq!(reading.file, "keeper-note.txt");
        assert_eq!((reading.start_line, reading.end_line), (3, 4));
        assert_ne!(state.passages[0].text, reading.wording);
        state.apply(&IndexEvent::Roots(vec![]));
        assert_eq!(state.saved_answers.as_slice(), []);
    }
}
