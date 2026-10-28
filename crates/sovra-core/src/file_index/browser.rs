//! Correlated inventory and excerpt requests; old replies cannot restore closed or revoked files.

use super::{CatalogPage, CatalogQuery, IndexCommand, IndexError, IndexPassage, PassageRef};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrowserState {
    pub page: Option<CatalogPage>,
    pub query: CatalogQuery,
    pub loading: bool,
    pub error: Option<String>,
    pub preview: Option<IndexPassage>,
    pub preview_reference: Option<PassageRef>,
    pub previewing: bool,
    pub preview_error: Option<String>,
    request_id: u64,
    preview_id: u64,
}

impl BrowserState {
    pub fn browse(&mut self, query: CatalogQuery) -> IndexCommand {
        self.request_id = self.request_id.saturating_add(1);
        self.loading = true;
        self.error = None;
        self.page = None;
        self.query = query.clone();
        self.close_preview();
        IndexCommand::Browse {
            request_id: self.request_id,
            query,
        }
    }

    pub fn preview(&mut self, reference: PassageRef) -> IndexCommand {
        self.close_preview();
        self.previewing = true;
        self.preview_reference = Some(reference.clone());
        IndexCommand::Preview {
            request_id: self.preview_id,
            reference,
        }
    }

    pub fn close_preview(&mut self) {
        self.preview_id = self.preview_id.saturating_add(1);
        self.preview = None;
        self.preview_reference = None;
        self.preview_error = None;
        self.previewing = false;
    }

    pub fn invalidate(&mut self) {
        self.request_id = self.request_id.saturating_add(1);
        self.page = None;
        self.loading = false;
        self.error =
            Some("Folder access or inclusion settings changed. Reload the file list.".into());
        self.close_preview();
    }

    pub(super) fn finish_browse(&mut self, id: u64, result: &Result<CatalogPage, IndexError>) {
        if id != self.request_id || !self.loading {
            return;
        }
        self.loading = false;
        match result {
            Ok(page) => self.page = Some(page.clone()),
            Err(error) => self.error = Some(format!("{error} Reload the list to start again.")),
        }
    }

    pub(super) fn finish_preview(&mut self, id: u64, result: &Result<IndexPassage, IndexError>) {
        if id != self.preview_id || !self.previewing {
            return;
        }
        self.previewing = false;
        match result {
            Ok(passage) if self.preview_reference.as_ref() == Some(&passage.reference) => {
                self.preview = Some(passage.clone());
            }
            Ok(_) => self.preview_error = Some("The reply did not match the requested saved text. Reload the file list and try again.".into()),
            Err(error) => {
                self.preview_error = Some(format!(
                    "{error} The saved file may have changed. Reload the list and try again."
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_navigation_rejects_late_or_mismatched_parts_and_keeps_failed_target_for_retry() {
        let mut state = BrowserState::default();
        let mut passage = IndexPassage {
            reference: PassageRef {
                root_id: "r".into(),
                file_id: "f".into(),
                version_id: "v".into(),
                ordinal: 0,
            },
            path: "notes.txt".into(),
            text: "first".into(),
            start_line: 1,
            end_line: 1,
            mime_type: "text/plain".into(),
            note: None,
            observed_at: 1,
            modified_at: None,
        };
        let IndexCommand::Preview {
            request_id: first, ..
        } = state.preview(passage.reference.clone())
        else {
            panic!()
        };
        state.finish_preview(first, &Ok(passage.clone()));
        let next = PassageRef {
            ordinal: 1,
            ..passage.reference.clone()
        };
        let IndexCommand::Preview {
            request_id: second, ..
        } = state.preview(next.clone())
        else {
            panic!()
        };
        state.finish_preview(first, &Ok(passage.clone()));
        assert!(state.previewing && state.preview.is_none());
        state.finish_preview(second, &Err(IndexError("Reader is busy".into())));
        assert_eq!(state.preview_reference, Some(next.clone()));
        let IndexCommand::Preview {
            request_id: retry, ..
        } = state.preview(next.clone())
        else {
            panic!()
        };
        state.finish_preview(retry, &Ok(passage.clone()));
        assert!(state.preview.is_none());
        assert!(
            state
                .preview_error
                .as_deref()
                .unwrap()
                .contains("did not match")
        );
        let IndexCommand::Preview {
            request_id: retry, ..
        } = state.preview(next.clone())
        else {
            panic!()
        };
        passage.reference = next;
        passage.text = "last".into();
        state.finish_preview(retry, &Ok(passage.clone()));
        assert_eq!(state.preview, Some(passage));
        state.close_preview();
        assert!(state.preview_reference.is_none());
    }

    #[test]
    fn removing_or_reconfiguring_a_root_rejects_its_inflight_listing() {
        use crate::file_index::{
            IndexEvent, IndexProgress, IndexState, IndexStatus, IndexedRoot, ScanOptions,
        };
        let root = IndexedRoot {
            id: "r".into(),
            path: "/chosen".into(),
            status: IndexStatus::Ready,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: Some(1),
            auto_refresh: false,
            scan_options: ScanOptions::default(),
        };
        let page = CatalogPage {
            revision: 1,
            total_files: 0,
            total_known_bytes: 0,
            unknown_sizes: 0,
            coverage: vec![(&root).into()],
            files: vec![],
            groups: vec![],
            next_cursor: None,
            has_more: false,
        };
        for removal in [false, true] {
            let mut state = IndexState::default();
            state.apply(&IndexEvent::Roots(vec![root.clone()]));
            let IndexCommand::Browse { request_id, .. } =
                state.browser.browse(CatalogQuery::default())
            else {
                panic!()
            };
            if removal {
                state.apply(&IndexEvent::Roots(vec![]));
                // Regranting the same folder must not revive an earlier request.
                state.apply(&IndexEvent::Roots(vec![root.clone()]));
            } else {
                let mut changed = root.clone();
                changed.scan_options.include_hidden = true;
                state.apply(&IndexEvent::Progress(changed));
            }
            state.apply(&IndexEvent::Browsed {
                request_id,
                result: Ok(page.clone()),
            });
            assert!(state.browser.page.is_none());
            assert!(!state.browser.loading);
        }
    }

    #[test]
    fn late_results_and_errors_cannot_replace_a_newer_listing() {
        let mut state = BrowserState::default();
        let IndexCommand::Browse {
            request_id: old, ..
        } = state.browse(CatalogQuery::default())
        else {
            panic!()
        };
        let IndexCommand::Browse {
            request_id: new, ..
        } = state.browse(CatalogQuery::default())
        else {
            panic!()
        };
        state.finish_browse(old, &Err(IndexError("old".into())));
        assert!(state.loading);
        assert_eq!(state.error, None);
        state.finish_browse(new, &Err(IndexError("new".into())));
        assert!(!state.loading);
        assert!(state.error.as_deref().unwrap().starts_with("new"));
    }

    #[test]
    fn revocation_invalidates_both_inflight_operations_and_close_ignores_preview_errors() {
        let mut state = BrowserState::default();
        let IndexCommand::Browse { request_id, .. } = state.browse(CatalogQuery::default()) else {
            panic!()
        };
        let reference = PassageRef {
            root_id: "r".into(),
            file_id: "f".into(),
            version_id: "v".into(),
            ordinal: 0,
        };
        let IndexCommand::Preview {
            request_id: preview_id,
            ..
        } = state.preview(reference.clone())
        else {
            panic!()
        };
        state.invalidate();
        state.finish_browse(request_id, &Err(IndexError("late".into())));
        state.finish_preview(preview_id, &Err(IndexError("late".into())));
        assert!(!state.loading && !state.previewing);
        assert!(!state.error.as_deref().unwrap().contains("late"));
        assert_eq!(state.preview_error, None);
        let IndexCommand::Preview { request_id, .. } = state.preview(reference) else {
            panic!()
        };
        state.close_preview();
        state.finish_preview(request_id, &Err(IndexError("closed".into())));
        assert_eq!(state.preview_error, None);
    }
}
