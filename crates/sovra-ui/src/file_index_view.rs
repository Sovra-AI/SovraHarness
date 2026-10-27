//! Native folder grants and index management; all reads and scans belong to runtime.

use gpui_kit::component::{
    Disableable as _, h_flex,
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

pub(crate) struct FileIndexView {
    pub(crate) state: IndexState,
    search: Entity<InputState>,
    removing: Option<String>,
    picker: Option<Task<()>>,
    semantic_root: Option<String>,
    embedding_search: Entity<InputState>,
    _embedding_search: gpui_kit::Subscription,
}

impl FileIndexView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let embedding_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter embedding models…"));
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
            state: IndexState::default(),
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search indexed files…")),
            removing: None,
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
            multiple: false,
            prompt: Some("Index this folder".into()),
        });
        self.file_index.picker = Some(cx.spawn_in(window, async move |this, cx| {
            let result = selection.await;
            let _ = this.update_in(cx, |shell, window, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            if shell.state.session_id == session
                                && shell.composer_input.read(cx).value().as_ref() == original
                                && let Some(remaining) = remaining
                            {
                                shell.fill_composer_draft(&remaining, window, cx);
                            }
                            shell.index_command(IndexCommand::AddRoot { path }, cx);
                            shell.select_tab(NavTab::Knowledge, cx);
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => {
                        shell.file_index.state.notice = Some(
                            "The folder picker could not open. Try Index a folder again.".into(),
                        );
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(crate) fn index_command(&mut self, command: IndexCommand, cx: &mut Context<Self>) {
        if let Err(error) = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::FileIndex(command))
        {
            self.file_index.state.notice = Some(error.to_string());
            self.file_index.state.knowledge_reviewing.clear();
        }
        cx.notify();
    }

    fn search_index(&mut self, cx: &mut Context<Self>) {
        let query = IndexQuery {
            text: self.file_index.search.read(cx).value().to_string(),
            roots: vec![],
            path_prefix: None,
            limit: 20,
        };
        if let Err(error) = query.validate() {
            self.file_index.state.notice = Some(error.to_string());
            cx.notify();
            return;
        }
        let request_id = self.file_index.state.begin_search();
        self.index_command(IndexCommand::Search { request_id, query }, cx);
    }
}

pub(crate) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let state = &shell.file_index.state;
    let mut panel = v_flex().id("file-index-panel").test_support().min_w_0().w_full().gap(t.space.small)
        .sovra_surface(Surface::Matte, cx).p(t.space.medium)
        .children((!state.knowledge_pending.is_empty()).then(|| crate::knowledge_review::render(state, cx)))
        .child(div().sovra_text(TextRole::SectionTitle, cx).child("Your indexed folders"))
        .child(div().sovra_text(TextRole::Caption, cx).child("Keyword indexing stays on this device. Optional semantic search sends extracted text and searches to the embedding server you choose for each folder. Chat queries send requested metadata and passages to your chat AI. Hidden files, generated folders and symbolic links are skipped. Automatic updates run while Sovra is open; indexed text is a snapshot."))
        .child(design::button("index-folder", "Index a folder", cx).on_click(cx.listener(|shell, _, window, cx| shell.choose_index_folder(window, cx))))
        .children(state.roots.iter().map(|root| root_row(root, state.monitors.get(&root.id), state.semantic.iter().find(|s| s.setting.root_id == root.id), shell.file_index.removing.as_deref() == Some(root.id.as_str()), cx)));
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
            .child(div().sovra_text(TextRole::SectionTitle,cx).child("Choose an embedding model"))
            .child(div().sovra_text(TextRole::Caption,cx).child("Enabling sends this folder’s extracted passages and future searches to the named server, including newly indexed files. The service may charge for usage. No model is downloaded automatically. Use a model designed for plain-text embeddings; models requiring task prefixes are not supported yet."))
            .child(Input::new(&shell.file_index.embedding_search).aria_label("Filter embedding models"));
        if candidates.is_empty() {
            chooser = chooser.child(div().sovra_text(TextRole::Body,cx).child("No matching models. In Models, inspect an installed Ollama embedding model to load its capabilities, or add an embedding model ID to an OpenAI-compatible connection."));
        }
        for (index, target) in candidates.iter().take(20).enumerate() {
            let root_id = root_id.clone();
            let target = target.clone();
            chooser = chooser.child(
                v_flex()
                    .min_w_0()
                    .gap(t.space.compact)
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .sovra_text(TextRole::Body, cx)
                            .child(target.key.model_id.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .sovra_text(TextRole::Caption, cx)
                            .id(gpui_kit::SharedString::from(format!(
                                "semantic-destination-{index}"
                            )))
                            .test_support()
                            .aria_label(target.base_url.clone())
                            .child(target.base_url.clone()),
                    )
                    .child(
                        design::button(("enable-semantic", index), "Enable for this folder", cx)
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                shell.index_command(
                                    IndexCommand::ConfigureSemantic {
                                        root_id: root_id.clone(),
                                        target: Some(target.clone()),
                                    },
                                    cx,
                                );
                                shell.file_index.semantic_root = None;
                            })),
                    ),
            );
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
    if let Some(notice) = &state.notice {
        panel = panel.child(div().sovra_text(TextRole::Body, cx).child(notice.clone()));
    }
    if !state.roots.is_empty() {
        panel = panel
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
    for passage in &state.passages {
        let preview: String = passage.text.chars().take(800).collect();
        let mut result = v_flex()
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
    panel.children(
        state
            .knowledge_pending
            .is_empty()
            .then(|| crate::knowledge_review::render(state, cx)),
    )
}

fn root_row(
    root: &IndexedRoot,
    monitor: Option<&MonitorStatus>,
    semantic: Option<&sovra_core::file_index::semantic::SemanticStatus>,
    removing: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let status = match root.status {
        IndexStatus::Pending => "Waiting to scan",
        IndexStatus::Scanning => "Indexing",
        IndexStatus::Ready => "Indexed",
        IndexStatus::Cancelled => "Cancelled — refresh to continue",
        IndexStatus::Interrupted => "Interrupted — refresh to recover",
        IndexStatus::Failed => "Needs attention",
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
        )
        .child(div().sovra_text(TextRole::Caption, cx).child(format!(
            "{status} · {} visited · {} updated · {} unchanged · {} skipped · {} failed",
            root.progress.visited,
            root.progress.indexed,
            root.progress.reused,
            root.progress.skipped,
            root.progress.failed
        )));
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
    if let Some(status) = semantic {
        row = row.child(
            div()
                .min_w_0()
                .sovra_text(TextRole::Caption, cx)
                .child(format!(
                    "Semantic search · {} · {} of {} unique passages · {}",
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
                "Disable semantic search",
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
                "Change or rebuild embeddings"
            } else {
                "Set up semantic search"
            },
            cx,
        )
        .on_click(cx.listener(move |shell, _, _, cx| {
            shell.file_index.semantic_root = Some(choose.clone());
            cx.notify();
        })),
    );
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
                "Refresh",
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
    for issue in root.issues.iter().take(5) {
        row = row.child(div().sovra_text(TextRole::Caption, cx).child(issue.clone()));
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

    fn root() -> IndexedRoot {
        IndexedRoot {
            id: "chosen".into(),
            path: format!("/selected/{}/notes", "long-folder-name".repeat(24)).into(),
            status: IndexStatus::Ready,
            progress: IndexProgress::default(),
            issues: vec![],
            completed_at: Some(42),
            auto_refresh: true,
        }
    }

    #[gpui_kit::test]
    fn controls_send_pause_refresh_and_confirmed_removal_without_touching_sources(
        cx: &mut TestAppContext,
    ) {
        let (client, commands, events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1280.), px(1000.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.unwrap();
        events
            .send_blocking(Event::FileIndex(IndexEvent::Roots(vec![root()])))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.select_tab(NavTab::Knowledge, cx));
            window.render_frame(cx);
            window.click("automatic-index-chosen", cx);
            window.click("refresh-index-chosen", cx);
            window.click("remove-index-chosen", cx);
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
        assert!(
            !sent
                .iter()
                .any(|command| matches!(command, Command::FileIndex(IndexCommand::Remove { .. })))
        );
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("confirm-remove-index-chosen", cx);
        })
        .unwrap();
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::FileIndex(IndexCommand::Remove { root_id }) if root_id == "chosen")));
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
                let view = cx.new(|cx| Shell::new(window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.unwrap();
            cx.update_window(window.into(), |_, window, cx| {
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                shell.update(cx, |shell, cx| {
                    shell
                        .file_index
                        .state
                        .apply(&IndexEvent::Roots(vec![root()]));
                    shell.select_tab(NavTab::Knowledge, cx);
                });
                for mode in [ThemeMode::Dark, ThemeMode::Light] {
                    Theme::change(mode, Some(window), cx);
                    window.render_frame(cx);
                    window.render_frame(cx);
                    for id in [
                        "file-index-panel",
                        "automatic-index-chosen",
                        "refresh-index-chosen",
                        "remove-index-chosen",
                        "search-index",
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
