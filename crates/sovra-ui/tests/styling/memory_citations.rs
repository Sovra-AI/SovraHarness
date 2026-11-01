//! Saved-source navigation through the actual chat and memory views.

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{App, ElementId, ScrollDelta, TestAppContext, point, size};
use gpui_kit::{AppContext as _, Window, px};
use sovra_core::ChatMessage;
use sovra_core::{
    Command, Episode, EpisodeId, EpisodeKind, Event, Fact, FactId, MemoryAction, MemoryDetail,
    MemoryEvent, MemoryReviewEvent, MemoryReviewResult, Timestamp,
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

fn reveal(
    window: &mut Window,
    cx: &mut App,
    area: &'static str,
    id: impl Into<ElementId>,
    height: f32,
) {
    let id = id.into();
    for _ in 0..60 {
        window.render_frame(cx);
        let bounds = window.find(id.clone()).bounds();
        // Content may sit flush under the window edge now that the empty in-app title bar is gone.
        if bounds.top() >= px(8.) && bounds.bottom() <= px(height - 8.) {
            return;
        }
        let delta = if bounds.top() < px(8.) { 60. } else { -60. };
        window.scroll(area, ScrollDelta::Pixels(point(px(0.), px(delta))), cx);
    }
    panic!("control {id:?} must be reachable");
}

fn detail() -> MemoryDetail {
    MemoryDetail {
        fact: Fact {
            id: FactId("fact:tea".into()),
            content: "Prefers tea".into(),
            entities: vec![],
            source: Some(EpisodeId("episode:tea".into())),
            replaces: None,
            extraction: None,
            created_at: Timestamp(1),
            valid_at: Some(Timestamp(1)),
            invalid_at: None,
            expired_at: None,
            stopped_at: Some(Timestamp(2)),
        },
        source: Some(Episode {
            id: EpisodeId("episode:tea".into()),
            session_id: "earlier".into(),
            kind: EpisodeKind::UserMessage,
            content: "I prefer tea.".into(),
            occurred_at: Timestamp(1),
        }),
    }
}

#[gpui_kit::test]
fn reply_citations_open_saved_sources_at_every_size(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (client, commands, events) = Client::loopback();
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
            let mut thread = sovra_core::ChatThread::new("restored", Timestamp(1));
            let mut reply =
                ChatMessage::assistant("reply", "Tea. [memory:fact:tea]", "Earlier", vec![]);
            if mode == ThemeMode::Light {
                reply.speaker = Some(sovra_core::Speaker {
                    member_id: "helper".into(),
                    name: "Mira".into(),
                    icon: "M".into(),
                    role: "Research".into(),
                });
            }
            thread.messages.push(reply);
            events
                .send_blocking(Event::ChatHistoryLoaded {
                    threads: vec![thread],
                })
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update(|cx| shell.update(cx, |shell, cx| shell.select_thread("restored", cx)));
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                let id = ("memory-citation-0", 0usize);
                reveal(window, cx, "chat-scroll-area", id, height);
                let button = window.find(id);
                assert!(button.visible(), "{width}@{scale}: {button:?}");
                assert!(button.bounds().right() <= px(width));
                window.click(id, cx);
                window.render_frame(cx);
                assert_eq!(
                    shell.read(cx).workspace().active_tab,
                    sovra_core::NavTab::Knowledge
                );
                reveal(
                    window,
                    cx,
                    "sovra-memory-source-view",
                    "memory-loading",
                    height,
                );
                assert!(window.find("memory-loading").visible());
                reveal(
                    window,
                    cx,
                    "sovra-memory-source-view",
                    "memory-back-chat",
                    height,
                );
                assert!(window.find("memory-back-chat").visible());
            })
            .unwrap();
            let request = std::iter::from_fn(|| commands.try_recv().ok())
                .find_map(|command| match command {
                    Command::Memory(command)
                        if command.action == MemoryAction::Inspect(FactId("fact:tea".into())) =>
                    {
                        Some(command)
                    }
                    _ => None,
                })
                .unwrap();
            events
                .send_blocking(Event::Memory(MemoryEvent::Review(MemoryReviewEvent {
                    request_id: request.request_id,
                    result: Ok(MemoryReviewResult::Detail(Box::new(detail()))),
                })))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                reveal(
                    window,
                    cx,
                    "sovra-memory-source-view",
                    "memory-source",
                    height,
                );
                assert_eq!(window.find("memory-source").label(), Some("I prefer tea."));
                assert!(window.find("memory-source").bounds().right() <= px(width));
                assert!(window.try_find("memory-stop").is_none());
                assert!(window.try_find("memory-edit").is_none());
                reveal(
                    window,
                    cx,
                    "sovra-memory-source-view",
                    "memory-back-chat",
                    height,
                );
                window.click("memory-back-chat", cx);
                window.render_frame(cx);
                assert_eq!(
                    shell.read(cx).workspace().active_tab,
                    sovra_core::NavTab::Chat
                );
                assert_eq!(
                    shell.read(cx).workspace().messages[0].text,
                    "Tea. [memory:fact:tea]"
                );
                reveal(
                    window,
                    cx,
                    "chat-scroll-area",
                    ("memory-citation-0", 0usize),
                    height,
                );
                window.click(("memory-citation-0", 0usize), cx);
            })
            .unwrap();
            let request = std::iter::from_fn(|| commands.try_recv().ok())
                .filter_map(|command| match command {
                    Command::Memory(command)
                        if matches!(command.action, MemoryAction::Inspect(_)) =>
                    {
                        Some(command)
                    }
                    _ => None,
                })
                .last()
                .unwrap();
            events
                .send_blocking(Event::Memory(MemoryEvent::Review(MemoryReviewEvent {
                    request_id: request.request_id,
                    result: Err("Saved source could not be found.".into()),
                })))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                reveal(
                    window,
                    cx,
                    "sovra-memory-source-view",
                    "memory-error",
                    height,
                );
                assert!(window.find("memory-error").visible());
                assert!(
                    window.try_find("memory-source").is_none(),
                    "a failed lookup must not show the earlier source"
                );
            })
            .unwrap();
        }
    }
}
