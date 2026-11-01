//! File inventory controls and literal saved-text preview. Runtime owns every read.

use super::*;
use sovra_core::file_index::{
    CatalogCursor, CatalogEntry, CatalogFilter, CatalogQuery, ExtractionState,
};

mod preview;

pub(super) struct BrowserView {
    open: bool,
    root: Option<String>,
    name: Entity<InputState>,
    extraction: Option<ExtractionState>,
    match_case: bool,
    cursors: Vec<Option<CatalogCursor>>,
    selected: Option<CatalogEntry>,
    preview_scroll: gpui_kit::ScrollHandle,
    _input: gpui_kit::Subscription,
}

impl BrowserView {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Filter by filename…"));
        let subscription = cx.subscribe_in(&name, window, |shell, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                shell.reload_index_inventory(cx);
            }
        });
        Self {
            open: false,
            root: None,
            name,
            extraction: None,
            match_case: false,
            cursors: vec![None],
            selected: None,
            preview_scroll: gpui_kit::ScrollHandle::new(),
            _input: subscription,
        }
    }
}

pub(super) fn is_open(shell: &Shell) -> bool {
    shell.file_index.browser.open
}

pub(super) fn close(shell: &mut Shell) {
    shell.file_index.browser.open = false;
    shell.file_index.browser.selected = None;
    shell.file_index.state.browser.invalidate();
    shell
        .knowledge_scroll
        .set_offset(gpui_kit::Point::default());
}

impl Shell {
    pub(crate) fn browse_index_folder(
        &mut self,
        root: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        report::close(self);
        self.select_knowledge_section(crate::knowledge_view::KnowledgeSection::Files, cx);
        self.file_index.browser.open = true;
        self.file_index.browser.root = root;
        self.file_index.browser.extraction = None;
        self.file_index.browser.match_case = false;
        self.file_index
            .browser
            .name
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.reload_index_inventory(cx);
    }

    fn reload_index_inventory(&mut self, cx: &mut Context<Self>) {
        let view = &mut self.file_index.browser;
        view.cursors = vec![None];
        view.selected = None;
        if view
            .root
            .as_ref()
            .is_some_and(|id| !self.file_index.state.roots.iter().any(|r| r.id == *id))
        {
            view.root = None;
        }
        let name = view.name.read(cx).value().trim().to_owned();
        let query = CatalogQuery {
            filter: CatalogFilter {
                roots: view.root.clone().into_iter().collect(),
                name_contains: (!name.is_empty()).then_some(name),
                match_case: view.match_case,
                extraction: view.extraction,
                ..CatalogFilter::default()
            },
            ..CatalogQuery::default()
        };
        let command = self.file_index.state.browser.browse(query);
        self.index_command(command, cx);
    }

    fn inventory_page(&mut self, next: bool, cx: &mut Context<Self>) {
        let state = &mut self.file_index.state.browser;
        if state.loading {
            return;
        }
        let view = &mut self.file_index.browser;
        if next {
            let Some(cursor) = state
                .page
                .as_ref()
                .and_then(|page| page.next_cursor.clone())
            else {
                return;
            };
            view.cursors.push(Some(cursor));
        } else if view.cursors.len() > 1 {
            view.cursors.pop();
        } else {
            return;
        }
        view.selected = None;
        let mut query = state.query.clone();
        query.cursor = view.cursors.last().cloned().flatten();
        let command = state.browse(query);
        self.index_command(command, cx);
    }

    fn inspect_index_file(&mut self, entry: CatalogEntry, cx: &mut Context<Self>) {
        self.file_index.state.browser.close_preview();
        self.file_index
            .browser
            .preview_scroll
            .set_offset(gpui_kit::Point::default());
        if let Some(reference) = entry.first_passage.clone() {
            let command = self.file_index.state.browser.preview(reference);
            self.index_command(command, cx);
        }
        self.file_index.browser.selected = Some(entry);
        cx.notify();
    }

    fn preview_index_part(&mut self, ordinal: u32, cx: &mut Context<Self>) {
        if self.file_index.state.browser.previewing {
            return;
        }
        let Some(reference) = self
            .file_index
            .browser
            .selected
            .as_ref()
            .and_then(|entry| entry.passage_at(ordinal))
        else {
            return;
        };
        self.file_index
            .browser
            .preview_scroll
            .set_offset(gpui_kit::Point::default());
        let command = self.file_index.state.browser.preview(reference);
        self.index_command(command, cx);
    }
}

pub(super) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let view = &shell.file_index.browser;
    let state = &shell.file_index.state.browser;
    let mut panel = v_flex()
        .id("index-inventory")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .child(
            design::button("browse-all-indexed-files", "Browse all files", cx).on_click(
                cx.listener(|shell, _, window, cx| shell.browse_index_folder(None, window, cx)),
            ),
        );
    if !view.open {
        return panel;
    }
    panel = panel.child(
        design::button("close-index-inventory", "Back to folders", cx).on_click(cx.listener(
            |shell, _, _, cx| {
                close(shell);
                cx.notify();
            },
        )),
    );
    let scope = view
        .root
        .as_ref()
        .and_then(|id| shell.file_index.state.roots.iter().find(|r| r.id == *id))
        .map_or_else(
            || "All indexed folders".into(),
            |r| r.path.display().to_string(),
        );
    panel = panel
        .child(div().w_full().min_w_0().text_ellipsis().sovra_text(TextRole::SectionTitle, cx).child(scope))
        .child(div().sovra_text(TextRole::Caption, cx).child("Observed files, including files whose contents could not be read. Excluded folders and links appear in the folder’s Scan report. Filename filters ignore capitalization unless Match case is selected."))
        .child(Input::new(&view.name).aria_label("Filter indexed filenames"))
        .child(Checkbox::new("inventory-match-case").cursor_pointer().label("Match case").checked(view.match_case)
            .on_click(cx.listener(|shell, checked, _, cx| {
                shell.file_index.browser.match_case = *checked;
                shell.reload_index_inventory(cx);
            })))
        .child(design::button("reload-index-inventory", "Apply filters / Reload list", cx).on_click(cx.listener(|shell, _, _, cx| shell.reload_index_inventory(cx))));
    let mut filters = h_flex().flex_wrap().gap(t.space.small);
    for (id, label, extraction) in [
        ("inventory-all", "All files", None),
        (
            "inventory-text",
            "Text available",
            Some(ExtractionState::Text),
        ),
        (
            "inventory-partial",
            "Partial text",
            Some(ExtractionState::PartialText),
        ),
        (
            "inventory-metadata",
            "Details only",
            Some(ExtractionState::MetadataOnly),
        ),
        (
            "inventory-failed",
            "Could not read",
            Some(ExtractionState::Failed),
        ),
    ] {
        filters = filters.child(
            design::button(id, label, cx)
                .selected(view.extraction == extraction)
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.file_index.browser.extraction = extraction;
                    shell.reload_index_inventory(cx);
                })),
        );
    }
    panel = panel.child(filters);
    if state.loading {
        panel = panel.child(div().sovra_text(TextRole::Body, cx).child("Loading files…"));
    }
    if let Some(error) = &state.error {
        panel = panel.child(div().sovra_text(TextRole::Body, cx).child(error.clone()));
    }
    let mut selected_details = None;
    // Revocation/inclusion changes clear the domain page; don't retain stale UI metadata.
    if state.page.is_some()
        && let Some(entry) = &view.selected
    {
        let mut details = v_flex()
            .id("index-file-details")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .p(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_ellipsis()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child(entry.path.clone()),
            )
            .child(div().sovra_text(TextRole::Caption, cx).child(format!(
                    "{} · {} · {}",
                    extraction_label(entry.extraction),
                    entry.mime_type.as_deref().unwrap_or("Unknown file type"),
                    entry
                        .byte_size
                        .map_or_else(|| "Size unknown".into(), |n| format!("{n} bytes"))
                )))
            .child(
                design::button("close-index-preview", "Close file details", cx).on_click(
                    cx.listener(|shell, _, _, cx| {
                        shell.file_index.browser.selected = None;
                        shell.file_index.state.browser.close_preview();
                        cx.notify();
                    }),
                ),
            );
        if let Some(note) = &entry.note {
            details = details.child(div().sovra_text(TextRole::Body, cx).child(note.clone()));
        }
        details = details.child(preview::render(entry, state, &view.preview_scroll, cx));
        selected_details = Some(details);
    }
    if let Some(page) = &state.page {
        panel = panel.child(
            div()
                .id("index-inventory-count")
                .test_support()
                .sovra_text(TextRole::Caption, cx)
                .child(format!(
                    "{} matching files · page {} · showing {} · saved inventory",
                    page.total_files,
                    view.cursors.len(),
                    page.files.len()
                )),
        );
        if page
            .coverage
            .iter()
            .any(|scope| scope.status != IndexStatus::Ready)
        {
            panel = panel.child(div().sovra_text(TextRole::Body, cx).child("Some folders have not finished scanning. This inventory may be incomplete; refresh those folders and reload the list."));
        }
        if page.files.is_empty() {
            panel = panel.child(div().sovra_text(TextRole::Body, cx).child("No observed files match. Try All files, clear the filename filter, or check the folder’s scan notices."));
        }
        for (index, entry) in page.files.iter().enumerate() {
            let selected = entry.clone();
            let mut row = v_flex()
                .id(("index-inventory-row", index))
                .test_support()
                .w_full()
                .min_w_0()
                .gap(t.space.compact)
                .p(t.space.small)
                .sovra_surface(Surface::Subtle, cx)
                .child(
                    div()
                        .id(("index-inventory-path", index))
                        .w_full()
                        .min_w_0()
                        .text_ellipsis()
                        .aria_label(entry.path.clone())
                        .sovra_text(TextRole::Body, cx)
                        .child(entry.path.clone()),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_ellipsis()
                        .sovra_text(TextRole::Caption, cx)
                        .child(format!(
                            "{} · {}",
                            extraction_label(entry.extraction),
                            entry.root_path
                        )),
                )
                .child(
                    design::button(("inspect-index-file", index), "View details", cx).on_click(
                        cx.listener(move |shell, _, _, cx| {
                            shell.inspect_index_file(selected.clone(), cx);
                        }),
                    ),
                );
            if let Some(note) = &entry.note {
                row = row.child(div().sovra_text(TextRole::Caption, cx).child(note.clone()));
            }
            panel = panel.child(row);
            if view.selected.as_ref().is_some_and(|selected| {
                selected.root_id == entry.root_id && selected.file_id == entry.file_id
            }) && let Some(details) = selected_details.take()
            {
                panel = panel.child(details);
            }
        }
        panel = panel.child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("inventory-previous", "Previous page", cx)
                        .disabled(view.cursors.len() <= 1 || state.loading)
                        .on_click(cx.listener(|shell, _, _, cx| shell.inventory_page(false, cx))),
                )
                .child(
                    design::button("inventory-next", "Next page", cx)
                        .disabled(page.next_cursor.is_none() || state.loading)
                        .on_click(cx.listener(|shell, _, _, cx| shell.inventory_page(true, cx))),
                ),
        );
    }
    panel
}

fn extraction_label(state: ExtractionState) -> &'static str {
    match state {
        ExtractionState::Text => "Text available",
        ExtractionState::PartialText => "Partial text — check the original",
        ExtractionState::MetadataOnly => "Details only",
        ExtractionState::Failed => "Could not read",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{Root, Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{App, ElementId, TestAppContext, point, px, size};
    use sovra_core::{
        Event,
        file_index::{
            CatalogPage, IndexEvent, IndexPassage, IndexProgress, PassageRef, ScanOptions,
        },
        models::InterfaceScale,
    };
    use sovra_runtime::Client;

    fn root() -> IndexedRoot {
        IndexedRoot {
            id: "r".into(),
            path: "/chosen".into(),
            status: IndexStatus::Ready,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: Some(1),
            auto_refresh: false,
            scan_options: ScanOptions::default(),
        }
    }

    fn page() -> CatalogPage {
        CatalogPage {
            revision: 1,
            total_files: 41,
            total_known_bytes: 200,
            unknown_sizes: 0,
            coverage: vec![(&root()).into()],
            groups: vec![],
            has_more: true,
            next_cursor: Some(CatalogCursor {
                revision: 1,
                filter_key: "k".into(),
                root_id: "r".into(),
                path: "notes.txt".into(),
                file_id: "f".into(),
            }),
            files: vec![CatalogEntry {
                root_id: "r".into(),
                root_path: "/chosen".into(),
                file_id: "f".into(),
                path: format!("{}/notes.txt", "longdirectory".repeat(40)),
                byte_size: Some(200),
                mime_type: Some("text/plain".into()),
                content_digest: None,
                observed_at: Some(1),
                modified_at: None,
                created_at: None,
                extraction: ExtractionState::PartialText,
                note: Some("Some content was not read. The second slide could not be decoded; check the original presentation.".into()),
                passage_count: 3,
                first_passage: Some(PassageRef {
                    root_id: "r".into(),
                    file_id: "f".into(),
                    version_id: "v".into(),
                    ordinal: 0,
                }),
            }],
        }
    }

    fn passage() -> IndexPassage {
        IndexPassage {
            reference: page().files[0].first_passage.clone().unwrap(),
            path: "notes.txt".into(),
            text: format!(
                "<script>never execute</script>\n{}",
                "long source line ".repeat(600)
            ),
            start_line: 1,
            end_line: 2,
            mime_type: "text/plain".into(),
            note: None,
            observed_at: 1,
            modified_at: None,
        }
    }

    fn click(window: &mut Window, cx: &mut App, id: impl Into<ElementId>) {
        let id = id.into();
        for _ in 0..100 {
            window.render_frame(cx);
            let target = window.find(id.clone());
            let bounds = target.bounds();
            let viewport = window.find("sovra-knowledge-view").bounds();
            if target.visible()
                && bounds.top() >= viewport.top()
                && bounds.bottom() < viewport.bottom()
            {
                window.click(id, cx);
                return;
            }
            let delta = if bounds.top() < viewport.top() {
                100.
            } else {
                -100.
            };
            window.scroll(
                "sovra-knowledge-view",
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(delta))),
                cx,
            );
        }
        panic!("control did not become visible: {id:?}");
    }

    #[gpui_kit::test]
    fn inventory_buttons_page_filter_and_preview_through_client(cx: &mut TestAppContext) {
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1280.), px(1000.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        events
            .send_blocking(Event::FileIndex(IndexEvent::Roots(vec![root()])))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.select_tab(NavTab::Knowledge, cx);
                shell.select_knowledge_section(crate::knowledge_view::KnowledgeSection::Files, cx);
            });
            click(window, cx, "browse-all-indexed-files");
        })
        .unwrap();
        let (request_id, query) = std::iter::from_fn(|| commands.try_recv().ok())
            .find_map(|command| {
                if let Command::FileIndex(IndexCommand::Browse { request_id, query }) = command {
                    Some((request_id, query))
                } else {
                    None
                }
            })
            .unwrap();
        assert!(query.filter.roots.is_empty() && query.cursor.is_none());
        assert!(!query.filter.match_case);
        events
            .send_blocking(Event::FileIndex(IndexEvent::Browsed {
                request_id,
                result: Ok(page()),
            }))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            click(window, cx, ("inspect-index-file", 0usize));
        })
        .unwrap();
        let (preview_id, reference) = std::iter::from_fn(|| commands.try_recv().ok())
            .find_map(|command| {
                if let Command::FileIndex(IndexCommand::Preview {
                    request_id,
                    reference,
                }) = command
                {
                    Some((request_id, reference))
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(reference, passage().reference);
        events
            .send_blocking(Event::FileIndex(IndexEvent::Previewed {
                request_id: preview_id,
                result: Ok(passage()),
            }))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("index-details-r").is_none());
            assert!(
                window.try_find("index-preview-next").is_some(),
                "Saved text must be navigable beyond its first part"
            );
            window.click("knowledge-memories", cx);
            window.render_frame(cx);
            assert!(window.try_find("index-inventory").is_none());
            window.click("knowledge-files", cx);
            window.render_frame(cx);
            assert_eq!(
                shell
                    .read(cx)
                    .file_index
                    .state
                    .browser
                    .preview
                    .as_ref()
                    .unwrap()
                    .text,
                passage().text
            );
        })
        .unwrap();
        for (control, ordinal) in [
            ("index-preview-next", 1),
            ("index-preview-last", 2),
            ("index-preview-previous", 1),
            ("index-preview-first", 0),
        ] {
            cx.update_window(window.into(), |_, window, cx| {
                click(window, cx, control);
                window.render_frame(cx);
                assert_eq!(window.find("index-preview-next").disabled(), Some(true));
            })
            .unwrap();
            let (mut request_id, reference) = std::iter::from_fn(|| commands.try_recv().ok())
                .find_map(|command| match command {
                    Command::FileIndex(IndexCommand::Preview {
                        request_id,
                        reference,
                    }) => Some((request_id, reference)),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                reference,
                PassageRef {
                    ordinal,
                    ..passage().reference
                }
            );
            if ordinal == 2 {
                events
                    .send_blocking(Event::FileIndex(IndexEvent::Previewed {
                        request_id,
                        result: Err(sovra_core::file_index::IndexError("Reader is busy".into())),
                    }))
                    .unwrap();
                cx.run_until_parked();
                cx.update_window(window.into(), |_, window, cx| {
                    click(window, cx, "index-preview-retry");
                })
                .unwrap();
                let retry = std::iter::from_fn(|| commands.try_recv().ok())
                    .find_map(|command| match command {
                        Command::FileIndex(IndexCommand::Preview {
                            request_id,
                            reference,
                        }) => Some((request_id, reference)),
                        _ => None,
                    })
                    .unwrap();
                assert!(retry.0 > request_id);
                assert_eq!(retry.1, reference);
                request_id = retry.0;
            }
            let current = IndexPassage {
                reference,
                text: format!("Saved source part {ordinal}"),
                ..passage()
            };
            events
                .send_blocking(Event::FileIndex(IndexEvent::Previewed {
                    request_id,
                    result: Ok(current.clone()),
                }))
                .unwrap();
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(
                    shell.read(cx).file_index.state.browser.preview.as_ref(),
                    Some(&current)
                );
                let position = format!("Saved text · part {} of 3", ordinal + 1);
                assert_eq!(
                    window.find("index-preview-position").label(),
                    Some(position.as_str())
                );
                assert_eq!(
                    window.find("index-preview-next").disabled() == Some(true),
                    ordinal == 2
                );
                assert_eq!(
                    window.find("index-preview-previous").disabled() == Some(true),
                    ordinal == 0
                );
            })
            .unwrap();
        }
        cx.update_window(window.into(), |_, window, cx| {
            click(window, cx, "close-index-preview");
            click(window, cx, "inventory-next");
        })
        .unwrap();
        let (request_id, query) = std::iter::from_fn(|| commands.try_recv().ok())
            .find_map(|command| {
                if let Command::FileIndex(IndexCommand::Browse { request_id, query }) = command {
                    Some((request_id, query))
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(query.cursor, page().next_cursor);
        events
            .send_blocking(Event::FileIndex(IndexEvent::Browsed {
                request_id,
                result: Ok(page()),
            }))
            .unwrap();
        cx.run_until_parked();
        for (id, extraction) in [
            ("inventory-partial", ExtractionState::PartialText),
            ("inventory-failed", ExtractionState::Failed),
        ] {
            cx.update_window(window.into(), |_, window, cx| {
                click(window, cx, id);
            })
            .unwrap();
            let query = std::iter::from_fn(|| commands.try_recv().ok())
                .find_map(|command| {
                    if let Command::FileIndex(IndexCommand::Browse { query, .. }) = command {
                        Some(query)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!(query.filter.extraction, Some(extraction));
            assert_eq!(query.cursor, None);
        }
        cx.update_window(window.into(), |_, window, cx| {
            click(window, cx, "inventory-match-case");
        })
        .unwrap();
        let query = std::iter::from_fn(|| commands.try_recv().ok())
            .find_map(|command| match command {
                Command::FileIndex(IndexCommand::Browse { query, .. }) => Some(query),
                _ => None,
            })
            .unwrap();
        assert!(query.filter.match_case && query.cursor.is_none());
        assert_eq!(query.filter.extraction, Some(ExtractionState::Failed));
        cx.update_window(window.into(), |_, _, cx| {
            assert!(shell.read(cx).file_index.state.browser.preview.is_none());
            assert_eq!(shell.read(cx).file_index.browser.cursors.len(), 1);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn inventory_and_literal_preview_fit_minimum_wide_and_double_text_in_both_themes(
        cx: &mut TestAppContext,
    ) {
        let (client, _commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.unwrap();
            cx.update_window(window.into(), |_, window, cx| {
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                shell.update(cx, |shell, cx| {
                    shell.file_index.apply(&IndexEvent::Roots(vec![root()]));
                    shell.select_tab(NavTab::Knowledge, cx);
                    shell.select_knowledge_section(
                        crate::knowledge_view::KnowledgeSection::Files,
                        cx,
                    );
                    shell.browse_index_folder(None, window, cx);
                    shell.file_index.state.browser.page = Some(page());
                    shell.file_index.state.browser.loading = false;
                    shell.file_index.browser.selected = Some(page().files[0].clone());
                    shell.file_index.state.browser.preview_reference = Some(passage().reference);
                    shell.file_index.state.browser.preview = Some(passage());
                });
                for mode in [ThemeMode::Dark, ThemeMode::Light] {
                    Theme::change(mode, Some(window), cx);
                    window.render_frame(cx);
                    window.render_frame(cx);
                    for id in [
                        "index-inventory",
                        "reload-index-inventory",
                        "inventory-match-case",
                        "inventory-metadata",
                        "inventory-failed",
                        "inventory-partial",
                        "index-file-details",
                        "index-preview-text",
                        "index-preview-position",
                        "index-preview-first",
                        "index-preview-previous",
                        "index-preview-next",
                        "index-preview-last",
                        "inventory-next",
                    ] {
                        let bounds = window.find(id).bounds();
                        assert!(
                            bounds.left() >= px(0.) && bounds.right() <= px(width),
                            "{id} at {scale}%: {bounds:?}"
                        );
                        assert!(bounds.size.height > px(0.), "{id}: {bounds:?}");
                    }
                }
            })
            .unwrap();
        }
    }
}
