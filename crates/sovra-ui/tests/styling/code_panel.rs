//! Source writing stays usable across window sizes and interface scales.
use super::*;
use sovra_core::{AgUiEvent, Event, LiveActivity};

#[gpui_kit::test]
fn code_panel_fits_and_can_return_to_chat(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for (width, height, percent) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)]
        {
            let (client, _commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(
                    sovra_core::models::InterfaceScale::from_percent(percent),
                    cx,
                );
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| Shell::new(0, window, cx));
                Root::new(shell, window, cx)
            });
            let saved = sovra_core::artifacts::Artifact::from_draft(
                "website".into(),
                sovra_core::artifacts::ArtifactDraft {
                    title: "Garden club".into(),
                    kind: sovra_core::artifacts::ArtifactKind::Webpage,
                    language: "html".into(),
                    content: "<!doctype html>\n<h1>Garden club</h1>".into(),
                    source: sovra_core::artifacts::ArtifactSource {
                        session_id: sovra_core::WorkspaceState::new(0).session_id,
                        ..Default::default()
                    },
                },
                sovra_core::Timestamp(1),
            );
            events
                .send_blocking(Event::Artifacts(
                    sovra_core::artifacts::ArtifactEvent::Saved(Box::new(saved)),
                ))
                .unwrap();
            events
                .send_blocking(Event::Artifacts(
                    sovra_core::artifacts::ArtifactEvent::Deployed {
                        id: "website".into(),
                        url: "http://127.0.0.1:1234/".into(),
                    },
                ))
                .unwrap();
            events
                .send_blocking(Event::AgUi {
                    session_id: sovra_core::WorkspaceState::new(0).session_id,
                    event: AgUiEvent::ActivitySnapshot {
                        message_id: "website-source".into(),
                        activity: LiveActivity::CodeWriting {
                            artifact_id: Some("website".into()),
                            language: "html".into(),
                            content: "<!doctype html>\n<h1>Garden club</h1>".into(),
                        },
                    },
                })
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                for id in ["code-writing-panel", "code-panel-close"] {
                    let bounds = window.find(id).bounds();
                    assert!(
                        bounds.left() >= px(0.) && bounds.right() <= px(width),
                        "{id}: {bounds:?}"
                    );
                    assert!(
                        bounds.top() >= px(0.) && bounds.bottom() <= px(height),
                        "{id}: {bounds:?}"
                    );
                    assert!(bounds.size.height > px(0.), "{id}");
                }
                for _ in 0..30 {
                    let bounds = window.find("code-writing-source").bounds();
                    if bounds.bottom() <= px(height) {
                        break;
                    }
                    window.scroll(
                        "code-writing-panel",
                        gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-40.))),
                        cx,
                    );
                    window.render_frame(cx);
                }
                let source = window.find("code-writing-source").bounds();
                assert!(
                    source.size.height > px(0.)
                        && source.top() >= px(0.)
                        && source.bottom() <= px(height),
                    "source can be scrolled into view: {source:?}"
                );
                window.scroll(
                    "code-writing-panel",
                    gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(2000.))),
                    cx,
                );
                window.render_frame(cx);
                window.click("code-panel-close", cx);
                window.render_frame(cx);
                assert!(window.try_find("code-writing-panel").is_none());
            })
            .unwrap();
            cx.update(|cx| {
                design::set_interface_scale(sovra_core::models::InterfaceScale::default(), cx);
            });
        }
    }
}
