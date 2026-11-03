//! Reviewed, source-grounded knowledge. Permission to save an inference never
//! upgrades it into an observed fact or a user assertion about its truth.

use super::{Confidence, EvidenceBasis, IndexError, IndexPassage, PassageRef, WorldValidity};
use serde::{Deserialize, Serialize};

pub const MAX_GRAPH_EVIDENCE: usize = 8;
pub const MAX_GRAPH_ENTITIES: usize = 16;
pub const MAX_GRAPH_CLAIMS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Document,
    Section,
    Topic,
    Person,
    Organization,
    Project,
    Task,
}

impl EntityKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Document => "Document",
            Self::Section => "Section",
            Self::Topic => "Topic",
            Self::Person => "Person",
            Self::Organization => "Organization",
            Self::Project => "Project",
            Self::Task => "Task",
        }
    }
}

fn calendar_time(value: u64) -> Result<(), IndexError> {
    if value > 253_402_300_799 {
        return Err(IndexError(
            "Knowledge dates must be between 1970 and 9999 UTC.".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    GroundedIn,
    PartOf,
    About,
    ForProject,
    DependsOn,
    RelatedTo,
    Revises,
}

impl RelationKind {
    /// None is a source-version citation, not a logical entity.
    pub fn accepts(self, subject: Option<EntityKind>, object: Option<EntityKind>) -> bool {
        use EntityKind::{Document, Project, Section, Task};
        match self {
            Self::GroundedIn => subject.is_some() && object.is_none(),
            Self::PartOf => matches!(
                (subject, object),
                (Some(Section), Some(Document | Section)) | (Some(Task), Some(Project | Task))
            ),
            Self::About => matches!(subject, Some(Document | Section)) && object.is_some(),
            Self::ForProject => {
                matches!(subject, Some(Document | Section | Task)) && object == Some(Project)
            }
            Self::DependsOn => {
                matches!(subject, Some(Document | Project | Task))
                    && matches!(object, Some(Document | Project | Task))
            }
            Self::RelatedTo => subject.is_some() && object.is_some(),
            Self::Revises => subject == Some(Document) && object == Some(Document),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::GroundedIn => "grounded in",
            Self::PartOf => "part of",
            Self::About => "about",
            Self::ForProject => "for project",
            Self::DependsOn => "depends on",
            Self::RelatedTo => "related to",
            Self::Revises => "revises",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DraftEndpoint {
    New(String),
    Entity(String),
    /// Index into GraphDraft.evidence; identifies a cited source version.
    Source(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityDraft {
    pub local_id: String,
    pub kind: EntityKind,
    pub label: String,
    pub evidence: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationDraft {
    pub subject: DraftEndpoint,
    pub relation: RelationKind,
    pub object: DraftEndpoint,
    pub statement: String,
    pub evidence: Vec<u8>,
    #[serde(default)]
    pub confidence: Option<Confidence>,
    #[serde(default)]
    pub valid_time: Option<WorldValidity>,
    #[serde(default)]
    pub replaces: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDraft {
    pub title: String,
    pub evidence: Vec<PassageRef>,
    pub entities: Vec<EntityDraft>,
    pub relations: Vec<RelationDraft>,
}

fn bounded_text(value: &str, limit: usize) -> Result<(), IndexError> {
    if value.trim().is_empty() || value.len() > limit || value.contains('\0') {
        Err(IndexError(
            "Knowledge text or identifier is empty, oversized or contains a null character.".into(),
        ))
    } else {
        Ok(())
    }
}

fn indices(indices: &[u8], evidence: usize) -> Result<(), IndexError> {
    let unique: std::collections::BTreeSet<_> = indices.iter().collect();
    if indices.is_empty()
        || indices.len() > evidence
        || unique.len() != indices.len()
        || indices.iter().any(|i| usize::from(*i) >= evidence)
    {
        return Err(IndexError(
            "Every entity and relationship needs distinct, valid source-evidence indices.".into(),
        ));
    }
    Ok(())
}

impl GraphDraft {
    pub fn validate(&self) -> Result<(), IndexError> {
        bounded_text(&self.title, 240)?;
        if self.evidence.is_empty()
            || self.evidence.len() > MAX_GRAPH_EVIDENCE
            || self.entities.len() > MAX_GRAPH_ENTITIES
            || self.relations.is_empty()
            || self.relations.len() > MAX_GRAPH_CLAIMS
        {
            return Err(IndexError(
                "Propose 1–24 relationships, up to 16 entities and 1–8 source passages.".into(),
            ));
        }
        let mut sources = std::collections::BTreeSet::new();
        for reference in &self.evidence {
            for id in [
                &reference.root_id,
                &reference.file_id,
                &reference.version_id,
            ] {
                bounded_text(id, 120)?;
            }
            if !sources.insert((
                &reference.root_id,
                &reference.file_id,
                &reference.version_id,
                reference.ordinal,
            )) {
                return Err(IndexError("Source passages must not be repeated.".into()));
            }
        }
        let mut entities = std::collections::BTreeMap::new();
        for entity in &self.entities {
            bounded_text(&entity.local_id, 80)?;
            bounded_text(&entity.label, 240)?;
            if entities
                .insert(entity.local_id.as_str(), entity.kind)
                .is_some()
            {
                return Err(IndexError(
                    "Local entity identifiers must be unique.".into(),
                ));
            }
            indices(&entity.evidence, self.evidence.len())?;
        }
        let endpoint = |value: &DraftEndpoint| -> Result<(), IndexError> {
            match value {
                DraftEndpoint::New(id) if entities.contains_key(id.as_str()) => Ok(()),
                DraftEndpoint::Entity(id) => bounded_text(id, 120),
                DraftEndpoint::Source(index) if usize::from(*index) < self.evidence.len() => Ok(()),
                DraftEndpoint::New(_) | DraftEndpoint::Source(_) => Err(IndexError(
                    "A relationship references an unknown entity or source.".into(),
                )),
            }
        };
        let mut replacements = std::collections::BTreeSet::new();
        for relation in &self.relations {
            endpoint(&relation.subject)?;
            endpoint(&relation.object)?;
            if relation.subject == relation.object {
                return Err(IndexError(
                    "A relationship cannot refer an entity to itself.".into(),
                ));
            }
            bounded_text(&relation.statement, 1000)?;
            indices(&relation.evidence, self.evidence.len())?;
            if let DraftEndpoint::Source(index) = relation.object
                && !relation.evidence.contains(&index)
            {
                return Err(IndexError(
                    "A source endpoint must be included in that relationship's evidence.".into(),
                ));
            }
            if let Some(valid) = &relation.valid_time {
                valid.validate()?;
                for value in [valid.from, valid.until].into_iter().flatten() {
                    calendar_time(value)?;
                }
            }
            if let Some(replaces) = &relation.replaces {
                bounded_text(replaces, 120)?;
                if !replacements.insert(replaces) {
                    return Err(IndexError(
                        "A relationship can be superseded only once in one proposal.".into(),
                    ));
                }
            }
        }
        for id in entities.keys() {
            if !self.relations.iter().any(|r| {
                [&r.subject, &r.object]
                    .iter()
                    .any(|e| matches!(e,DraftEndpoint::New(local) if local == id))
            }) {
                return Err(IndexError(
                    "Every proposed entity must participate in a relationship.".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphOrigin {
    pub session_id: String,
    pub invocation_id: String,
    pub agent: String,
}

impl GraphOrigin {
    pub fn validate(&self) -> Result<(), IndexError> {
        for value in [&self.session_id, &self.invocation_id, &self.agent] {
            bounded_text(value, 240)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Pending,
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeProposal {
    pub id: u64,
    pub state: ProposalState,
    pub created_at: u64,
    #[serde(default)]
    pub reviewed_at: Option<u64>,
    pub origin: GraphOrigin,
    pub draft: GraphDraft,
    pub sources: Vec<IndexPassage>,
    pub existing_entities: Vec<GraphEntity>,
    pub replaced_relations: Vec<GraphRelation>,
    pub required_roots: Vec<String>,
}

impl KnowledgeProposal {
    pub fn saved_endpoint_label(&self, endpoint: &GraphEndpoint) -> String {
        match endpoint {
            GraphEndpoint::Entity(id) => self
                .existing_entities
                .iter()
                .find(|e| e.id == *id)
                .map_or_else(
                    || format!("Entity [{id}]"),
                    |e| format!("{}: {} [{}]", e.kind.label(), e.label, e.id),
                ),
            GraphEndpoint::Source(id) => format!("Saved source evidence [{id}]"),
        }
    }

    /// Stable labels for reviewing the actual structured relationship.
    pub fn endpoint_label(&self, endpoint: &DraftEndpoint) -> String {
        match endpoint {
            DraftEndpoint::New(id) => self
                .draft
                .entities
                .iter()
                .find(|e| e.local_id == *id)
                .map_or_else(
                    || "Unavailable entity".into(),
                    |e| format!("{}: {} (new)", e.kind.label(), e.label),
                ),
            DraftEndpoint::Entity(id) => self
                .existing_entities
                .iter()
                .find(|e| e.id == *id)
                .map_or_else(
                    || "Unavailable entity".into(),
                    |e| format!("{}: {} [{}]", e.kind.label(), e.label, e.id),
                ),
            DraftEndpoint::Source(index) => self.sources.get(usize::from(*index)).map_or_else(
                || "Unavailable source".into(),
                |s| format!("Source {}: {}", usize::from(*index) + 1, s.path),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEntity {
    pub id: String,
    pub kind: EntityKind,
    pub label: String,
    pub evidence: Vec<u64>,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum GraphEndpoint {
    Entity(String),
    Source(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphRelation {
    pub id: String,
    pub subject: GraphEndpoint,
    pub relation: RelationKind,
    pub object: GraphEndpoint,
    pub statement: String,
    pub evidence: Vec<u64>,
    pub basis: EvidenceBasis,
    pub proposal_id: u64,
    #[serde(default)]
    pub origin: Option<GraphOrigin>,
    pub replaces: Option<String>,
    pub expired_at: Option<u64>,
    /// Stale citations invalidate retrieval support, not historical world truth.
    pub sources_current: bool,
}

impl GraphRelation {
    pub fn visible_at(
        &self,
        recorded_at: u64,
        valid_at: Option<u64>,
        include_unknown_validity: bool,
    ) -> bool {
        if self.basis.recorded_at > recorded_at
            || self.expired_at.is_some_and(|at| at <= recorded_at)
        {
            return false;
        }
        match (valid_at, &self.basis.valid_time) {
            (None, _) => true,
            (
                Some(_),
                None
                | Some(WorldValidity {
                    from: None,
                    until: None,
                }),
            ) => include_unknown_validity,
            (Some(at), Some(valid)) => {
                valid.from.is_none_or(|from| from <= at)
                    && valid.until.is_none_or(|until| until >= at)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphCursor {
    pub recorded_at: u64,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphQuery {
    /// Exclusive boundary for one-hop browsing, ordered by recording time then ID.
    pub before: Option<GraphCursor>,
    pub roots: Vec<String>,
    pub text: Option<String>,
    pub entity_id: Option<String>,
    pub hops: u8,
    pub limit: usize,
    pub include_stale: bool,
    pub include_superseded: bool,
    pub recorded_at: Option<u64>,
    pub valid_at: Option<u64>,
    pub include_unknown_validity: bool,
}

impl Default for GraphQuery {
    fn default() -> Self {
        Self {
            before: None,
            roots: vec![],
            text: None,
            entity_id: None,
            hops: 1,
            limit: 20,
            include_stale: false,
            include_superseded: false,
            recorded_at: None,
            valid_at: None,
            include_unknown_validity: false,
        }
    }
}

impl GraphQuery {
    pub fn validate(&self) -> Result<(), IndexError> {
        if let Some(cursor) = &self.before {
            calendar_time(cursor.recorded_at)?;
            bounded_text(&cursor.id, 120)?;
            if self.hops != 1 {
                return Err(IndexError(
                    "Page one level of relationships at a time.".into(),
                ));
            }
        }
        for value in [self.recorded_at, self.valid_at].into_iter().flatten() {
            calendar_time(value)?;
        }
        if !(1..=3).contains(&self.hops) || !(1..=40).contains(&self.limit) {
            return Err(IndexError(
                "Query 1–3 graph hops and 1–40 relationships within granted folders.".into(),
            ));
        }
        for root in &self.roots {
            bounded_text(root, 120)?;
        }
        if let Some(text) = &self.text {
            bounded_text(text, 240)?;
        }
        if let Some(id) = &self.entity_id {
            bounded_text(id, 120)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphSource {
    pub id: u64,
    pub reference: PassageRef,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphPage {
    pub entities: Vec<GraphEntity>,
    pub relations: Vec<GraphRelation>,
    pub sources: Vec<GraphSource>,
    pub truncated: bool,
    /// Continue one-hop browsing, including past batches containing only filtered records.
    #[serde(default)]
    pub next: Option<GraphCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEvidence {
    pub source: GraphSource,
    pub passage: IndexPassage,
}

pub trait FileKnowledge: Send + Sync {
    fn propose(
        &self,
        draft: &GraphDraft,
        origin: &GraphOrigin,
    ) -> Result<KnowledgeProposal, IndexError>;
    fn pending(&self) -> Result<Vec<KnowledgeProposal>, IndexError>;
    fn decide(&self, proposal_id: u64, approve: bool) -> Result<KnowledgeProposal, IndexError>;
    fn query(&self, query: &GraphQuery) -> Result<GraphPage, IndexError>;
    fn evidence(&self, id: u64) -> Result<GraphEvidence, IndexError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> GraphDraft {
        serde_json::from_value(serde_json::json!({
            "title":"Orchid knowledge",
            "evidence":[{"root_id":"root","file_id":"file","version_id":"v1","ordinal":0}],
            "entities":[{"local_id":"doc","kind":"document","label":"Orchid brief","evidence":[0]}],
            "relations":[{"subject":{"kind":"new","id":"doc"},"relation":"grounded_in","object":{"kind":"source","id":0},"statement":"This is the brief.","evidence":[0]}]
        })).unwrap()
    }
    #[test]
    fn proposals_require_bounded_unique_current_references_and_linked_entities() {
        let valid = draft();
        assert!(valid.validate().is_ok());
        let mut invalid = valid.clone();
        invalid.evidence.push(invalid.evidence[0].clone());
        assert!(invalid.validate().is_err());
        invalid = valid.clone();
        invalid.entities[0].evidence = vec![1];
        assert!(invalid.validate().is_err());
        invalid = valid.clone();
        invalid.relations[0].object = DraftEndpoint::New("missing".into());
        assert!(invalid.validate().is_err());
        invalid = valid.clone();
        invalid.relations[0].valid_time = Some(WorldValidity {
            from: Some(u64::MAX),
            until: None,
        });
        assert!(invalid.validate().is_err());
        invalid = valid.clone();
        invalid.entities.push(EntityDraft {
            local_id: "unused".into(),
            ..invalid.entities[0].clone()
        });
        assert!(invalid.validate().is_err());
        let mut json = serde_json::to_value(valid).unwrap();
        json["relations"][0]["valid_time"] = serde_json::json!({"from":1,"approved":true});
        assert!(serde_json::from_value::<GraphDraft>(json).is_err());
    }

    #[test]
    fn review_state_deduplicates_submission_and_ignores_late_pending_snapshots() {
        use super::super::{IndexEvent, IndexProgress, IndexState, IndexStatus, IndexedRoot};
        let mut state = IndexState::default();
        state.apply(&IndexEvent::Roots(vec![IndexedRoot {
            id: "root".into(),
            path: "/chosen".into(),
            status: IndexStatus::Ready,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: Some(1),
            auto_refresh: false,
            scan_options: crate::file_index::ScanOptions::default(),
        }]));
        let proposal = KnowledgeProposal {
            id: 1,
            state: ProposalState::Pending,
            created_at: 1,
            reviewed_at: None,
            origin: GraphOrigin {
                session_id: "s".into(),
                invocation_id: "i".into(),
                agent: "helper".into(),
            },
            draft: draft(),
            sources: vec![],
            existing_entities: vec![],
            replaced_relations: vec![],
            required_roots: vec!["root".into()],
        };
        assert_eq!(
            proposal.endpoint_label(&DraftEndpoint::New("doc".into())),
            "Document: Orchid brief (new)"
        );
        assert_eq!(EntityKind::Project.label(), "Project");
        assert_eq!(
            proposal.saved_endpoint_label(&GraphEndpoint::Source(4)),
            "Saved source evidence [4]"
        );
        assert!(!state.begin_review(1));
        state.apply(&IndexEvent::KnowledgePending(vec![proposal.clone()]));
        assert!(state.begin_review(1));
        assert!(!state.begin_review(1));
        state.apply(&IndexEvent::Error("retry".into()));
        assert!(state.begin_review(1));
        state.apply(&IndexEvent::KnowledgeReviewed {
            proposal_id: 1,
            accepted: true,
        });
        state.apply(&IndexEvent::KnowledgePending(vec![proposal.clone()]));
        assert_eq!(state.knowledge_pending, Vec::<KnowledgeProposal>::new());
        let mut other = proposal;
        other.id = 2;
        state.apply(&IndexEvent::KnowledgePending(vec![other]));
        assert!(state.begin_review(2));
        state.apply(&IndexEvent::Roots(vec![]));
        assert_eq!(state.knowledge_pending, Vec::<KnowledgeProposal>::new());
        assert!(state.knowledge_reviewing.is_empty());
    }

    #[test]
    fn ontology_edges_have_explicit_domains_and_ranges() {
        assert!(RelationKind::GroundedIn.accepts(Some(EntityKind::Document), None));
        assert!(!RelationKind::GroundedIn.accepts(None, Some(EntityKind::Document)));
        assert!(
            RelationKind::PartOf.accepts(Some(EntityKind::Section), Some(EntityKind::Document))
        );
        assert!(
            !RelationKind::PartOf.accepts(Some(EntityKind::Document), Some(EntityKind::Section))
        );
        assert!(
            RelationKind::ForProject.accepts(Some(EntityKind::Task), Some(EntityKind::Project))
        );
        assert!(
            !RelationKind::Revises.accepts(Some(EntityKind::Person), Some(EntityKind::Document))
        );
        assert_eq!(RelationKind::PartOf.label(), "part of");
    }
    #[test]
    fn graph_query_limits_are_enforced() {
        assert!(GraphQuery::default().validate().is_ok());
        assert!(
            GraphQuery {
                hops: 4,
                ..GraphQuery::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            GraphQuery {
                text: Some(String::new()),
                ..GraphQuery::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            GraphQuery {
                limit: 41,
                ..GraphQuery::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            GraphOrigin {
                session_id: "session".into(),
                invocation_id: "call".into(),
                agent: "agent".into()
            }
            .validate()
            .is_ok()
        );
    }
    #[test]
    fn recorded_and_world_valid_time_are_independent() {
        let mut relation = GraphRelation {
            id: "fact".into(),
            subject: GraphEndpoint::Entity("a".into()),
            relation: RelationKind::RelatedTo,
            object: GraphEndpoint::Entity("b".into()),
            statement: "a relates to b".into(),
            evidence: vec![1],
            basis: EvidenceBasis {
                tier: super::super::EvidenceTier::ModelInference,
                confidence: Confidence::new(0.6).ok(),
                activity_id: None,
                recorded_at: 100,
                observed_at: None,
                valid_time: Some(WorldValidity {
                    from: Some(20),
                    until: Some(40),
                }),
            },
            proposal_id: 1,
            origin: None,
            replaces: None,
            expired_at: Some(200),
            sources_current: true,
        };
        assert!(!relation.visible_at(99, Some(30), false));
        assert!(relation.visible_at(150, Some(30), false));
        assert!(!relation.visible_at(200, Some(30), false));
        assert!(!relation.visible_at(150, Some(80), false));
        relation.basis.valid_time = None;
        assert!(!relation.visible_at(150, Some(30), false));
        assert!(relation.visible_at(150, Some(30), true));
        assert!(relation.visible_at(150, None, false));
    }
}
