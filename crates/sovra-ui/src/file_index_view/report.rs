//! Full latest-scan diagnostics, with bounded display pages and native controls.

use super::*;
use sovra_core::file_index::{NoticeCursor, NoticeKind, NoticeQuery};

pub(super) struct ReportView {
    root: Option<String>,
    filter: Entity<InputState>,
    kind: Option<NoticeKind>,
    match_case: bool,
    cursors: Vec<Option<NoticeCursor>>,
    _filter: gpui_kit::Subscription,
}

impl ReportView {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter paths or reasons…"));
        let subscription =
            cx.subscribe_in(&filter, window, |shell, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    shell.reload_scan_report(cx);
                }
            });
        Self {
            root: None,
            filter,
            kind: None,
            match_case: false,
            cursors: vec![None],
            _filter: subscription,
        }
    }
}

pub(super) fn active_root(shell: &Shell) -> Option<&str> {
    shell.file_index.report.root.as_deref().filter(|id| {
        shell
            .file_index
            .state
            .roots
            .iter()
            .any(|root| root.id == *id)
    })
}

pub(super) fn close(shell: &mut Shell) {
    shell.file_index.report.root = None;
    shell.file_index.state.report.invalidate();
    shell
        .knowledge_scroll
        .set_offset(gpui_kit::Point::default());
}

impl Shell {
    pub(super) fn open_scan_report(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        browser::close(self);
        self.select_knowledge_section(crate::knowledge_view::KnowledgeSection::Files, cx);
        self.file_index.detailed_roots.remove(id);
        self.file_index.report.root = Some(id.into());
        self.file_index.report.kind = None;
        self.file_index.report.match_case = false;
        self.file_index
            .report
            .filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.reload_scan_report(cx);
    }

    fn reload_scan_report(&mut self, cx: &mut Context<Self>) {
        let view = &mut self.file_index.report;
        let Some(root_id) = view.root.clone() else {
            return;
        };
        view.cursors = vec![None];
        let text = view.filter.read(cx).value().trim().to_owned();
        let query = NoticeQuery {
            root_id,
            kind: view.kind,
            contains: (!text.is_empty()).then_some(text),
            match_case: view.match_case,
            cursor: None,
            limit: 40,
        };
        let command = self.file_index.state.report.begin(query);
        self.index_command(command, cx);
    }

    fn scan_report_page(&mut self, next: bool, cx: &mut Context<Self>) {
        let state = &mut self.file_index.state.report;
        if state.loading {
            return;
        }
        let Some(mut query) = state.query.clone() else {
            return;
        };
        let cursors = &mut self.file_index.report.cursors;
        if next {
            let Some(cursor) = state.page.as_ref().and_then(|p| p.next_cursor.clone()) else {
                return;
            };
            cursors.push(Some(cursor));
        } else if cursors.len() > 1 {
            cursors.pop();
        } else {
            return;
        }
        query.cursor = cursors.last().cloned().flatten();
        let command = state.begin(query);
        self.index_command(command, cx);
    }
}

pub(super) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let view = &shell.file_index.report;
    let state = &shell.file_index.state.report;
    let root_id = view.root.clone();
    let mut panel = v_flex().id("index-scan-report").test_support().w_full().min_w_0().gap(t.space.small).p(t.space.small).sovra_surface(Surface::Subtle, cx)
        .child(div().sovra_text(TextRole::SectionTitle, cx).child("Full scan report"))
        .children(active_root(shell).and_then(|id| shell.file_index.state.roots.iter().find(|root| root.id == id)).map(|root| div().min_w_0().w_full().text_ellipsis().sovra_text(TextRole::Body,cx).child(root.path.display().to_string())))
        .child(div().sovra_text(TextRole::Caption, cx).child("Every recorded skip or reading problem from the latest scan. Excluded folders appear once; their contents were not visited. Press Enter or Reload report to apply filters and include newer entries. Filters ignore capitalization unless Match case is selected."))
        .child(Input::new(&view.filter).aria_label("Filter scan report paths or reasons"))
        .child(Checkbox::new("report-match-case").cursor_pointer().label("Match case").checked(view.match_case)
            .on_click(cx.listener(|shell, checked, _, cx| {
                shell.file_index.report.match_case = *checked;
                shell.reload_scan_report(cx);
            })))
        .child(h_flex().flex_wrap().gap(t.space.small)
            .child(design::button("reload-scan-report", "Reload report", cx).on_click(cx.listener(|shell,_,_,cx| shell.reload_scan_report(cx))))
            .child(design::button("refresh-report-folder", "Refresh folder", cx).on_click(cx.listener(move |shell,_,_,cx| { if let Some(root_id) = &root_id { shell.index_command(IndexCommand::Refresh { root_id: root_id.clone() }, cx); } })))
            .child(design::button("close-scan-report", "Back to folders", cx).on_click(cx.listener(|shell,_,_,cx| { close(shell); cx.notify(); }))));
    let mut filters = h_flex().flex_wrap().gap(t.space.small);
    for (id, label, kind) in [
        ("report-all", "All entries", None),
        ("report-skipped", "Skipped", Some(NoticeKind::Skipped)),
        ("report-unreadable", "No text", Some(NoticeKind::Unreadable)),
        ("report-failed", "Read failures", Some(NoticeKind::Failed)),
        ("report-partial", "Partial text", Some(NoticeKind::Partial)),
    ] {
        filters = filters.child(
            design::button(id, label, cx)
                .selected(view.kind == kind)
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.file_index.report.kind = kind;
                    shell.reload_scan_report(cx);
                })),
        );
    }
    panel = panel.child(filters);
    if state.loading {
        panel = panel.child(
            div()
                .sovra_text(TextRole::Body, cx)
                .child("Loading report…"),
        );
    }
    if let Some(error) = &state.error {
        panel = panel.child(div().sovra_text(TextRole::Body, cx).child(error.clone()));
    }
    if let Some(page) = &state.page {
        if page.scan_id.is_none() {
            return panel.child(div().sovra_text(TextRole::Body,cx).child("A full report has not been saved for this folder yet. Refresh the folder to create one. Earlier scans may have only 50 examples."));
        }
        let status = match page.status {
            IndexStatus::Ready => "Scan finished",
            IndexStatus::Pending => "Waiting for a scan — coverage is incomplete",
            IndexStatus::Scanning => "Scan in progress — reload for newer entries",
            IndexStatus::Cancelled => "Scan cancelled — coverage is incomplete",
            IndexStatus::Interrupted => "Scan interrupted — coverage is incomplete",
            IndexStatus::Failed => "Scan incomplete — check the reading problems below",
        };
        panel = panel.child(div().sovra_text(TextRole::Body, cx).child(status));
        if page.has_newer {
            panel = panel.child(div().sovra_text(TextRole::Body,cx).child("More entries were recorded after this snapshot. Reload the report to include them."));
        }
        let count = |kind| {
            page.counts
                .iter()
                .find(|(k, _)| *k == kind)
                .map_or(0, |(_, n)| *n)
        };
        panel = panel.child(div().id("scan-report-counts").test_support().sovra_text(TextRole::Caption,cx).child(format!("{} skipped · {} with no text · {} read failures · {} partial reads · {} matching entries · page {}",count(NoticeKind::Skipped),count(NoticeKind::Unreadable),count(NoticeKind::Failed),count(NoticeKind::Partial),page.total,view.cursors.len())));
        if page.notices.is_empty() {
            panel = panel.child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .child("No recorded entries match these filters."),
            );
        }
        for (index, notice) in page.notices.iter().enumerate() {
            panel = panel.child(
                v_flex()
                    .id(("scan-report-row", index))
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.compact)
                    .child(
                        div()
                            .id(("scan-report-path", index))
                            .w_full()
                            .min_w_0()
                            .text_ellipsis()
                            .aria_label(notice.path.clone())
                            .sovra_text(TextRole::Body, cx)
                            .child(notice.path.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .sovra_text(TextRole::Caption, cx)
                            .child(notice.message.clone()),
                    ),
            );
        }
        panel = panel.child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("scan-report-previous", "Previous page", cx)
                        .disabled(view.cursors.len() <= 1 || state.loading)
                        .on_click(cx.listener(|shell, _, _, cx| shell.scan_report_page(false, cx))),
                )
                .child(
                    design::button("scan-report-next", "Next page", cx)
                        .disabled(page.next_cursor.is_none() || state.loading)
                        .on_click(cx.listener(|shell, _, _, cx| shell.scan_report_page(true, cx))),
                ),
        );
    }
    panel
}

#[cfg(test)]
mod tests {
    use super::super::tests::{click_control, root};
    use super::*;
    use gpui_kit::component::{Root, Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, px, size};
    use sovra_core::{
        Event,
        file_index::{IndexError, IndexEvent, NoticePage, ScanNotice},
        models::InterfaceScale,
    };
    use sovra_runtime::Client;

    fn page() -> NoticePage {
        NoticePage { root_id: "chosen".into(), scan_id: Some(1), status: IndexStatus::Ready, total: 94, has_newer: false, counts: vec![(NoticeKind::Skipped,92),(NoticeKind::Unreadable,1),(NoticeKind::Partial,1)], notices: vec![ScanNotice { sequence: 1,kind:NoticeKind::Skipped,path:format!("{}/.notes", "long-folder/".repeat(40)),message:"Hidden files are switched off. Open folder settings to include them, then refresh.".into() }], next_cursor:Some(NoticeCursor { scan_id:1,after:1,through:94,filter_key:"k".into() }) }
    }

    #[gpui_kit::test]
    fn report_controls_page_filter_and_ignore_old_errors_through_client(cx: &mut TestAppContext) {
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
            click_control(window, cx, "scan-report-chosen");
        })
        .unwrap();
        let request = || {
            std::iter::from_fn(|| commands.try_recv().ok())
                .find_map(|command| {
                    if let Command::FileIndex(IndexCommand::Report { request_id, query }) = command
                    {
                        Some((request_id, query))
                    } else {
                        None
                    }
                })
                .unwrap()
        };
        let (id, query) = request();
        assert_eq!(query.root_id, "chosen");
        assert!(query.kind.is_none() && query.cursor.is_none());
        assert!(!query.match_case);
        events
            .send_blocking(Event::FileIndex(IndexEvent::Reported {
                request_id: id,
                result: Ok(page()),
            }))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            click_control(window, cx, "scan-report-next");
        })
        .unwrap();
        let (id, query) = request();
        assert_eq!(query.cursor, page().next_cursor);
        let mut newer = page();
        newer.has_newer = true;
        events
            .send_blocking(Event::FileIndex(IndexEvent::Reported {
                request_id: id,
                result: Ok(newer),
            }))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            click_control(window, cx, "report-partial");
        })
        .unwrap();
        let (_, query) = request();
        assert_eq!(query.kind, Some(NoticeKind::Partial));
        assert!(query.cursor.is_none());
        cx.update_window(window.into(), |_, window, cx| {
            click_control(window, cx, "report-unreadable");
        })
        .unwrap();
        let (old, query) = request();
        assert_eq!(query.kind, Some(NoticeKind::Unreadable));
        assert!(query.cursor.is_none());
        cx.update_window(window.into(), |_, window, cx| {
            click_control(window, cx, "report-match-case");
        })
        .unwrap();
        let (_, query) = request();
        assert!(query.match_case && query.cursor.is_none());
        assert_eq!(query.kind, Some(NoticeKind::Unreadable));
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell
                    .file_index
                    .report
                    .filter
                    .update(cx, |input, cx| input.set_value("blank", window, cx));
            });
            click_control(window, cx, "reload-scan-report");
        })
        .unwrap();
        let (current, query) = request();
        assert_eq!(query.contains.as_deref(), Some("blank"));
        events
            .send_blocking(Event::FileIndex(IndexEvent::Reported {
                request_id: old,
                result: Err(IndexError("old error".into())),
            }))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            assert!(shell.read(cx).file_index.state.report.loading);
            assert!(shell.read(cx).file_index.state.report.error.is_none());
            click_control(window, cx, "close-scan-report");
        })
        .unwrap();
        events
            .send_blocking(Event::FileIndex(IndexEvent::Reported {
                request_id: current,
                result: Ok(page()),
            }))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("index-scan-report").is_none());
            assert!(window.find("index-details-chosen").bounds().size.height > px(0.));
            assert!(shell.read(cx).file_index.report.root.is_none());
            assert!(shell.read(cx).file_index.state.report.page.is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn scan_reports_fit_minimum_wide_and_double_text_in_both_themes(cx: &mut TestAppContext) {
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
                    shell.open_scan_report("chosen", window, cx);
                    shell.file_index.state.report.page = Some(page());
                    shell.file_index.state.report.loading = false;
                });
                for mode in [ThemeMode::Dark, ThemeMode::Light] {
                    Theme::change(mode, Some(window), cx);
                    window.render_frame(cx);
                    window.render_frame(cx);
                    for id in [
                        "index-scan-report",
                        "reload-scan-report",
                        "report-match-case",
                        "refresh-report-folder",
                        "close-scan-report",
                        "report-unreadable",
                        "report-failed",
                        "report-partial",
                        "scan-report-counts",
                        "scan-report-next",
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
