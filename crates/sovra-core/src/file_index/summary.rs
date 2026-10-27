//! Derived summaries retain their exact inputs. They are navigation aids with
//! model provenance, never replacements for source evidence or reviewed facts.

use super::{EvidenceBasis, IndexError, MAX_INDEX_ROOTS, PassageRef};
use crate::models::ModelDestination;
use serde::{Deserialize, Serialize};

pub const MAX_SUMMARY_INPUTS: usize = 16;
pub const MAX_SUMMARY_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_SUMMARY_TEXT_BYTES: usize = 6000;
pub const MAX_SUMMARY_LEVEL: u8 = 64;
pub const SUMMARY_PROMPT_REVISION: &str = "source-linked-summary-v1";
pub const SUMMARY_ALGORITHM_REVISION: &str = "source-linked-summary-dag-v1";

fn text(value: &str, limit: usize) -> Result<(), IndexError> {
    if value.trim().is_empty() || value.len() > limit || value.contains('\0') {
        return Err(IndexError(
            "Summary text or identifier is empty, oversized or contains a null character.".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryPlan {
    pub name: String,
    pub roots: Vec<String>,
    pub summarizer: ModelDestination,
    pub embedder: ModelDestination,
    /// A build stops before exceeding this explicitly chosen request budget.
    pub max_calls: u32,
    pub automatic: bool,
}
impl SummaryPlan {
    pub fn validate(&self) -> Result<(), IndexError> {
        text(&self.name, 240)?;
        if self.roots.is_empty()
            || self.roots.len() > MAX_INDEX_ROOTS
            || !(1..=10_000).contains(&self.max_calls)
        {
            return Err(IndexError(
                "Choose granted folders and a summary request budget from 1 to 10,000.".into(),
            ));
        }
        let unique: std::collections::BTreeSet<_> = self.roots.iter().collect();
        if unique.len() != self.roots.len() {
            return Err(IndexError("A summary scope cannot repeat a folder.".into()));
        }
        for root in &self.roots {
            text(root, 120)?;
        }
        for model in [&self.summarizer, &self.embedder] {
            text(&model.key.connection_id, 120)?;
            text(&model.key.model_id, 500)?;
            text(&model.base_url, 2048)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryConfiguration {
    pub id: u64,
    pub plan: SummaryPlan,
    pub enabled: bool,
    pub created_at: u64,
    pub calls_used: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryKind {
    PassageCluster,
    Document,
    Directory,
    Project,
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "reference",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SummaryInputRef {
    Passage(PassageRef),
    Summary(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryRequest {
    pub configuration_id: u64,
    pub kind: SummaryKind,
    pub inputs: Vec<SummaryInputRef>,
}
impl SummaryRequest {
    pub fn validate(&self) -> Result<(), IndexError> {
        if self.configuration_id == 0
            || self.inputs.is_empty()
            || self.inputs.len() > MAX_SUMMARY_INPUTS
        {
            return Err(IndexError(
                "A summary needs an approved configuration and 1–16 inputs.".into(),
            ));
        }
        for (index, source) in self.inputs.iter().enumerate() {
            if self.inputs[..index].contains(source) {
                return Err(IndexError("A summary cannot repeat an input.".into()));
            }
            match source {
                SummaryInputRef::Passage(reference) => {
                    for id in [
                        &reference.root_id,
                        &reference.file_id,
                        &reference.version_id,
                    ] {
                        text(id, 120)?;
                    }
                }
                SummaryInputRef::Summary(0) => {
                    return Err(IndexError("Invalid summary reference.".into()));
                }
                SummaryInputRef::Summary(_) => {}
            }
        }
        Ok(())
    }
}

/// Inputs remain data, including when their text contains apparent instructions.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryInput {
    pub reference: SummaryInputRef,
    pub label: String,
    pub text: String,
    pub extraction_note: Option<String>,
}
impl std::fmt::Debug for SummaryInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SummaryInput")
            .field("reference", &self.reference)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedSummary {
    pub configuration: SummaryConfiguration,
    pub request: SummaryRequest,
    pub inputs: Vec<SummaryInput>,
    pub level: u8,
    /// Binds model, prompt/algorithm revision, exact references, labels and text.
    pub content_key: String,
}

impl PreparedSummary {
    /// Validate a transport payload; storage still owns freshness and reservations.
    pub fn validate(&self) -> Result<(), IndexError> {
        self.configuration.plan.validate()?;
        self.request.validate()?;
        text(&self.content_key, 240)?;
        if !self.configuration.enabled
            || self.request.configuration_id != self.configuration.id
            || self.inputs.len() != self.request.inputs.len()
            || !(1..=MAX_SUMMARY_LEVEL).contains(&self.level)
        {
            return Err(IndexError(
                "The prepared summary is disabled or inconsistent.".into(),
            ));
        }
        let mut bytes = 0usize;
        for (input, reference) in self.inputs.iter().zip(&self.request.inputs) {
            if &input.reference != reference {
                return Err(IndexError(
                    "Summary inputs do not match their requested references.".into(),
                ));
            }
            if let SummaryInputRef::Passage(passage) = reference
                && !self.configuration.plan.roots.contains(&passage.root_id)
            {
                return Err(IndexError(
                    "A summary input is outside its approved folders.".into(),
                ));
            }
            text(&input.label, MAX_SUMMARY_INPUT_BYTES)?;
            text(&input.text, MAX_SUMMARY_INPUT_BYTES)?;
            let note = input.extraction_note.as_deref().unwrap_or_default();
            if note.len() > MAX_SUMMARY_INPUT_BYTES || note.contains('\0') {
                return Err(IndexError("The summary extraction note is invalid.".into()));
            }
            bytes = bytes
                .saturating_add(input.label.len())
                .saturating_add(input.text.len())
                .saturating_add(note.len());
        }
        if bytes > MAX_SUMMARY_INPUT_BYTES {
            return Err(IndexError(
                "Summary inputs exceed 64 KiB. Reduce this group before sending it.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryAttempt {
    pub id: u64,
    pub configuration_id: u64,
    pub content_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryReservation {
    Cached(Box<SummaryRecord>),
    Attempt(SummaryAttempt),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryClaim {
    pub statement: String,
    /// Zero-based direct input indices. Support does not establish truth.
    pub supports: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryText {
    pub title: String,
    pub claims: Vec<SummaryClaim>,
    pub caveats: Vec<String>,
}
impl SummaryText {
    pub fn validate(&self, input_count: usize) -> Result<(), IndexError> {
        text(&self.title, 240)?;
        if !(1..=MAX_SUMMARY_INPUTS).contains(&input_count)
            || self.claims.is_empty()
            || self.claims.len() > 24
            || self.caveats.len() > 8
        {
            return Err(IndexError(
                "Summaries need 1–24 supported claims and at most eight caveats.".into(),
            ));
        }
        let mut bytes = self.title.len();
        for claim in &self.claims {
            text(&claim.statement, 1200)?;
            bytes += claim.statement.len();
            let unique: std::collections::BTreeSet<_> = claim.supports.iter().collect();
            if claim.supports.is_empty()
                || claim.supports.len() > input_count
                || unique.len() != claim.supports.len()
                || claim
                    .supports
                    .iter()
                    .any(|i| usize::from(*i) >= input_count)
            {
                return Err(IndexError(
                    "Every summary claim needs distinct citations to its actual inputs.".into(),
                ));
            }
        }
        for caveat in &self.caveats {
            text(caveat, 1000)?;
            bytes += caveat.len();
        }
        if bytes > MAX_SUMMARY_TEXT_BYTES {
            return Err(IndexError("Summary content exceeds 6,000 bytes.".into()));
        }
        Ok(())
    }

    /// Embedding input excludes citation syntax while preserving all summary text.
    pub fn embedding_text(&self) -> String {
        std::iter::once(self.title.as_str())
            .chain(self.claims.iter().map(|c| c.statement.as_str()))
            .chain(self.caveats.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryInvalidation {
    SourceChanged,
    DependencyInvalidated,
    GrantRemoved,
    ConfigurationChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryRecord {
    pub id: u64,
    pub configuration_id: u64,
    pub kind: SummaryKind,
    pub level: u8,
    pub content: SummaryText,
    pub inputs: Vec<SummaryInput>,
    pub model: ModelDestination,
    pub prompt_revision: String,
    pub algorithm_revision: String,
    pub basis: EvidenceBasis,
    pub source_passages: u64,
    pub source_files: u64,
    pub invalidated_at: Option<u64>,
    pub invalidation: Option<SummaryInvalidation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SummaryQuery {
    pub configuration_ids: Vec<u64>,
    pub roots: Vec<String>,
    pub text: Option<String>,
    pub min_level: u8,
    pub max_level: u8,
    pub include_stale: bool,
    pub before_id: Option<u64>,
    pub limit: usize,
}
impl Default for SummaryQuery {
    fn default() -> Self {
        Self {
            configuration_ids: vec![],
            roots: vec![],
            text: None,
            min_level: 1,
            max_level: MAX_SUMMARY_LEVEL,
            include_stale: false,
            before_id: None,
            limit: 12,
        }
    }
}
impl SummaryQuery {
    pub fn validate(&self) -> Result<(), IndexError> {
        if self.min_level == 0
            || self.min_level > self.max_level
            || self.max_level > MAX_SUMMARY_LEVEL
            || !(1..=20).contains(&self.limit)
            || self.roots.len() > MAX_INDEX_ROOTS
            || self.configuration_ids.len() > 16
            || self.configuration_ids.contains(&0)
            || self.before_id == Some(0)
        {
            return Err(IndexError(
                "Restrict summary queries to levels 1–64 and 1–20 results in granted scopes."
                    .into(),
            ));
        }
        for root in &self.roots {
            text(root, 120)?;
        }
        if let Some(query) = &self.text {
            text(query, 240)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryHit {
    pub id: u64,
    pub configuration_id: u64,
    pub kind: SummaryKind,
    pub level: u8,
    pub title: String,
    pub preview: String,
    pub source_passages: u64,
    pub source_files: u64,
    pub stale: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryPage {
    pub summaries: Vec<SummaryHit>,
    pub next_before: Option<u64>,
}

/// Blocking persistence boundary; runtime owns model calls and cancellation.
pub trait SummaryIndex: Send + Sync {
    fn configure(&self, plan: &SummaryPlan) -> Result<SummaryConfiguration, IndexError>;
    fn configurations(&self) -> Result<Vec<SummaryConfiguration>, IndexError>;
    fn set_enabled(&self, id: u64, enabled: bool) -> Result<(), IndexError>;
    fn prepare(&self, request: &SummaryRequest) -> Result<PreparedSummary, IndexError>;
    /// Reserve before sending a request; interrupted/failed calls still consume budget.
    fn reserve(&self, prepared: &PreparedSummary) -> Result<SummaryReservation, IndexError>;
    fn fail_summary(&self, attempt: &SummaryAttempt, message: &str) -> Result<(), IndexError>;
    fn commit(
        &self,
        attempt: &SummaryAttempt,
        prepared: &PreparedSummary,
        output: &SummaryText,
    ) -> Result<SummaryRecord, IndexError>;
    fn read_summary(&self, id: u64, include_stale: bool) -> Result<SummaryRecord, IndexError>;
    /// Retire a derived result and every dependent ancestor, preserving history.
    fn invalidate_summary(&self, id: u64) -> Result<u64, IndexError>;
    fn query_summaries(&self, query: &SummaryQuery) -> Result<SummaryPage, IndexError>;
}

/// The implementation must execute without tools and treat inputs as untrusted data.
#[async_trait::async_trait]
pub trait TextSummarizer: Send + Sync {
    async fn summarize(&self, prepared: &PreparedSummary) -> Result<SummaryText, IndexError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_summaries_bind_scope_references_and_the_entire_payload_budget() {
        let connection = crate::models::ModelConnection::new(
            "local",
            crate::models::ProviderKind::OpenAiCompatible,
        );
        let model = crate::models::ModelInfo {
            id: "summary".into(),
            ..crate::models::ModelInfo::default()
        };
        let target = ModelDestination::for_embeddings(&connection, &model).unwrap();
        let reference = SummaryInputRef::Passage(PassageRef {
            root_id: "root".into(),
            file_id: "file".into(),
            version_id: "version".into(),
            ordinal: 0,
        });
        let valid = PreparedSummary {
            configuration: SummaryConfiguration {
                id: 1,
                plan: SummaryPlan {
                    name: "Map".into(),
                    roots: vec!["root".into()],
                    summarizer: target.clone(),
                    embedder: target,
                    max_calls: 10,
                    automatic: false,
                },
                enabled: true,
                created_at: 1,
                calls_used: 0,
            },
            request: SummaryRequest {
                configuration_id: 1,
                kind: SummaryKind::Document,
                inputs: vec![reference.clone()],
            },
            inputs: vec![SummaryInput {
                reference,
                label: "A file".into(),
                text: "A proposal.".into(),
                extraction_note: None,
            }],
            level: 1,
            content_key: "bound-input".into(),
        };
        assert!(valid.validate().is_ok());
        let mut bad = valid.clone();
        bad.configuration.enabled = false;
        assert!(bad.validate().is_err());
        bad = valid.clone();
        bad.configuration.plan.roots = vec!["different".into()];
        assert!(bad.validate().is_err());
        bad = valid.clone();
        bad.inputs[0].reference = SummaryInputRef::Summary(1);
        assert!(bad.validate().is_err());
        bad = valid.clone();
        bad.inputs[0].extraction_note = Some("x".repeat(MAX_SUMMARY_INPUT_BYTES));
        assert!(bad.validate().is_err());
        bad = valid;
        bad.inputs.clear();
        assert!(bad.validate().is_err());
    }
    fn output() -> SummaryText {
        SummaryText {
            title: "Garden plan".into(),
            claims: vec![SummaryClaim {
                statement: "The proposed garden uses orchids.".into(),
                supports: vec![0],
            }],
            caveats: vec!["This is a proposal, not a completed project.".into()],
        }
    }
    #[test]
    fn summaries_require_real_support_and_never_silently_trim_text() {
        let valid = output();
        assert!(valid.validate(1).is_ok());
        assert!(valid.embedding_text().contains(&valid.caveats[0]));
        let mut invalid = valid.clone();
        invalid.claims[0].supports = vec![1];
        assert!(invalid.validate(1).is_err());
        invalid = valid.clone();
        invalid.claims[0].supports = vec![0, 0];
        assert!(invalid.validate(2).is_err());
        invalid = valid.clone();
        invalid.claims[0].supports.clear();
        assert!(invalid.validate(1).is_err());
        invalid = valid;
        invalid.caveats = vec!["x".repeat(1000); 8];
        assert!(invalid.validate(1).is_err());
    }
    #[test]
    fn summary_requests_and_queries_are_bounded() {
        let mut request = SummaryRequest {
            configuration_id: 1,
            kind: SummaryKind::Workspace,
            inputs: vec![SummaryInputRef::Summary(1)],
        };
        assert!(request.validate().is_ok());
        request.inputs.push(SummaryInputRef::Summary(1));
        assert!(request.validate().is_err());
        assert!(SummaryQuery::default().validate().is_ok());
        assert!(
            SummaryQuery {
                min_level: 2,
                max_level: 1,
                ..SummaryQuery::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            SummaryQuery {
                limit: 21,
                ..SummaryQuery::default()
            }
            .validate()
            .is_err()
        );
        let connection = crate::models::ModelConnection::new(
            "local",
            crate::models::ProviderKind::OpenAiCompatible,
        );
        let model = crate::models::ModelInfo {
            id: "chosen".into(),
            ..crate::models::ModelInfo::default()
        };
        let target = ModelDestination::for_embeddings(&connection, &model).unwrap();
        let mut plan = SummaryPlan {
            name: "Project map".into(),
            roots: vec!["root".into()],
            summarizer: target.clone(),
            embedder: target,
            max_calls: 64,
            automatic: false,
        };
        assert!(plan.validate().is_ok());
        plan.roots.push("root".into());
        assert!(plan.validate().is_err());
    }
}
