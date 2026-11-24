//! Discoveries derived from granted, indexed source versions without a model or a review step.
use super::{
    IndexError, PassageRef,
    graph::{
        DomainProperty, DraftEndpoint, EntityDraft, EntityKind, GraphDraft, RelationDraft,
        RelationKind,
    },
    ontology::{FileKind, LifeArea},
};
use crate::knowledge_analysis::{FindingKind, Statement};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFinding {
    pub id: i64,
    pub kind: FindingKind,
    pub detail: String,
    pub path: String,
    pub line: u32,
    pub reference: PassageRef,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryOverview {
    pub revision: u64,
    pub roots: Vec<String>,
    pub working: bool,
    pub kinds: Vec<(FileKind, u64)>,
    pub areas: Vec<(LifeArea, u64)>,
    pub counts: Vec<(FindingKind, u64)>,
    pub subjects: Vec<(EntityKind, u64)>,
    pub predicates: Vec<(RelationKind, u64)>,
    pub examples: Vec<DiscoveredFinding>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindingPage {
    pub items: Vec<DiscoveredFinding>,
    pub next: Option<i64>,
}

/// Only the requested page is retained. A late response cannot replace a newer selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindingBrowser {
    pub kind: Option<FindingKind>,
    pub request_id: u64,
    pub page: Option<Result<FindingPage, IndexError>>,
}
impl FindingBrowser {
    pub fn open(&mut self, kind: FindingKind, before: Option<i64>) -> super::IndexCommand {
        self.request_id = self.request_id.wrapping_add(1);
        self.kind = Some(kind);
        self.page = None;
        super::IndexCommand::BrowseFindings {
            request_id: self.request_id,
            kind,
            before,
        }
    }
    pub fn close(&mut self) {
        self.kind = None;
        self.page = None;
        self.request_id = self.request_id.wrapping_add(1);
    }
}

/// One page of one kind of finding, optionally only those whose wording or
/// file path contains some text. Helpers ask through this; the UI pages by kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingQuery {
    pub kind: FindingKind,
    pub contains: Option<String>,
    pub before: Option<i64>,
}

impl FindingQuery {
    pub const MAX_TEXT: usize = 240;

    /// Blank text means no filter; matching ignores letter case.
    pub fn new(
        kind: FindingKind,
        contains: Option<&str>,
        before: Option<i64>,
    ) -> Result<Self, IndexError> {
        let contains = contains.map(str::trim).filter(|t| !t.is_empty());
        if contains.is_some_and(|t| t.len() > Self::MAX_TEXT) {
            return Err(IndexError(format!(
                "Search findings with at most {} bytes of text.",
                Self::MAX_TEXT
            )));
        }
        if before.is_some_and(|b| b < 1) {
            return Err(IndexError(
                "Use the next cursor returned by the previous page.".into(),
            ));
        }
        Ok(Self {
            kind,
            contains: contains.map(str::to_lowercase),
            before,
        })
    }
}

pub trait AutomaticIndex: Send + Sync {
    /// Advance a bounded portion of saved data. More work can resume after restart.
    fn advance(&self) -> Result<bool, IndexError>;
    fn findings(&self, kind: FindingKind, before: Option<i64>) -> Result<FindingPage, IndexError> {
        self.search(&FindingQuery {
            kind,
            contains: None,
            before,
        })
    }
    fn search(&self, query: &FindingQuery) -> Result<FindingPage, IndexError>;
    fn overview(&self, since: Option<u64>) -> Result<Option<DiscoveryOverview>, IndexError>;
}

fn class(label: Option<&str>, fallback: EntityKind) -> Option<EntityKind> {
    let Some(label) = label else {
        return Some(fallback);
    };
    EntityKind::BUILT_INS
        .iter()
        .find(|k| k.matches_source_class(label))
        .cloned()
        .or_else(|| label.to_lowercase().try_into().ok().map(EntityKind::Domain))
}

/// Convert a source claim to a typed graph edge. Explicit incompatible types remain
/// readable in the source analysis. Never coerce them to fit a built-in predicate.
pub fn source_claim(
    claim: &Statement,
    reference: &PassageRef,
    wording: &str,
) -> Option<GraphDraft> {
    use EntityKind::{Category, Document, Organization, Person, Project, Section, Task, Topic};
    let predicate = claim.predicate.to_lowercase().replace([' ', '-'], "_");
    let (left, right) = match predicate.as_str() {
        "part_of" => (Section, Document),
        "depends_on" => (Task, Task),
        "for_project" => (Task, Project),
        "is_a" => (Topic, Category),
        "about" => (Document, Topic),
        "revises" => (Document, Document),
        "works_for" => (Person, Organization),
        _ => (Topic, Topic),
    };
    let left = class(claim.subject_type.as_deref(), left)?;
    let right = class(claim.object_type.as_deref(), right)?;
    let relation = match RelationKind::BUILT_INS
        .iter()
        .find(|k| k.label().replace(' ', "_") == predicate)
    {
        Some(kind) => kind.clone(),
        None => RelationKind::Property(DomainProperty {
            id: predicate.try_into().ok()?,
            subject: left.clone(),
            object: right.clone(),
        }),
    };
    // Grounding links point to a concrete passage. An arbitrary text object
    // cannot stand in for a source reference.
    if !relation.accepts(Some(&left), Some(&right))
        || claim.subject.eq_ignore_ascii_case(&claim.object)
    {
        return None;
    }
    let entities = [("s", &claim.subject, left), ("o", &claim.object, right)]
        .into_iter()
        .map(|(id, label, kind)| EntityDraft {
            local_id: id.into(),
            kind,
            label: label.clone(),
            evidence: vec![0],
        })
        .collect();
    let draft = GraphDraft {
        title: "Relationship found in indexed text".into(),
        evidence: vec![reference.clone()],
        entities,
        relations: vec![RelationDraft {
            subject: DraftEndpoint::New("s".into()),
            relation,
            object: DraftEndpoint::New("o".into()),
            statement: wording.to_owned(),
            evidence: vec![0],
            valid_time: None,
            replaces: None,
            confidence: None,
        }],
    };
    draft.validate().ok()?;
    Some(draft)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_measurements_and_dates_cover_the_window() {
        let text = "100 kg measured on 2024-02-01.\n".repeat(40);
        let report = crate::knowledge_analysis::analyze_indexed(&text).unwrap();
        assert_eq!(report.quantities.len(), 40);
        assert_eq!(
            report
                .findings
                .iter()
                .filter(|f| f.kind == FindingKind::Chronology)
                .count(),
            40
        );
    }
    #[test]
    fn subclass_claims_chain_and_declared_endpoint_types_are_respected() {
        let reference = PassageRef {
            root_id: "r".into(),
            file_id: "f".into(),
            version_id: "v".into(),
            ordinal: 0,
        };
        let wording = "Basil is a kind of herb.";
        let analysis = crate::knowledge_analysis::analyze_indexed(wording).unwrap();
        let mut claim = analysis.statements[0].clone();
        let edge = source_claim(&claim, &reference, wording).unwrap();
        assert_eq!(edge.entities[0].kind, EntityKind::Category);
        assert_eq!(edge.entities[1].kind, EntityKind::Category);
        claim.object_type = Some("person".into());
        assert!(source_claim(&claim, &reference, wording).is_none());
        claim.object_type = None;
        claim.subject_type = Some("crop".into());
        claim.predicate = "ag:grown_by".into();
        let custom = source_claim(&claim, &reference, wording).unwrap();
        assert!(
            matches!(&custom.relations[0].relation,RelationKind::Property(p) if p.subject==custom.entities[0].kind)
        );
    }
}
