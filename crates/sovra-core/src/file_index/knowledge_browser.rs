//! Request-scoped review of saved file relationships and their original evidence.

use super::{
    IndexCommand, IndexError,
    graph::{GraphEvidence, GraphPage, GraphQuery},
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnowledgeBrowserState {
    pub query: GraphQuery,
    pub page: Option<GraphPage>,
    pub loading: bool,
    pub error: Option<String>,
    pub evidence: Option<GraphEvidence>,
    pub evidence_id: Option<u64>,
    pub reading: bool,
    pub evidence_error: Option<String>,
    request_id: u64,
    evidence_request: u64,
}

impl KnowledgeBrowserState {
    pub fn browse(&mut self, query: GraphQuery) -> IndexCommand {
        self.request_id = self.request_id.saturating_add(1);
        self.query = query.clone();
        self.page = None;
        self.loading = true;
        self.error = None;
        self.close_evidence();
        IndexCommand::BrowseKnowledge {
            request_id: self.request_id,
            query,
        }
    }

    pub fn read_evidence(&mut self, id: u64) -> Option<IndexCommand> {
        if !self
            .page
            .as_ref()
            .is_some_and(|p| p.sources.iter().any(|s| s.id == id))
        {
            return None;
        }
        self.close_evidence();
        self.evidence_id = Some(id);
        self.reading = true;
        Some(IndexCommand::KnowledgeEvidence {
            request_id: self.evidence_request,
            id,
        })
    }

    pub fn close_evidence(&mut self) {
        self.evidence_request = self.evidence_request.saturating_add(1);
        self.evidence = None;
        self.evidence_id = None;
        self.reading = false;
        self.evidence_error = None;
    }

    pub fn invalidate(&mut self) {
        self.request_id = self.request_id.saturating_add(1);
        self.page = None;
        self.loading = false;
        self.error =
            Some("Folder access or included files changed. Reload saved relationships.".into());
        self.close_evidence();
    }

    pub(super) fn finish(&mut self, id: u64, result: &Result<GraphPage, IndexError>) {
        if id != self.request_id || !self.loading {
            return;
        }
        self.loading = false;
        match result {
            Ok(page) => self.page = Some(page.clone()),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(super) fn finish_evidence(
        &mut self,
        id: u64,
        result: &Result<Box<GraphEvidence>, IndexError>,
    ) {
        if id != self.evidence_request || !self.reading {
            return;
        }
        self.reading = false;
        match result {
            Ok(evidence) if Some(evidence.source.id) == self.evidence_id => {
                self.evidence = Some(evidence.as_ref().clone());
            }
            Ok(_) => {
                self.evidence_error =
                    Some("The saved text did not match this reference. Try again.".into());
            }
            Err(error) => self.evidence_error = Some(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::{
        IndexPassage, PassageRef,
        graph::{GraphCursor, GraphSource},
    };

    #[test]
    fn evidence_retry_close_and_revocation_reject_old_or_mismatched_text() {
        let source = GraphSource {
            id: 7,
            reference: PassageRef {
                root_id: "r".into(),
                file_id: "f".into(),
                version_id: "v".into(),
                ordinal: 0,
            },
            path: "brief.txt".into(),
            start_line: 1,
            end_line: 1,
            current: true,
        };
        let evidence = GraphEvidence {
            passage: IndexPassage {
                reference: source.reference.clone(),
                path: source.path.clone(),
                text: "Original passage".into(),
                start_line: 1,
                end_line: 1,
                mime_type: "text/plain".into(),
                note: None,
                observed_at: 1,
                modified_at: None,
            },
            source: source.clone(),
        };
        let mut state = KnowledgeBrowserState {
            page: Some(GraphPage {
                entities: vec![],
                relations: vec![],
                sources: vec![source],
                truncated: false,
                next: None,
            }),
            ..Default::default()
        };
        let IndexCommand::KnowledgeEvidence { request_id, .. } = state.read_evidence(7).unwrap()
        else {
            panic!()
        };
        let mut wrong = evidence.clone();
        wrong.source.id = 8;
        state.finish_evidence(request_id, &Ok(Box::new(wrong)));
        assert!(state.evidence.is_none());
        assert!(state.evidence_error.is_some());
        let IndexCommand::KnowledgeEvidence {
            request_id: retry, ..
        } = state.read_evidence(7).unwrap()
        else {
            panic!()
        };
        state.finish_evidence(request_id, &Ok(Box::new(evidence.clone())));
        assert!(state.reading);
        state.finish_evidence(retry, &Ok(Box::new(evidence.clone())));
        assert_eq!(state.evidence, Some(evidence.clone()));
        let IndexCommand::KnowledgeEvidence {
            request_id: late, ..
        } = state.read_evidence(7).unwrap()
        else {
            panic!()
        };
        state.close_evidence();
        state.finish_evidence(late, &Ok(Box::new(evidence.clone())));
        assert!(state.evidence.is_none());
        let IndexCommand::KnowledgeEvidence {
            request_id: revoked,
            ..
        } = state.read_evidence(7).unwrap()
        else {
            panic!()
        };
        state.invalidate();
        state.finish_evidence(revoked, &Ok(Box::new(evidence)));
        assert!(state.evidence.is_none());
        assert!(state.page.is_none());
    }

    #[test]
    fn cursor_validation_rejects_ambiguous_multihop_paging_and_invalid_boundaries() {
        let mut query = GraphQuery {
            before: Some(GraphCursor {
                recorded_at: 1,
                id: "r".into(),
            }),
            ..Default::default()
        };
        assert!(query.validate().is_ok());
        query.hops = 2;
        assert!(query.validate().is_err());
        query.hops = 1;
        query.before.as_mut().unwrap().recorded_at = u64::MAX;
        assert!(query.validate().is_err());
        query.before.as_mut().unwrap().recorded_at = 1;
        query.before.as_mut().unwrap().id.clear();
        assert!(query.validate().is_err());
    }
    #[test]
    fn late_results_and_access_changes_cannot_restore_a_previous_page() {
        let mut state = KnowledgeBrowserState::default();
        let IndexCommand::BrowseKnowledge { request_id, .. } = state.browse(GraphQuery::default())
        else {
            panic!()
        };
        let page = GraphPage {
            entities: vec![],
            relations: vec![],
            sources: vec![],
            truncated: false,
            next: None,
        };
        state.invalidate();
        state.finish(request_id, &Ok(page.clone()));
        assert!(state.page.is_none());
        assert!(state.error.is_some());
        let IndexCommand::BrowseKnowledge {
            request_id: next, ..
        } = state.browse(GraphQuery::default())
        else {
            panic!()
        };
        state.finish(request_id, &Err(IndexError("late failure".into())));
        assert!(state.loading);
        state.finish(next, &Ok(page.clone()));
        assert_eq!(state.page, Some(page));
        assert!(!state.loading);
        assert!(state.read_evidence(999).is_none());
    }
}
