//! Batch folder selection and recoverable per-folder failures.

use super::*;

impl Shell {
    pub(super) fn add_index_folders(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        browser::close(self);
        report::close(self);
        self.select_tab(NavTab::Knowledge, cx);
        self.select_knowledge_section(crate::knowledge_view::KnowledgeSection::Files, cx);
        self.file_index.state.folder_addition = None;
        let noun = if paths.len() == 1 {
            "folder"
        } else {
            "folders"
        };
        self.file_index.state.notice = Some(format!("Connecting {} selected {noun}…", paths.len()));
        self.index_command(IndexCommand::AddRoots { paths }, cx);
    }
}

pub(super) fn render(shell: &Shell, cx: &mut Context<Shell>) -> Option<gpui_kit::AnyElement> {
    let result = shell.file_index.state.folder_addition.as_ref()?;
    let t = design::tokens(cx);
    let mut summary = v_flex()
        .id("index-folder-addition")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .child(div().sovra_text(TextRole::Body, cx).child(format!(
            "{} connected · {} could not be added",
            result.added.len(),
            result.failed.len()
        )))
        .child(div().sovra_text(TextRole::Caption, cx).child(
            "New folders scan in the background. Folders already connected keep their settings.",
        ));
    for (index, failure) in result.failed.iter().enumerate() {
        let path = failure.path.display().to_string();
        summary = summary.child(
            v_flex()
                .id(("index-folder-add-failure", index))
                .test_support()
                .w_full()
                .min_w_0()
                .gap(t.space.compact)
                .child(
                    div()
                        .id(("index-folder-add-path", index))
                        .min_w_0()
                        .text_ellipsis()
                        .aria_label(path.clone())
                        .sovra_text(TextRole::Body, cx)
                        .child(path),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .child(failure.reason.clone()),
                ),
        );
    }
    let mut actions = h_flex().flex_wrap().gap(t.space.small);
    if !result.failed.is_empty() {
        let paths: Vec<_> = result
            .failed
            .iter()
            .map(|failure| failure.path.clone())
            .collect();
        summary = summary.child(div().sovra_text(TextRole::Caption, cx).child(
            "Check that these folders are available and you have permission to read them, then retry."
        ));
        actions = actions.child(
            design::button("index-retry-folders", "Retry failed folders", cx).on_click(
                cx.listener(move |shell, _, _, cx| shell.add_index_folders(paths.clone(), cx)),
            ),
        );
    }
    Some(
        summary
            .child(actions.child(
                design::button("index-dismiss-addition", "Dismiss", cx).on_click(cx.listener(
                    |shell, _, _, cx| {
                        shell.file_index.state.folder_addition = None;
                        cx.notify();
                    },
                )),
            ))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{Root, Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, px, size};
    use sovra_core::file_index::{FolderAddFailure, FolderAddition, IndexEvent};

    #[gpui_kit::test]
    fn selection_errors_fit_each_layout_and_retry_only_failed_paths(cx: &mut TestAppContext) {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            for mode in [ThemeMode::Dark, ThemeMode::Light] {
                let (client, commands, _events) = sovra_runtime::Client::loopback();
                cx.update(|cx| {
                    crate::init(client, cx);
                    Theme::change(mode, None, cx);
                    design::set_interface_scale(
                        sovra_core::models::InterfaceScale::from_percent(scale),
                        cx,
                    );
                });
                let failed: std::path::PathBuf =
                    format!("/chosen/{}/notes", "long folder name ".repeat(20)).into();
                let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                    let view = cx.new(|cx| {
                        let mut shell = Shell::new(0, window, cx);
                        shell.select_tab(NavTab::Knowledge, cx);
                        shell.select_knowledge_section(
                            crate::knowledge_view::KnowledgeSection::Files,
                            cx,
                        );
                        shell
                            .file_index
                            .apply(&IndexEvent::FoldersAdded(FolderAddition {
                                added: vec!["/chosen/ready".into()],
                                failed: vec![FolderAddFailure {
                                    path: failed.clone(),
                                    reason:
                                        "The folder is unavailable. Reconnect its drive and retry."
                                            .into(),
                                }],
                            }));
                        shell
                    });
                    Root::new(view, window, cx)
                });
                cx.update_window(window.into(), |_, window, cx| {
                    window.render_frame(cx);
                    let failure = window.find(("index-folder-add-failure", 0usize)).bounds();
                    assert!(
                        failure.left() >= px(0.) && failure.right() <= px(width),
                        "{width}@{scale}: {failure:?}"
                    );
                    for _ in 0..100 {
                        let control = window.find("index-retry-folders");
                        let viewport = window.find("sovra-knowledge-view").bounds();
                        if control.visible()
                            && control.bounds().top() > viewport.top()
                            && control.bounds().bottom() < viewport.bottom()
                        {
                            break;
                        }
                        let delta = if control.bounds().top() <= viewport.top() {
                            50.
                        } else {
                            -50.
                        };
                        window.scroll(
                            "sovra-knowledge-view",
                            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(delta))),
                            cx,
                        );
                        window.render_frame(cx);
                    }
                    let control = window.find("index-retry-folders");
                    assert!(control.visible());
                    assert!(control.bounds().right() <= px(width));
                    assert!(control.bounds().bottom() < px(height));
                    window.click("index-retry-folders", cx);
                })
                .unwrap();
                let submitted: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
                    .filter_map(|command| {
                        if let Command::FileIndex(IndexCommand::AddRoots { paths }) = command {
                            Some(paths)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(submitted, vec![vec![failed]]);
            }
        }
    }
}
