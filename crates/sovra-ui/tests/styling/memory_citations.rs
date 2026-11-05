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
        // The fixed header can cover a control inside the window bounds. Reveal
        // it within its actual scroll pane before sending the pointer click.
        let viewport = window.find(area).bounds();
        let top = viewport.top().max(px(8.));
        let bottom = viewport.bottom().min(px(height - 8.));
        if bounds.top() >= top && bounds.bottom() <= bottom {
            return;
        }
        let delta = if bounds.top() < top {
            top - bounds.top() + px(1.)
        } else {
            bottom - bounds.bottom() - px(1.)
        };
        window.scroll(area, ScrollDelta::Pixels(point(px(0.), delta)), cx);
    }
    panic!("control {id:?} must be reachable");
}

fn detail() -> MemoryDetail {
    MemoryDetail {
        neighborhood: sovra_core::MemoryNeighborhood::default(),
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
    for original in [false, true] {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            for (width, height, scale) in
                [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)]
            {
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
                let reply_text = if original {
                    "Tea. [source:episode:tea]"
                } else {
                    "Tea. [memory:fact:tea]"
                };
                let action = if original {
                    MemoryAction::InspectSource(EpisodeId("episode:tea".into()))
                } else {
                    MemoryAction::Inspect(FactId("fact:tea".into()))
                };
                let mut reply = ChatMessage::assistant("reply", reply_text, "Earlier", vec![]);
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
                    if let Some(label) = button.label() {
                        assert!(
                            !label.contains("memory:fact:") && !label.contains("source:episode:"),
                            "{width}@{scale} button: {label}"
                        );
                    }
                    let card = window.find(("memory-recall-card", 0usize));
                    assert!(card.visible(), "{width}@{scale}: {card:?}");
                    if let Some(label) = card.label() {
                        assert!(
                            !label.contains("memory:fact:") && !label.contains("source:episode:"),
                            "{width}@{scale} card: {label}"
                        );
                    }
                    let preview = window.find(("memory-recall-preview", 0usize));
                    if let Some(label) = preview.label() {
                        assert!(
                            !label.contains("memory:fact:") && !label.contains("source:episode:"),
                            "{width}@{scale} preview: {label}"
                        );
                    }
                    if let Some(label) = window.find(("assistant-answer", 0usize)).label() {
                        assert!(
                            !label.contains("memory:fact:") && !label.contains("source:episode:"),
                            "{width}@{scale} answer: {label}"
                        );
                    }
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
                        Command::Memory(command) if command.action == action => Some(command),
                        _ => None,
                    })
                    .unwrap();
                events
                    .send_blocking(Event::Memory(MemoryEvent::Review(MemoryReviewEvent {
                        request_id: request.request_id,
                        result: Ok(if original {
                            MemoryReviewResult::Source(sovra_core::MemorySourceRecord {
                                episode: detail().source.unwrap(),
                                metadata: sovra_core::MemorySourceMetadata::default(),
                            })
                        } else {
                            MemoryReviewResult::Detail(Box::new(detail()))
                        }),
                    })))
                    .unwrap();
                cx.executor()
                    .advance_clock(std::time::Duration::from_millis(16));
                cx.run_until_parked();
                if !original {
                    cx.update_window(window.into(), |_, window, cx| {
                        reveal(
                            window,
                            cx,
                            "sovra-memory-source-view",
                            "memory-supporting-records",
                            height,
                        );
                        window.click("memory-supporting-records", cx);
                    })
                    .unwrap();
                    let sources = super::knowledge_profile::request(&commands);
                    assert!(
                        matches!(&sources.action, MemoryAction::ListSources(query) if query.fact.as_ref() == Some(&detail().fact.id))
                    );
                    super::knowledge_profile::respond(
                        &events,
                        sources.request_id,
                        MemoryReviewResult::Sources(sovra_core::MemorySourcePage::default()),
                    );
                    super::knowledge_profile::settle(cx);
                    cx.update_window(window.into(), |_, window, cx| {
                        super::knowledge_profile::reveal(window, "memory-sources-back-detail", cx);
                        window.click("memory-sources-back-detail", cx);
                    })
                    .unwrap();
                    let back = super::knowledge_profile::request(&commands);
                    assert_eq!(back.action, MemoryAction::Inspect(detail().fact.id));
                    super::knowledge_profile::respond(
                        &events,
                        back.request_id,
                        MemoryReviewResult::Detail(Box::new(detail())),
                    );
                    super::knowledge_profile::settle(cx);
                }
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
                    if original {
                        reveal(
                            window,
                            cx,
                            "sovra-memory-source-view",
                            "memory-stop-source",
                            height,
                        );
                        window.click("memory-stop-source", cx);
                        window.render_frame(cx);
                        reveal(
                            window,
                            cx,
                            "sovra-memory-source-view",
                            "memory-confirm-stop-source",
                            height,
                        );
                        assert!(window.find("memory-confirm-stop-source").visible());
                        reveal(
                            window,
                            cx,
                            "sovra-memory-source-view",
                            "memory-cancel-stop-source",
                            height,
                        );
                        window.click("memory-cancel-stop-source", cx);
                        window.render_frame(cx);
                        assert!(window.try_find("memory-confirm-stop-source").is_none());
                    }
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
                    assert_eq!(shell.read(cx).workspace().messages[0].text, reply_text);
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
                            if matches!(
                                command.action,
                                MemoryAction::Inspect(_) | MemoryAction::InspectSource(_)
                            ) =>
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
}

#[gpui_kit::test]
fn recall_listing_uses_cards_and_hides_hash_tokens(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| {
        sovra_ui::init(client, cx);
        Theme::change(ThemeMode::Light, None, cx);
        design::set_interface_scale(sovra_core::models::InterfaceScale::from_percent(100), cx);
    });
    let mut shell = None;
    let window = cx.open_window(size(px(640.), px(420.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    let live_id = "memory_0565e570b3b344ef836475a27e4586b023fbc91d38c2818650159b612c2c5d57";
    let mint_id = "memory_7fd3744a12b4969536e48da78e98170ba326edea02467a94156dd6dbd46c8d7f";
    let reply_text = format!(
        "Here's what I can see so far. It's not a complete picture, and some details may be missing.\n\
         Where you live: You've mentioned living in Dubai with your girlfriend [memory:fact:{live_id}].\n\
         Food you enjoy: You like mint sauce [memory:fact:{mint_id}].\n\
         You can review and correct what I've saved in Knowledge."
    );
    let mut thread = sovra_core::ChatThread::new("recall-list", Timestamp(1));
    thread.messages.push(ChatMessage::assistant(
        "reply",
        reply_text.clone(),
        "Just now",
        vec![],
    ));
    events
        .send_blocking(Event::ChatHistoryLoaded {
            threads: vec![thread],
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update(|cx| shell.update(cx, |shell, cx| shell.select_thread("recall-list", cx)));
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).workspace().messages[0].text,
            reply_text,
            "stored reply keeps citation tokens for navigation"
        );
        reveal(
            window,
            cx,
            "chat-scroll-area",
            ("memory-citation-0", 0usize),
            420.,
        );
        assert!(window.find(("memory-recall-card", 0usize)).visible());
        assert!(window.find(("memory-recall-status", 0usize)).visible());
        for number in [0usize, 1] {
            let preview = window.find(("memory-recall-preview", number));
            assert!(preview.visible(), "row {number}");
            let label = preview.label().unwrap_or_default();
            assert!(
                !label.contains("memory:fact:") && !label.contains("source:episode:"),
                "preview {number}: {label}"
            );
            let open = window.find(("memory-citation-0", number));
            assert!(open.visible(), "open {number}");
            if let Some(open_label) = open.label() {
                assert!(
                    !open_label.contains("memory:fact:") && !open_label.contains("source:episode:"),
                    "open {number}: {open_label}"
                );
            }
        }
        let dubai_row = window.find(("memory-recall-preview", 0usize));
        let dubai = dubai_row.label().unwrap_or_default();
        assert!(dubai.contains("Dubai"), "{dubai}");
        if let Some(answer) = window.find(("assistant-answer", 0usize)).label() {
            assert!(
                !answer.contains("memory:fact:") && !answer.contains("source:episode:"),
                "{answer}"
            );
        }
        window.click(("memory-citation-0", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).workspace().active_tab,
            sovra_core::NavTab::Knowledge
        );
    })
    .unwrap();
    let inspect = MemoryAction::Inspect(FactId(format!("fact:{live_id}")));
    let request = std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| match command {
            Command::Memory(command) if command.action == inspect => Some(command),
            _ => None,
        })
        .expect("opening a row inspects the stored fact id");
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
            420.,
        );
        assert!(window.find("memory-source").visible());
    })
    .unwrap();
}
