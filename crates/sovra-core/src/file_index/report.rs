//! Paged diagnostics for the latest scan, separate from searchable source content.

use super::{IndexCommand, IndexError, IndexStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticeKind {
    Skipped,
    Unreadable,
    Failed,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeCursor {
    pub scan_id: u64,
    pub after: u64,
    pub through: u64,
    pub filter_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeQuery {
    pub root_id: String,
    pub kind: Option<NoticeKind>,
    pub contains: Option<String>,
    #[serde(default)]
    pub match_case: bool,
    pub cursor: Option<NoticeCursor>,
    pub limit: usize,
}

impl NoticeQuery {
    pub fn validate(&self) -> Result<(), IndexError> {
        if self.root_id.is_empty() || self.root_id.len() > 120 || !(1..=40).contains(&self.limit) {
            return Err(IndexError(
                "Choose an indexed folder and request 1–40 report entries per page.".into(),
            ));
        }
        if self
            .contains
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 256 || s.contains('\0'))
        {
            return Err(IndexError("Use a report filter of 1–256 bytes.".into()));
        }
        if self.cursor.as_ref().is_some_and(|c| {
            c.scan_id == 0
                || c.after > c.through
                || c.through > i64::MAX as u64
                || c.scan_id > i64::MAX as u64
                || c.filter_key.is_empty()
                || c.filter_key.len() > 120
        }) {
            return Err(IndexError(
                "Reload the scan report to reset its pages.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanNotice {
    pub sequence: u64,
    pub kind: NoticeKind,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticePage {
    pub root_id: String,
    /// None means this folder predates full reports or has never been scanned.
    pub scan_id: Option<u64>,
    pub status: IndexStatus,
    pub total: u64,
    pub has_newer: bool,
    /// Totals for the captured scan snapshot, before applying display filters.
    pub counts: Vec<(NoticeKind, u64)>,
    pub notices: Vec<ScanNotice>,
    pub next_cursor: Option<NoticeCursor>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportState {
    pub query: Option<NoticeQuery>,
    pub page: Option<NoticePage>,
    pub loading: bool,
    pub error: Option<String>,
    request_id: u64,
}

impl ReportState {
    pub fn begin(&mut self, query: NoticeQuery) -> IndexCommand {
        self.request_id = self.request_id.saturating_add(1);
        self.query = Some(query.clone());
        self.page = None;
        self.error = None;
        self.loading = true;
        IndexCommand::Report {
            request_id: self.request_id,
            query,
        }
    }

    pub fn invalidate(&mut self) {
        self.request_id = self.request_id.saturating_add(1);
        self.page = None;
        self.loading = false;
        self.error =
            Some("Folder access or inclusion settings changed. Reload the scan report.".into());
    }

    pub(super) fn finish(&mut self, id: u64, result: &Result<NoticePage, IndexError>) {
        if id != self.request_id || !self.loading {
            return;
        }
        self.loading = false;
        match result {
            Ok(page) => self.page = Some(page.clone()),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn query() -> NoticeQuery {
        NoticeQuery {
            root_id: "r".into(),
            kind: None,
            contains: None,
            match_case: false,
            cursor: None,
            limit: 40,
        }
    }
    #[test]
    fn report_requests_validate_filters_and_snapshot_bounds() {
        assert!(query().validate().is_ok());
        for q in [
            NoticeQuery {
                limit: 0,
                ..query()
            },
            NoticeQuery {
                contains: Some("\0".into()),
                ..query()
            },
            NoticeQuery {
                cursor: Some(NoticeCursor {
                    scan_id: 1,
                    after: 3,
                    through: 2,
                    filter_key: "k".into(),
                }),
                ..query()
            },
        ] {
            assert!(q.validate().is_err());
        }
    }
    #[test]
    fn report_changes_and_revocation_ignore_stale_responses() {
        let mut state = ReportState::default();
        let IndexCommand::Report {
            request_id: old, ..
        } = state.begin(query())
        else {
            panic!()
        };
        let IndexCommand::Report {
            request_id: current,
            ..
        } = state.begin(query())
        else {
            panic!()
        };
        state.finish(old, &Err(IndexError("old".into())));
        assert!(state.loading && state.error.is_none());
        state.finish(current, &Err(IndexError("current".into())));
        assert_eq!(state.error.as_deref(), Some("current"));
        let IndexCommand::Report { request_id, .. } = state.begin(query()) else {
            panic!()
        };
        state.invalidate();
        state.finish(request_id, &Err(IndexError("late".into())));
        assert!(!state.loading);
        assert!(!state.error.as_deref().unwrap().contains("late"));
    }
}
