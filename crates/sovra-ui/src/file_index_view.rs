//! Native folder grants and index management; all reads and scans belong to runtime.

use gpui_kit::component::{
    Disableable as _, Selectable as _,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Task, TestSupportExt as _, Window, div,
};
use sovra_core::{
    Command, NavTab,
    file_index::{
        IndexCommand, IndexQuery, IndexState, IndexStatus, IndexedRoot, MonitorMode, MonitorStatus,
    },
    skills::{SkillAction, SkillKind},
};

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, Surface, TextRole},
    shell::Shell,
};

mod browser;
mod knowledge;
mod live;
mod report;
mod selection;

pub(crate) struct FileIndexView {
    knowledge: knowledge::KnowledgeView,
    browser: browser::BrowserView,
    report: report::ReportView,
    pub(crate) state: IndexState,
    search: Entity<InputState>,
    _search: gpui_kit::Subscription,
    removing: Option<String>,
    detailed_roots: std::collections::BTreeSet<String>,
    options: std::collections::BTreeMap<String, sovra_core::file_index::ScanOptions>,
    picker: Option<Task<()>>,
    semantic_root: Option<String>,
    embedding_search: Entity<InputState>,
    _embedding_search: gpui_kit::Subscription,
}

impl FileIndexView {
    pub(crate) fn apply(&mut self, event: &sovra_core::file_index::IndexEvent) {
        self.state.apply(event);
        if matches!(event, sovra_core::file_index::IndexEvent::FoldersAdded(_)) {
            self.state.notice = None;
        }
        if matches!(event, sovra_core::file_index::IndexEvent::Error(_)) {
            self.options.clear();
        } else {
            self.options.retain(|id, options| {
                self.state
                    .roots
                    .iter()
                    .any(|root| root.id == *id && root.scan_options != *options)
            });
        }
    }

    pub(crate) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find words in your files…"));
        let search_subscription =
            cx.subscribe_in(&search, window, |shell, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && !shell.file_index.state.searching
                {
                    shell.search_index(cx);
                }
            });
        let embedding_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a search helper…"));
        let subscription = cx.subscribe_in(
            &embedding_search,
            window,
            |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        Self {
            knowledge: knowledge::KnowledgeView::new(window, cx),
            browser: browser::BrowserView::new(window, cx),
            report: report::ReportView::new(window, cx),
            state: IndexState::default(),
            search,
            _search: search_subscription,
            removing: None,
            detailed_roots: std::collections::BTreeSet::new(),
            options: std::collections::BTreeMap::new(),
            picker: None,
            semantic_root: None,
            embedding_search,
            _embedding_search: subscription,
        }
    }
}

impl Shell {
    pub(crate) fn choose_index_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let original = self.composer_input.read(cx).value().to_string();
        let session = self.state.session_id.clone();
        let remaining = self
            .skills
            .resolve(&original)
            .ok()
            .flatten()
            .filter(|call| call.skill.kind == SkillKind::Action(SkillAction::IndexFolder))
            .map(|call| call.arguments.to_owned());
        let selection = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add selected folders".into()),
        });
        self.file_index.picker = Some(cx.spawn_in(window, async move |this, cx| {
            let result = selection.await;
            let _ = this.update_in(cx, |shell, window, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if !paths.is_empty() {
                            if shell.state.session_id == session
                                && shell.composer_input.read(cx).value().as_ref() == original
                                && let Some(remaining) = remaining
                            {
                                shell.fill_composer_draft(&remaining, window, cx);
                            }
                            shell.add_index_folders(paths, cx);
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => {
                        shell.file_index.state.notice =
                            Some("The folder picker could not open. Try Add folders again.".into());
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(crate) fn index_command(&mut self, command: IndexCommand, cx: &mut Context<Self>) {
        let knowledge_request = match &command {
            IndexCommand::BrowseKnowledge { request_id, .. } => Some((*request_id, false)),
            IndexCommand::KnowledgeEvidence { request_id, .. } => Some((*request_id, true)),
            _ => None,
        };
        let report_id = if let IndexCommand::Report { request_id, .. } = &command {
            Some(*request_id)
        } else {
            None
        };
        let browse_id = if let IndexCommand::Browse { request_id, .. } = &command {
            Some(*request_id)
        } else {
            None
        };
        let preview_id = if let IndexCommand::Preview { request_id, .. } = &command {
            Some(*request_id)
        } else {
            None
        };
        if let Err(error) = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::FileIndex(command))
        {
            if let Some((request_id, evidence)) = knowledge_request {
                let error = sovra_core::file_index::IndexError(error.to_string());
                self.file_index.state.apply(&if evidence {
                    sovra_core::file_index::IndexEvent::KnowledgeEvidence {
                        request_id,
                        result: Err(error),
                    }
                } else {
                    sovra_core::file_index::IndexEvent::KnowledgeBrowsed {
                        request_id,
                        result: Err(error),
                    }
                });
            }
            if let Some(request_id) = report_id {
                self.file_index
                    .state
                    .apply(&sovra_core::file_index::IndexEvent::Reported {
                        request_id,
                        result: Err(sovra_core::file_index::IndexError(error.to_string())),
                    });
            }
            if let Some(request_id) = browse_id {
                self.file_index
                    .state
                    .apply(&sovra_core::file_index::IndexEvent::Browsed {
                        request_id,
                        result: Err(sovra_core::file_index::IndexError(error.to_string())),
                    });
            }
            if let Some(request_id) = preview_id {
                self.file_index
                    .state
                    .apply(&sovra_core::file_index::IndexEvent::Previewed {
                        request_id,
                        result: Err(sovra_core::file_index::IndexError(error.to_string())),
                    });
            }
            self.file_index
                .state
                .apply(&sovra_core::file_index::IndexEvent::Error(
                    error.to_string(),
                ));
            self.file_index.options.clear();
        }
        cx.notify();
    }

    fn change_index_options(
        &mut self,
        id: &str,
        update: impl FnOnce(&mut sovra_core::file_index::ScanOptions),
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self
            .file_index
            .state
            .roots
            .iter()
            .find(|root| root.id == id)
        else {
            return;
        };
        let mut options = self
            .file_index
            .options
            .get(id)
            .copied()
            .unwrap_or(root.scan_options);
        update(&mut options);
        self.file_index.options.insert(id.into(), options);
        self.index_command(
            IndexCommand::SetScanOptions {
                root_id: id.into(),
                options,
            },
            cx,
        );
    }

    fn search_index(&mut self, cx: &mut Context<Self>) {
        let query = IndexQuery {
            text: self.file_index.search.read(cx).value().to_string(),
            roots: vec![],
            path_prefix: None,
            limit: 20,
        };
        if let Err(error) = query.validate() {
            self.file_index
                .state
                .apply(&sovra_core::file_index::IndexEvent::Error(
                    error.to_string(),
                ));
            cx.notify();
            return;
        }
        let request_id = self.file_index.state.begin_search();
        self.index_command(IndexCommand::Search { request_id, query }, cx);
    }
}

pub(crate) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    if shell.file_index.knowledge.active {
        return knowledge::render(shell, cx).into_any_element();
    }
    let t = design::tokens(cx);
    let state = &shell.file_index.state;
    if browser::is_open(shell) {
        return browser::render(shell, cx).into_any_element();
    }
    if report::active_root(shell).is_some() {
        return report::render(shell, cx).into_any_element();
    }
    let capture_summary = state.capture_summary();
    let mut panel = v_flex().id("file-index-panel").test_support().min_w_0().w_full().gap(t.space.small)
        .sovra_surface(Surface::Matte, cx).p(t.space.medium)
        .child(div().sovra_text(TextRole::SectionTitle, cx).child("Your folders"))
        .child(h_flex().w_full().min_w_0().flex_wrap().gap(t.space.small)
            .child(design::button("index-folder", "Add folders", cx).max_w_full().on_click(cx.listener(|shell, _, window, cx| shell.choose_index_folder(window, cx))))
            .child(knowledge::entry(cx)))
        .child(div().sovra_text(TextRole::Caption, cx).child("Choose one or more folders to search on this computer. Chat sends requested file details and saved passages to your selected AI service. Files stay in their original folders."))
        .children(state.notice.as_ref().map(|notice| div().sovra_text(TextRole::Body, cx).child(notice.clone())))
        .children(selection::render(shell, cx))
        .children((!state.roots.is_empty()).then(|| div().id("index-capture-summary").test_support()
            .role(gpui_kit::Role::Status).aria_label(capture_summary.clone())
            .min_w_0().sovra_text(TextRole::Body, cx).child(capture_summary)))
        .children((!state.roots.is_empty()).then(|| render_search(shell, cx)))
        .children((!state.roots.is_empty()).then(|| browser::render(shell, cx)))
        .children(state.roots.iter().map(|root| root_row(root, state.monitors.get(&root.id), state.semantic.iter().find(|s| s.setting.root_id == root.id), shell.file_index.removing.as_deref() == Some(root.id.as_str()), shell.file_index.detailed_roots.contains(&root.id), shell.file_index.options.get(&root.id).copied(), cx)));
    if let Some(root_id) = shell
        .file_index
        .semantic_root
        .as_ref()
        .filter(|id| state.roots.iter().any(|r| r.id == **id))
    {
        let candidates = sovra_core::file_index::semantic::embedding_targets(
            &shell.models().read(cx).state,
            shell.file_index.embedding_search.read(cx).value().as_ref(),
        );
        let mut chooser = v_flex().id("semantic-chooser").test_support().min_w_0().gap(t.space.small)
            .child(div().sovra_text(TextRole::SectionTitle,cx).child("Choose a search helper"))
            .child(div().sovra_text(TextRole::Caption,cx).child("Enabling sends this folder’s extracted passages and future searches to the named server, including newly indexed files. The service may charge for usage. No model is downloaded automatically. Use a model designed for plain-text embeddings; models requiring task prefixes are not supported yet."))
            .child(Input::new(&shell.file_index.embedding_search).aria_label("Filter search helpers"));
        if candidates.is_empty() {
            chooser = chooser.child(div().sovra_text(TextRole::Body,cx).child("No matching models. In Models, refresh OpenRouter, inspect an installed Ollama embedding model to load its capabilities, or add an embedding model ID to an OpenAI-compatible connection."));
        }
        for (index, target) in candidates.iter().take(20).enumerate() {
            let root_id = root_id.clone();
            let target = target.clone();
            let selection = target.clone();
            chooser = chooser.child(crate::widgets::model_destination::choice(
                &target,
                gpui_kit::SharedString::from(format!("semantic-destination-{index}")).into(),
                design::button(("enable-semantic", index), "Enable for this folder", cx).on_click(
                    cx.listener(move |shell, _, _, cx| {
                        shell.index_command(
                            IndexCommand::ConfigureSemantic {
                                root_id: root_id.clone(),
                                target: Some(selection.clone()),
                            },
                            cx,
                        );
                        shell.file_index.semantic_root = None;
                    }),
                ),
                cx,
            ));
        }
        if candidates.len() > 20 {
            chooser =
                chooser.child(div().sovra_text(TextRole::Caption, cx).child(
                    "Showing 20 choices. Filter by model or connection name to find another.",
                ));
        }
        panel = panel.child(chooser.child(
            design::button("cancel-semantic-choice", "Cancel", cx).on_click(cx.listener(
                |shell, _, _, cx| {
                    shell.file_index.semantic_root = None;
                    cx.notify();
                },
            )),
        ));
    }
    panel
        .child(crate::knowledge_review::render(state, cx))
        .into_any_element()
}

fn render_search(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let state = &shell.file_index.state;
    let mut panel = v_flex().w_full().min_w_0().gap(t.space.small);
    if !state.roots.is_empty() {
        panel = panel
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Find something you saved"),
            )
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .child("Enter a word or phrase from a file to see its saved text and source."),
            )
            .child(Input::new(&shell.file_index.search).aria_label("Search indexed files"))
            .child(
                design::button(
                    "search-index",
                    if state.searching {
                        "Searching…"
                    } else {
                        "Search files"
                    },
                    cx,
                )
                .disabled(state.searching)
                .on_click(cx.listener(|shell, _, _, cx| shell.search_index(cx))),
            );
    }
    if state.searching || state.search_completed {
        let label = if state.searching {
            "Searching saved file text…".to_owned()
        } else if state.passages.is_empty() {
            "No matching text found. Try fewer words, or check that the folder has finished reading.".to_owned()
        } else if state.passages.len() == 1 {
            "1 matching passage found. Its saved text is shown below.".to_owned()
        } else {
            format!(
                "{} matching passages found. Their saved text is shown below.",
                state.passages.len()
            )
        };
        panel = panel.child(
            div()
                .id("index-search-result")
                .test_support()
                .role(gpui_kit::Role::Status)
                .aria_label(label.clone())
                .min_w_0()
                .sovra_text(TextRole::Body, cx)
                .child(label),
        );
    }
    for (index, passage) in state
        .passages
        .iter()
        .filter(|_| state.search_completed)
        .enumerate()
    {
        let preview: String = passage.text.chars().take(800).collect();
        let mut result = v_flex()
            .id(("index-search-passage", index))
            .test_support()
            .gap(t.space.compact)
            .p(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(div().sovra_text(TextRole::Caption, cx).child(format!(
                "{} · lines {}–{}",
                passage.path, passage.start_line, passage.end_line
            )))
            .child(div().sovra_text(TextRole::Body, cx).child(preview));
        if let Some(note) = &passage.note {
            result = result.child(div().sovra_text(TextRole::Caption, cx).child(note.clone()));
        }
        panel = panel.child(result);
    }
    panel
}

fn root_row(
    root: &IndexedRoot,
    monitor: Option<&MonitorStatus>,
    semantic: Option<&sovra_core::file_index::semantic::SemanticStatus>,
    removing: bool,
    detailed: bool,
    pending_options: Option<sovra_core::file_index::ScanOptions>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let has_notices =
        !root.issues.is_empty() || root.progress.skipped > 0 || root.progress.failed > 0;
    let status = match root.status {
        IndexStatus::Pending => {
            if root.auto_refresh {
                "Waiting to read this folder"
            } else {
                "Paused — turn on automatic updates to read it"
            }
        }
        IndexStatus::Scanning => "Reading your files…",
        IndexStatus::Ready if has_notices => "Done — a few files need a look",
        IndexStatus::Ready => "Done — ready to search",
        IndexStatus::Cancelled => "Stopped — press Refresh to carry on",
        IndexStatus::Interrupted => "Interrupted — press Refresh to carry on",
        IndexStatus::Failed => "Couldn't finish reading this folder",
    };
    let mut row = v_flex()
        .min_w_0()
        .gap(t.space.small)
        .p(t.space.small)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            div()
                .id(gpui_kit::SharedString::from(format!(
                    "index-path-{}",
                    root.id
                )))
                .min_w_0()
                .text_ellipsis()
                .aria_label(root.path.display().to_string())
                .sovra_text(TextRole::Body, cx)
                .child(root.path.display().to_string()),
        );
    row = if root.status == IndexStatus::Scanning {
        row.child(live::reading_panel(root, cx))
    } else {
        let progress = &root.progress;
        let row = row.child(
            div()
                .id(gpui_kit::SharedString::from(format!(
                    "index-capture-status-{}",
                    root.id
                )))
                .test_support()
                .role(gpui_kit::Role::Status)
                .aria_label(status)
                .sovra_text(TextRole::Body, cx)
                .child(status),
        );
        let row = match live::found_summary(root, cx) {
            Some(found) => row.child(found),
            None => row,
        };
        // The plain-number detail stays for people who want it, in everyday words.
        row.child(div().sovra_text(TextRole::Caption, cx).child(format!(
            "Last time: {} new or changed · {} already saved · {} left out · {} couldn't be read",
            sovra_core::file_index::ontology::group_digits(progress.indexed),
            sovra_core::file_index::ontology::group_digits(progress.reused),
            sovra_core::file_index::ontology::group_digits(progress.skipped),
            sovra_core::file_index::ontology::group_digits(progress.failed),
        )))
    };
    if has_notices {
        row = row.child(div()
            .id(gpui_kit::SharedString::from(format!("index-attention-{}", root.id)))
            .test_support().min_w_0()
            .sovra_text(TextRole::Body, cx)
            .child("Some items were skipped or could not be fully read. Open Scan report for details and recovery steps."));
    }
    if root.status == IndexStatus::Failed {
        row = row.child(div()
            .id(gpui_kit::SharedString::from(format!("index-recovery-{}", root.id)))
            .test_support().min_w_0()
            .sovra_text(TextRole::Body, cx)
            .child("Saved results may include older versions. Open Scan report to see what could not be checked, fix the problem, then retry the scan."));
    }
    let monitoring = match monitor.map(|status| status.mode) {
        Some(MonitorMode::Native) => {
            "Automatic updates · native notifications and checks every 15 minutes"
        }
        Some(MonitorMode::Periodic) => "Automatic updates · checks every 15 minutes",
        None if root.auto_refresh => "Starting automatic updates…",
        Some(MonitorMode::Paused) | None => "Automatic updates paused",
    };
    row = row.child(div().sovra_text(TextRole::Caption, cx).child(monitoring));
    if let Some(detail) = monitor.and_then(|status| status.detail.as_ref()) {
        row = row.child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .child(detail.clone()),
        );
    }
    let id = root.id.clone();
    let cancel_id = id.clone();
    let remove_id = id.clone();
    let mut actions = h_flex().flex_wrap().gap(t.space.small);
    let report_id = id.clone();
    actions = actions.child(
        design::button(
            gpui_kit::SharedString::from(format!("scan-report-{id}")),
            "Scan report",
            cx,
        )
        .on_click(
            cx.listener(move |shell, _, window, cx| shell.open_scan_report(&report_id, window, cx)),
        ),
    );
    let browse_id = id.clone();
    actions = actions.child(
        design::button(
            gpui_kit::SharedString::from(format!("browse-index-{id}")),
            "Browse files",
            cx,
        )
        .on_click(cx.listener(move |shell, _, window, cx| {
            shell.browse_index_folder(Some(browse_id.clone()), window, cx);
        })),
    );
    if detailed {
        if let Some(status) = semantic {
            row = row.child(
                div()
                    .min_w_0()
                    .sovra_text(TextRole::Caption, cx)
                    .child(format!(
                        "Search by meaning · {} · {} of {} unique passages · {}",
                        status.setting.target.key.model_id,
                        status.embedded,
                        status.total,
                        status.setting.target.base_url
                    )),
            );
            if let Some(message) = &status.message {
                row = row.child(div().sovra_text(TextRole::Body, cx).child(message.clone()));
            }
            let disable = id.clone();
            actions = actions.child(
                design::button(
                    gpui_kit::SharedString::from(format!("disable-semantic-{id}")),
                    "Turn off search by meaning",
                    cx,
                )
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.index_command(
                        IndexCommand::ConfigureSemantic {
                            root_id: disable.clone(),
                            target: None,
                        },
                        cx,
                    );
                })),
            );
        }
        let choose = id.clone();
        actions = actions.child(
            design::button(
                gpui_kit::SharedString::from(format!("choose-semantic-{id}")),
                if semantic.is_some() {
                    "Change search helper"
                } else {
                    "Find by meaning (optional)"
                },
                cx,
            )
            .on_click(cx.listener(move |shell, _, _, cx| {
                shell.file_index.semantic_root = Some(choose.clone());
                cx.notify();
            })),
        );
    }
    let automatic_id = id.clone();
    let enabled =
        !root.auto_refresh || monitor.is_some_and(|status| status.mode == MonitorMode::Paused);
    actions = actions.child(
        design::button(
            gpui_kit::SharedString::from(format!("automatic-index-{id}")),
            if enabled {
                "Resume updates"
            } else {
                "Pause updates"
            },
            cx,
        )
        .on_click(cx.listener(move |shell, _, _, cx| {
            shell.index_command(
                IndexCommand::SetAutoRefresh {
                    root_id: automatic_id.clone(),
                    enabled,
                },
                cx,
            );
        })),
    );
    if root.status == IndexStatus::Scanning {
        actions = actions.child(
            design::button(
                gpui_kit::SharedString::from(format!("cancel-index-{id}")),
                "Cancel",
                cx,
            )
            .on_click(cx.listener(move |shell, _, _, cx| {
                shell.index_command(
                    IndexCommand::Cancel {
                        root_id: cancel_id.clone(),
                    },
                    cx,
                );
            })),
        );
    } else {
        actions = actions.child(
            design::button(
                gpui_kit::SharedString::from(format!("refresh-index-{id}")),
                if matches!(
                    root.status,
                    IndexStatus::Failed | IndexStatus::Interrupted | IndexStatus::Cancelled
                ) {
                    "Retry scan"
                } else {
                    "Refresh"
                },
                cx,
            )
            .on_click(cx.listener(move |shell, _, _, cx| {
                shell.index_command(
                    IndexCommand::Refresh {
                        root_id: id.clone(),
                    },
                    cx,
                );
            })),
        );
    }
    actions = actions.child(
        design::button(
            gpui_kit::SharedString::from(format!("remove-index-{remove_id}")),
            "Remove",
            cx,
        )
        .on_click(cx.listener(move |shell, _, _, cx| {
            shell.file_index.removing = Some(remove_id.clone());
            cx.notify();
        })),
    );
    row = row.child(actions);
    if removing {
        let id = root.id.clone();
        row = row.child(div().sovra_text(TextRole::Body, cx).child("Remove this folder's saved index and access grant? The original files stay in place. Knowledge review history stays on this device but becomes unavailable, including after re-adding this folder."))
            .child(h_flex().flex_wrap().gap(t.space.small)
                .child(design::button(gpui_kit::SharedString::from(format!("confirm-remove-index-{id}")), "Remove index", cx).on_click(cx.listener(move |shell, _, _, cx| { shell.file_index.removing = None; shell.index_command(IndexCommand::Remove { root_id: id.clone() }, cx); })))
                .child(design::button(gpui_kit::SharedString::from(format!("keep-index-{}", root.id)), "Keep", cx).on_click(cx.listener(|shell, _, _, cx| { shell.file_index.removing = None; cx.notify(); }))));
    }
    let details_id = root.id.clone();
    row = row.child(
        design::button(
            gpui_kit::SharedString::from(format!("index-details-{}", root.id)),
            if detailed {
                "Hide folder details"
            } else {
                "Folder settings"
            },
            cx,
        )
        .on_click(cx.listener(move |shell, _, _, cx| {
            if !shell.file_index.detailed_roots.remove(&details_id) {
                shell.file_index.detailed_roots.insert(details_id.clone());
            }
            cx.notify();
        })),
    );
    if detailed {
        let options = pending_options.unwrap_or(root.scan_options);
        let hidden_id = root.id.clone();
        let generated_id = root.id.clone();
        row = row
            .child(div().sovra_text(TextRole::SectionTitle, cx).child("What to include"))
            .child(Checkbox::new(gpui_kit::SharedString::from(format!("index-hidden-{}", root.id)))
                .cursor_pointer()
                .label("Include hidden files and folders").checked(options.include_hidden)
                .on_click(cx.listener(move |shell, checked, _, cx| shell.change_index_options(&hidden_id, |options| options.include_hidden = *checked, cx))))
            .child(Checkbox::new(gpui_kit::SharedString::from(format!("index-generated-{}", root.id)))
                .cursor_pointer()
                .label("Include generated folders").checked(options.include_generated)
                .on_click(cx.listener(move |shell, checked, _, cx| shell.change_index_options(&generated_id, |options| options.include_generated = *checked, cx))))
            .child(div().sovra_text(TextRole::Caption, cx).child("Generated folders include build, target and node_modules. Hidden files may contain private settings. Changes refresh this folder; excluded files leave current search, while saved history is kept. Credential folders and Sovra's own index data remain excluded."));
        let mut budgets = h_flex().flex_wrap().gap(t.space.small);
        for (index, budget) in sovra_core::file_index::DocumentBudget::ALL
            .into_iter()
            .enumerate()
        {
            let id = root.id.clone();
            budgets = budgets.child(
                design::button(
                    gpui_kit::SharedString::from(format!("index-documents-{index}-{id}")),
                    budget.label(),
                    cx,
                )
                .selected(options.documents == budget)
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.change_index_options(&id, |options| options.documents = budget, cx);
                })),
            );
        }
        row = row.child(div().sovra_text(TextRole::SectionTitle, cx).child("Document reading"))
            .child(budgets)
            .child(div().sovra_text(TextRole::Body, cx).child(options.documents.details()))
            .child(div().sovra_text(TextRole::Caption, cx).child("Larger budgets use more memory; peak usage can be several times the document size. Changes refresh this folder. Text and source files stream without a file-size cutoff. Nested archive and decompression protections still apply."));
        if pending_options.is_some() {
            row = row.child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .child("Applying folder settings…"),
            );
        }
        if !root.issues.is_empty() {
            row = row.child(div().sovra_text(TextRole::SectionTitle, cx).child("Scan notices"))
                .child(div().sovra_text(TextRole::Caption, cx).child(format!("Showing {} of {} saved examples. The full scan report includes all recorded skips and reading problems. Excluded folders appear once, without visiting their contents.", root.issues.len().min(3), root.issues.len())));
            let report_id = root.id.clone();
            row = row.child(
                design::button(
                    gpui_kit::SharedString::from(format!("index-notices-{}", root.id)),
                    "Open full scan report",
                    cx,
                )
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.open_scan_report(&report_id, window, cx);
                })),
            );
            for (index, issue) in root.issues.iter().take(3).enumerate() {
                row = row.child(
                    div()
                        .id(gpui_kit::SharedString::from(format!(
                            "index-issue-{}-{index}",
                            root.id
                        )))
                        .test_support()
                        .w_full()
                        .min_w_0()
                        .sovra_text(TextRole::Caption, cx)
                        .child(issue.clone()),
                );
            }
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{Root, Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, px, size};
    use sovra_core::{
        Event,
        file_index::{IndexEvent, IndexProgress},
        models::InterfaceScale,
    };
    use sovra_runtime::Client;

    pub(super) fn root() -> IndexedRoot {
        IndexedRoot {
            id: "chosen".into(),
            path: format!("/selected/{}/notes", "long-folder-name".repeat(24)).into(),
            status: IndexStatus::Ready,
            progress: IndexProgress::default(),
            issues: (0..12).map(|n| format!("File {n}: contents could not be read. Check its format and refresh to retry.")).collect(),
            completed_at: Some(42),
            auto_refresh: true,
            scan_options: sovra_core::file_index::ScanOptions::default(),
        }
    }

    pub(super) fn click_control(window: &mut Window, cx: &mut gpui_kit::App, id: &str) {
        for _ in 0..60 {
            window.render_frame(cx);
            let element = window.find(gpui_kit::SharedString::from(id.to_owned()));
            let bounds = element.bounds();
            if element.visible() && bounds.top() >= px(0.) && bounds.bottom() < px(940.) {
                window.click(gpui_kit::SharedString::from(id.to_owned()), cx);
                return;
            }
            let delta = if bounds.top() < px(0.) { 80. } else { -80. };
            window.scroll(
                "sovra-knowledge-view",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(delta))),
                cx,
            );
        }
        panic!("control did not become fully visible: {id}");
    }

    #[gpui_kit::test]
    fn file_search_reports_empty_results_and_enter_submits_once(cx: &mut TestAppContext) {
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1280.), px(1000.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.file_index.apply(&IndexEvent::Roots(vec![root()]));
                shell.select_tab(NavTab::Knowledge, cx);
                shell.select_knowledge_section(crate::knowledge_view::KnowledgeSection::Files, cx);
                shell.file_index.search.update(cx, |input, cx| {
                    input.set_value("tea", window, cx);
                    cx.emit(InputEvent::PressEnter {
                        secondary: false,
                        shift: false,
                    });
                });
            });
        })
        .unwrap();
        cx.run_until_parked();
        let requests: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| {
                if let Command::FileIndex(IndexCommand::Search { request_id, query }) = command {
                    Some((request_id, query))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1.text, "tea");
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("index-search-result").label(),
                Some("Searching saved file text…")
            );
            assert!(window.try_find("choose-semantic-chosen").is_none());
            assert!(
                window.find("index-capture-summary").bounds().top()
                    < window.find("search-index").bounds().top()
            );
            assert!(
                window.find("search-index").bounds().top()
                    < window.find("index-capture-status-chosen").bounds().top()
            );
            let search = shell.read(cx).file_index.search.clone();
            search.update(cx, |_, cx| {
                cx.emit(InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        })
        .unwrap();
        cx.run_until_parked();
        assert!(
            !std::iter::from_fn(|| commands.try_recv().ok())
                .any(|command| matches!(command, Command::FileIndex(IndexCommand::Search { .. })))
        );
        events
            .send_blocking(Event::FileIndex(IndexEvent::Results {
                request_id: requests[0].0,
                passages: vec![],
            }))
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .find("index-search-result")
                    .label()
                    .unwrap()
                    .starts_with("No matching text found.")
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_failed_search_never_displays_previous_results(cx: &mut TestAppContext) {
        let (client, commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1280.), px(1000.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.select_tab(NavTab::Knowledge, cx);
                shell.select_knowledge_section(crate::knowledge_view::KnowledgeSection::Files, cx);
                shell.file_index.apply(&IndexEvent::Roots(vec![root()]));
                let request_id = shell.file_index.state.begin_search();
                shell.file_index.apply(&IndexEvent::Results {
                    request_id,
                    passages: vec![sovra_core::file_index::IndexPassage {
                        reference: sovra_core::file_index::PassageRef {
                            root_id: "chosen".into(),
                            file_id: "file".into(),
                            version_id: "v1".into(),
                            ordinal: 0,
                        },
                        path: "notes.txt".into(),
                        text: "I prefer tea without sugar.".into(),
                        start_line: 1,
                        end_line: 1,
                        mime_type: "text/plain".into(),
                        note: None,
                        observed_at: 1,
                        modified_at: None,
                    }],
                });
            });
            window.render_frame(cx);
            assert!(window.try_find(("index-search-passage", 0usize)).is_some());
            assert!(
                window
                    .find("index-search-result")
                    .label()
                    .unwrap()
                    .starts_with("1 matching passage")
            );
            shell.update(cx, |shell, cx| {
                shell.file_index.state.begin_search();
                shell.file_index.apply(&IndexEvent::Error(
                    "Search could not finish. Try again.".into(),
                ));
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.try_find(("index-search-passage", 0usize)).is_none());
            assert!(window.try_find("index-search-result").is_none());
            shell.update(cx, |shell, cx| {
                let passages = shell.file_index.state.passages.clone();
                let request_id = shell.file_index.state.begin_search();
                shell.file_index.apply(&IndexEvent::Results {
                    request_id,
                    passages,
                });
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.try_find(("index-search-passage", 0usize)).is_some());
            shell.update(cx, |shell, cx| {
                shell
                    .file_index
                    .search
                    .update(cx, |input, cx| input.set_value("", window, cx));
                shell.search_index(cx);
            });
            window.render_frame(cx);
            assert!(
                window.try_find(("index-search-passage", 0usize)).is_none(),
                "An invalid new search must not retain previous matches."
            );
            assert!(window.try_find("index-search-result").is_none());
        })
        .unwrap();
        drop(commands);
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell
                    .file_index
                    .search
                    .update(cx, |input, cx| input.set_value("coffee", window, cx));
                shell.search_index(cx);
                assert!(
                    !shell.file_index.state.searching,
                    "A disconnected runtime must not leave search stuck."
                );
                assert!(shell.file_index.state.notice.is_some());
            });
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn controls_send_pause_refresh_and_confirmed_removal_without_touching_sources(
        cx: &mut TestAppContext,
    ) {
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1280.), px(1000.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let mut incomplete = root();
        incomplete.status = IndexStatus::Failed;
        events
            .send_blocking(Event::FileIndex(IndexEvent::Roots(vec![incomplete])))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.select_tab(NavTab::Knowledge, cx);
                shell.select_knowledge_section(crate::knowledge_view::KnowledgeSection::Files, cx);
            });
            window.render_frame(cx);
            assert!(window.find("index-recovery-chosen").bounds().size.height > px(0.));
            for id in [
                "automatic-index-chosen",
                "refresh-index-chosen",
                "index-details-chosen",
                "index-hidden-chosen",
                "index-documents-2-chosen",
                "index-documents-0-chosen",
                "remove-index-chosen",
            ] {
                click_control(window, cx, id);
            }
            assert!(shell.read(cx).file_index.detailed_roots.contains("chosen"));
            assert!(shell.read(cx).file_index.removing.is_some());
        })
        .unwrap();
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(
            sent.contains(&Command::FileIndex(IndexCommand::SetAutoRefresh {
                root_id: "chosen".into(),
                enabled: false
            }))
        );
        assert!(sent.contains(&Command::FileIndex(IndexCommand::Refresh {
            root_id: "chosen".into()
        })));
        let options: Vec<_> = sent
            .iter()
            .filter_map(|command| {
                if let Command::FileIndex(IndexCommand::SetScanOptions { options, .. }) = command {
                    Some(*options)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(options.len(), 3);
        assert!(options.iter().all(|options| options.include_hidden));
        assert_eq!(
            options[1].documents,
            sovra_core::file_index::DocumentBudget::Large
        );
        assert_eq!(
            options[2].documents,
            sovra_core::file_index::DocumentBudget::LowerMemory
        );
        assert!(
            !sent
                .iter()
                .any(|command| matches!(command, Command::FileIndex(IndexCommand::Remove { .. })))
        );
        cx.update_window(window.into(), |_, window, cx| {
            click_control(window, cx, "confirm-remove-index-chosen");
        })
        .unwrap();
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::FileIndex(IndexCommand::Remove { root_id }) if root_id == "chosen")));
    }

    #[gpui_kit::test]
    fn completed_scans_surface_notices_before_opening_folder_settings(cx: &mut TestAppContext) {
        let (client, commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1280.), px(1000.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        let with_notes = root();
        let mut clean = root();
        clean.issues.clear();
        let mut skipped = clean.clone();
        skipped.progress.skipped = 1;
        cx.update_window(window.into(), |_, window, cx| {
            for (root, visible) in [(with_notes, true), (clean, false), (skipped, true)] {
                shell.update(cx, |shell, cx| {
                    shell.file_index.apply(&IndexEvent::Roots(vec![root]));
                    shell.select_tab(NavTab::Knowledge, cx);
                    shell.select_knowledge_section(
                        crate::knowledge_view::KnowledgeSection::Files,
                        cx,
                    );
                });
                window.render_frame(cx);
                window.render_frame(cx);
                assert_eq!(window.try_find("index-attention-chosen").is_some(), visible);
                assert!(shell.read(cx).file_index.detailed_roots.is_empty());
            }
            click_control(window, cx, "scan-report-chosen");
        })
        .unwrap();
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::FileIndex(IndexCommand::Report { query, .. }) if query.root_id == "chosen"))
        );
    }

    #[gpui_kit::test]
    fn long_folder_paths_and_controls_fit_minimum_wide_and_double_text_sizes(
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
                    let mut incomplete = root();
                    incomplete.status = IndexStatus::Failed;
                    shell
                        .file_index
                        .state
                        .apply(&IndexEvent::Roots(vec![incomplete]));
                    shell.file_index.detailed_roots.insert("chosen".into());
                    shell.select_tab(NavTab::Knowledge, cx);
                    shell.select_knowledge_section(
                        crate::knowledge_view::KnowledgeSection::Files,
                        cx,
                    );
                });
                for mode in [ThemeMode::Dark, ThemeMode::Light] {
                    Theme::change(mode, Some(window), cx);
                    window.render_frame(cx);
                    window.render_frame(cx);
                    assert!(window.try_find("index-issue-chosen-3").is_none());
                    for id in [
                        "file-index-panel",
                        "index-capture-summary",
                        "index-capture-status-chosen",
                        "index-attention-chosen",
                        "index-recovery-chosen",
                        "automatic-index-chosen",
                        "refresh-index-chosen",
                        "remove-index-chosen",
                        "search-index",
                        "index-details-chosen",
                        "index-hidden-chosen",
                        "index-generated-chosen",
                        "index-documents-0-chosen",
                        "index-documents-1-chosen",
                        "index-documents-2-chosen",
                        "index-notices-chosen",
                        "index-issue-chosen-2",
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
