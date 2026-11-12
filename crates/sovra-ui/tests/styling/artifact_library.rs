//! Folder filters and framework source remain within their pane at every scale.
use super::*;
use sovra_core::artifacts::{Artifact, ArtifactDraft, ArtifactEvent, ArtifactKind, ArtifactSource};
use sovra_core::{Event, NavTab, Timestamp};

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
                for id in ["artifact-folders", "artifact-grid", "artifact-page-next"] {
                    let bounds = window.find(id).bounds();
                    assert!(
                        bounds.left() >= page.left() && bounds.right() <= page.right(),
                        "{id} {bounds:?} exceeds {page:?}"
                    );
                }
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
            })
            .unwrap();
        }
    }
}
