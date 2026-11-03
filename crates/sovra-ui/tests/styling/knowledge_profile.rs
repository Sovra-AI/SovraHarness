//! Personal knowledge actions remain reachable and retain drafts during capture.

#![allow(
    clippy::unwrap_used,
    reason = "headless UI fixtures fail loudly when the scripted runtime is gone"
)]

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    App, AppContext as _, ElementId, ScrollDelta, TestAppContext, Window, point, px, size,
};
use sovra_core::{
    Command, EpisodeId, Event, FactId, MemoryAction, MemoryActivity, MemoryCommand, MemoryEvent,
    MemoryJob, MemoryJobState, MemoryOverview, MemoryPage, MemoryReviewEvent, MemoryReviewResult,
    NavTab, Timestamp,
    models::{InterfaceScale, ModelEvent, ModelPreferences},
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

pub(super) fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

pub(super) fn request(commands: &async_channel::Receiver<Command>) -> MemoryCommand {
    std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| match command {
            Command::Memory(command) => Some(command),
            _ => None,
        })
        .expect("a memory action")
}

pub(super) fn respond(
    events: &async_channel::Sender<Event>,
    request_id: u64,
    result: MemoryReviewResult,
) {
    events
        .send_blocking(Event::Memory(MemoryEvent::Review(MemoryReviewEvent {
            request_id,
            result: Ok(result),
        })))
        .expect("the memory review fixture is accepted");
}

fn page(failed: usize) -> MemoryPage {
    MemoryPage {
        overview: Some(MemoryOverview {
            current: 3,
            waiting: 2,
            working: 1,
            failed,
            recent_activity: (0..8)
                .map(|index| MemoryActivity {
                    episode_id: EpisodeId(format!("episode:{index}")),
                    session_id: "fixture".into(),
                    created_at: Timestamp(index),
                    state: MemoryJobState::Complete,
                    saved: 0,
                    error: None,
                })
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub(super) fn reveal(window: &mut Window, id: impl Into<ElementId>, cx: &mut App) {
    let id = id.into();
    for _ in 0..100 {
        window.render_frame(cx);
        let item = window.find(id.clone());
        let bounds = item.bounds();
        let viewport = window.find("sovra-knowledge-view").bounds();
        assert!(
            bounds.left() >= viewport.left() && bounds.right() <= viewport.right(),
            "{id:?} overflows: {bounds:?}"
        );
        let taller_than_pane = bounds.bottom() - bounds.top() > viewport.bottom() - viewport.top();
        let on_screen = bounds.top() < viewport.bottom() && bounds.bottom() > viewport.top();
        let contained = bounds.top() >= viewport.top() && bounds.bottom() <= viewport.bottom();
        // A 600-character excerpt is taller than the 640×420 window at 200% text.
        // It is reachable when the pane shows part of it. Shorter controls must sit fully inside.
        if item.visible() && (contained || (taller_than_pane && on_screen)) {
            return;
        }
        let delta = if bounds.top() < viewport.top() {
            viewport.top() - bounds.top() + px(1.)
        } else {
            viewport.bottom() - bounds.bottom() - px(1.)
        };
        window.scroll(
            "sovra-knowledge-view",
            ScrollDelta::Pixels(point(px(0.), delta)),
            cx,
        );
    }
    panic!("{id:?} must be reachable");
}

#[gpui_kit::test]
fn personal_knowledge_saves_retries_and_preserves_drafts_at_every_size(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                Theme::change(mode, None, cx);
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| {
                    let mut shell = Shell::new(0, window, cx);
                    shell.select_tab(NavTab::Knowledge, cx);
                    shell
                });
                Root::new(shell, window, cx)
            });
            events
                .send_blocking(Event::Models(ModelEvent::Loaded {
                    connections: vec![],
                    preferences: ModelPreferences::default(),
                }))
                .unwrap();
            let load = request(&commands);
            respond(&events, load.request_id, MemoryReviewResult::Page(page(1)));
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(window.find("knowledge-memories").label(), Some("About you"));
                assert!(window.try_find("file-index-panel").is_none());
                for (id, label) in [
                    ("memory-total-current", "3 Saved details"),
                    ("memory-total-pending", "3 Being learned"),
                    ("memory-total-failed", "1 Need attention"),
                ] {
                    reveal(window, id, cx);
                    assert_eq!(window.find(id).label(), Some(label));
                }
                reveal(window, "memory-activity-toggle", cx);
                window.click("memory-activity-toggle", cx);
                window.render_frame(cx);
                for index in 0..8usize {
                    reveal(window, ("memory-activity-item", index), cx);
                    assert!(
                        window
                            .find(("memory-activity-item", index))
                            .label()
                            .unwrap()
                            .contains("no new personal details")
                    );
                }
                reveal(window, "memory-retry-failed", cx);
                window.click("memory-retry-failed", cx);
            })
            .unwrap();
            let retry = request(&commands);
            assert_eq!(retry.action, MemoryAction::RetryFailed);
            respond(&events, retry.request_id, MemoryReviewResult::Retried(1));
            settle(cx);
            let reload = request(&commands);
            respond(
                &events,
                reload.request_id,
                MemoryReviewResult::Page(page(0)),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("memory-retry-failed").is_none());
                reveal(window, "memory-add", cx);
                window.click("memory-add", cx);
                window.render_frame(cx);
                reveal(window, "memory-note-input", cx);
                window.click("memory-note-input", cx);
                window.input("I prefer short answers", cx);
            })
            .unwrap();
            events
                .send_blocking(Event::Memory(MemoryEvent::Processed(MemoryJob {
                    episode_id: EpisodeId("episode:background".into()),
                    state: MemoryJobState::Complete,
                    attempts: 1,
                    error: None,
                    fact_ids: vec![],
                    candidates: vec![],
                    extraction_model: Some("fixture".into()),
                })))
                .unwrap();
            settle(cx);
            assert!(
                !std::iter::from_fn(|| commands.try_recv().ok())
                    .any(|command| matches!(command, Command::Memory(_))),
                "capture must not discard the note draft"
            );
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "memory-add-save", cx);
                window.click("memory-add-save", cx);
                window.render_frame(cx);
                assert_eq!(window.find("memory-add-save").disabled(), Some(true));
            })
            .unwrap();
            let save = request(&commands);
            assert!(
                matches!(save.action, MemoryAction::Remember(note) if note.text == "I prefer short answers")
            );
            respond(
                &events,
                save.request_id,
                MemoryReviewResult::Changed(FactId("fact:note".into())),
            );
            settle(cx);
            let reload = request(&commands);
            respond(
                &events,
                reload.request_id,
                MemoryReviewResult::Page(page(0)),
            );
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("memory-note-form").is_none());
                assert!(
                    window
                        .find("memory-action-notice")
                        .label()
                        .unwrap()
                        .starts_with("Saved exactly")
                );
            })
            .unwrap();
        }
    }
}
