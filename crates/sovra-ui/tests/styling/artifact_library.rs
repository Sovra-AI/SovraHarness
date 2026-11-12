//! Folder filters and framework source remain within their pane at every scale.
use super::*;
use sovra_core::artifacts::{Artifact, ArtifactDraft, ArtifactEvent, ArtifactKind, ArtifactSource};
use sovra_core::{Event, NavTab, Timestamp};

fn reveal(window: &mut gpui_kit::Window, id: &'static str, cx: &mut gpui_kit::App) {
    for _ in 0..40 {
        window.render_frame(cx);
        let page = window.find("sovra-artifacts-view").bounds();
        let item = window.find(id).bounds();
        assert!(item.left() >= page.left() && item.right() <= page.right());
        if item.top() >= page.top() && item.bottom() <= page.bottom() {
            return;
        }
        let delta = if item.top() < page.top() {
            page.top() - item.top()
        } else {
            page.bottom() - item.bottom()
        };
        window.scroll(
            "sovra-artifacts-view",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), delta)),
            cx,
        );
    }
    panic!("{id} must be reachable by scrolling");
}

#[gpui_kit::test]
fn organized_library_and_project_fit_both_themes(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (client, _commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(
                    sovra_core::models::InterfaceScale::from_percent(scale),
                    cx,
                );
            });
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.unwrap();
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
                reveal(window, "artifacts-open-chat", cx);
                window.click("artifacts-open-chat", cx);
                assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);
            })
            .unwrap();
            let content = serde_json::json!({"files":{"package.json":"{\"scripts\":{\"build\":\"vite build\"}}","src/App.jsx":"export default () => <h1>Garden</h1>"}}).to_string();
            let mut artifact = Artifact::from_draft(
                "project".into(),
                ArtifactDraft {
                    title: "Garden project".into(),
                    kind: ArtifactKind::Project,
                    language: "website-project".into(),
                    content,
                    source: ArtifactSource::default(),
                },
                Timestamp(1),
            );
            artifact.collection = "Garden".into();
            events
                .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![artifact])))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
                window.render_frame(cx);
                let page = window.find("sovra-artifacts-view").bounds();
                assert!(window.try_find("artifact-page-next").is_none());
                for id in ["artifact-folders", "artifact-grid", "artifact-search"] {
                    let bounds = window.find(id).bounds();
                    assert!(
                        bounds.left() >= page.left() && bounds.right() <= page.right(),
                        "{id} {bounds:?} exceeds {page:?}"
                    );
                }
                reveal(window, "artifact-favorites", cx);
                window.click("artifact-favorites", cx);
                reveal(window, "artifact-clear-filters", cx);
                window.click("artifact-clear-filters", cx);
                window.render_frame(cx);
                assert!(window.try_find("artifact-grid").is_some());
                assert!(window.try_find("artifact-clear-filters").is_none());
                shell.update(cx, |shell, cx| shell.open_artifact("project", cx));
                window.render_frame(cx);
                for id in [
                    "artifact-build-project",
                    "artifact-project-source",
                    "artifact-move-folder",
                ] {
                    let bounds = window.find(id).bounds();
                    assert!(
                        bounds.left() >= page.left() && bounds.right() <= page.right(),
                        "{id} {bounds:?} exceeds {page:?}"
                    );
                }
                reveal(window, "artifact-edit", cx);
                window.click("artifact-edit", cx);
                reveal(window, "artifact-edit-title", cx);
                window.click("artifact-edit-title", cx);
                window.input(" draft", cx);
                reveal(window, "artifact-edit-cancel", cx);
                window.click("artifact-edit-cancel", cx);
                for id in ["artifact-keep-editing", "artifact-discard-changes"] {
                    reveal(window, id, cx);
                }
                window.click("artifact-discard-changes", cx);
                assert!(window.try_find("artifact-unsaved-choice").is_none());
                assert!(window.try_find("artifact-edit-form").is_none());
                reveal(window, "artifact-edit", cx);
                window.click("artifact-edit", cx);
                reveal(window, "artifact-edit-save", cx);
                window.click("artifact-edit-save", cx);
            })
            .unwrap();
            events
                .send_blocking(Event::Artifacts(ArtifactEvent::Error(
                    "The test folder is unavailable.".into(),
                )))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "artifact-save-error", cx);
                reveal(window, "artifact-edit-save", cx);
                reveal(window, "artifact-edit-cancel", cx);
                assert!(window.find("artifact-edit-form").bounds().size.height > px(0.));
            })
            .unwrap();
        }
    }
}
