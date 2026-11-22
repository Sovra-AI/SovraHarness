//! Team and Artifacts, end to end through the shell with a scripted runtime.

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, Entity, ScrollDelta, TestAppContext, WindowHandle, point, px, size,
};
use sovra_core::artifacts::{
    Artifact, ArtifactCommand, ArtifactDraft, ArtifactEvent, ArtifactKind, ArtifactSource,
};
use sovra_core::models::InterfaceScale;
use sovra_core::teams::{
    SpeakerState, TeamActivity, TeamCommand, TeamEvent, TeamRoom, default_team,
};
use sovra_core::{
    AgUiEvent, AgUiWidget, Command, Event, LiveActivity, NavTab, ParsedResearchGoal, ResearchBoard,
    ResearchGoalState, Speaker, SubagentOutcome, Timestamp, parse_research_plan,
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (
    WindowHandle<Root>,
    Entity<Shell>,
    async_channel::Receiver<Command>,
    async_channel::Sender<Event>,
) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    events
        .send_blocking(Event::Teams(TeamEvent::Loaded {
            members: default_team(),
        }))
        .expect("headless test fixture is available");
    settle(cx);
    (
        window,
        shell.expect("headless test fixture is available"),
        commands,
        events,
    )
}

fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

fn drain(commands: &async_channel::Receiver<Command>) -> Vec<Command> {
    std::iter::from_fn(|| commands.try_recv().ok()).collect()
}

fn artifact(id: &str, kind: ArtifactKind, content: &str, at: i64) -> Artifact {
    Artifact::from_draft(
        id.into(),
        ArtifactDraft {
            title: format!("{id} title"),
            kind,
            language: if kind == ArtifactKind::Code {
                "python".into()
            } else {
                String::new()
            },
            content: content.into(),
            source: ArtifactSource {
                session_id: "s".into(),
                message_id: "m".into(),
                author: "Priya".into(),
                shared_from: String::new(),
            },
        },
        Timestamp(at),
    )
}

#[gpui_kit::test]
fn ready_loads_the_team_and_library(cx: &mut TestAppContext) {
    let (_window, _shell, commands, events) = open(cx, 1280., 820.);
    drain(&commands);
    events
        .send_blocking(Event::Ready {
            version: "1".into(),
        })
        .unwrap();
    settle(cx);
    let sent = drain(&commands);
    assert!(sent.contains(&Command::Teams(TeamCommand::Load)));
    assert!(sent.contains(&Command::Artifacts(ArtifactCommand::Load)));
}

#[gpui_kit::test]
fn team_page_fits_every_window_and_text_size_and_asks_the_team(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (1280., 820., 200)] {
        let (window, shell, commands, _events) = open(cx, width, height);
        for theme in [ThemeMode::Dark, ThemeMode::Light] {
            cx.update(|cx| {
                Theme::change(theme, None, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Team, cx));
                window.render_frame(cx);
                let page = window.find("sovra-team-view").bounds();
                let ask = window.find("team-ask");
                assert!(ask.visible(), "{width}x{height}@{scale}: {ask:?}");
                assert!(ask.bounds().right() <= page.right());
                let start = window.find("team-start-group");
                assert!(start.visible(), "{width}x{height}@{scale}: {start:?}");
                assert!(start.bounds().right() <= page.right());
                assert!(start.bounds().left() >= page.left());
                let card = window.find(("team-member", 0usize));
                assert!(card.bounds().left() >= page.left());
                assert!(
                    card.bounds().right() <= page.right(),
                    "cards stay inside the page"
                );
            })
            .unwrap();
        }
        cx.update_window(window.into(), |_, window, cx| window.click("team-ask", cx))
            .unwrap();
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);
            let room = &shell.read(cx).workspace().room;
            assert!(room.is_group() && !room.ask_everyone());
            assert!(window.find("composer-team-note").visible());
            assert!(
                window.find("chat-composer-input").visible(),
                "{width}x{height}@{scale}: input {:?} container {:?}",
                window.find("chat-composer-input").bounds(),
                window.find("sovra-composer-container").bounds()
            );
            shell.update(cx, |shell, cx| {
                shell.send_prompt_text("Plan my garden", window, cx);
            });
        })
        .unwrap();
        let sent = drain(&commands);
        let run = sent
            .iter()
            .find_map(|command| match command {
                Command::Teams(TeamCommand::Run {
                    prompt, context, ..
                }) => Some((prompt, context)),
                _ => None,
            })
            .expect("a team chat sends a team run");
        assert_eq!(run.0, "Plan my garden");
        assert!(
            run.1.messages().is_empty(),
            "the current request is separate from prior history"
        );
        cx.update(|cx| design::set_interface_scale(InterfaceScale::default(), cx));
    }
}

#[gpui_kit::test]
fn team_members_can_be_switched_off_and_edited(cx: &mut TestAppContext) {
    let (window, shell, commands, _events) = open(cx, 1280., 820.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Team, cx));
        window.render_frame(cx);
        window.click(("team-member-active", 1usize), cx);
        window.click(("team-member-edit", 2usize), cx);
        window.render_frame(cx);
        assert!(window.find("team-member-form").visible());
        window.click("team-form-save", cx);
    })
    .unwrap();
    let sent = drain(&commands);
    let saved: Vec<_> = sent
        .iter()
        .filter_map(|command| match command {
            Command::Teams(TeamCommand::SaveMember(member)) => Some(member),
            _ => None,
        })
        .collect();
    assert_eq!(saved.len(), 2, "{sent:?}");
    assert_eq!((saved[0].id.as_str(), saved[0].active), ("morgan", false));
    assert_eq!(saved[1].id, "priya");
    assert_eq!(saved[1].name, "Priya", "editing keeps what was there");
}

#[gpui_kit::test]
fn mentions_route_to_the_team_and_agui_events_show_each_member(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 640., 420.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("@priya is coffee healthy?", window, cx);
        });
    })
    .unwrap();
    let session = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    assert!(
        drain(&commands)
            .iter()
            .any(|command| matches!(command, Command::Teams(TeamCommand::Run { .. })))
    );
    let members = default_team();
    let mut activity = TeamActivity::new(1, &members, &["priya".into()]);
    activity.set("priya", SpeakerState::Speaking);
    let priya = Speaker {
        member_id: "priya".into(),
        name: "Priya".into(),
        icon: "🔬".into(),
        role: "Researcher".into(),
    };
    for event in [
        AgUiEvent::RunStarted { run_id: "r".into() },
        AgUiEvent::ActivitySnapshot {
            message_id: "r-team".into(),
            activity: LiveActivity::Team(activity.clone()),
        },
        AgUiEvent::SubagentStarted {
            subagent_run_id: "r-1".into(),
            name: "priya".into(),
            description: "Researcher".into(),
            speaker: priya,
        },
        AgUiEvent::TextMessageStart {
            message_id: "r-1-reply".into(),
            subagent_run_id: Some("r-1".into()),
        },
        AgUiEvent::TextMessageContent {
            message_id: "r-1-reply".into(),
            delta: "In moderation, yes.".into(),
        },
        AgUiEvent::TextMessageEnd {
            message_id: "r-1-reply".into(),
        },
        AgUiEvent::SubagentFinished {
            subagent_run_id: "r-1".into(),
            outcome: SubagentOutcome::Success,
        },
    ] {
        events
            .send_blocking(Event::AgUi {
                session_id: session.clone(),
                event,
            })
            .unwrap();
    }
    settle(cx);
    activity.set("priya", SpeakerState::Done);
    activity.finish("Your team is done");
    for event in [
        AgUiEvent::ActivitySnapshot {
            message_id: "r-team".into(),
            activity: LiveActivity::Team(activity),
        },
        AgUiEvent::RunFinished { run_id: "r".into() },
    ] {
        events
            .send_blocking(Event::AgUi {
                session_id: session.clone(),
                event,
            })
            .unwrap();
    }
    settle(cx);
    let saved = drain(&commands)
        .into_iter()
        .filter_map(|command| {
            if let Command::SaveChatThread { thread } = command {
                Some(thread)
            } else {
                None
            }
        })
        .next_back()
        .expect("team completion must persist the conversation without navigation");
    assert!(
        saved
            .messages
            .iter()
            .any(|message| message.text == "In moderation, yes." && message.author == "Priya")
    );
    assert!(saved.messages.iter().any(|message| {
        message
            .widgets
            .iter()
            .any(|widget| matches!(widget, AgUiWidget::TeamActivity(activity) if activity.finished))
    }));
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = shell.read(cx).workspace();
        assert!(!state.is_generating);
        let reply = state
            .messages
            .iter()
            .find(|m| m.id == "r-1-reply")
            .expect("member reply");
        assert_eq!(reply.author, "Priya");
        assert_eq!(reply.text, "In moderation, yes.");
        let card = state
            .messages
            .iter()
            .position(|m| m.id == "r-team")
            .expect("team card message");
        // The chat keeps the newest text in view; a short window (or the title bar
        // Sovra draws on macOS and Windows) puts the card's first line above it, so
        // scroll up to it as a person would.
        for _ in 0..20 {
            if window.find(("team-activity-status", card * 10)).visible() {
                break;
            }
            window.scroll(
                "chat-scroll-area",
                ScrollDelta::Pixels(point(px(0.), px(40.))),
                cx,
            );
            window.render_frame(cx);
        }
        let status = window.find(("team-activity-status", card * 10));
        assert!(status.visible(), "{status:?}");
        assert!(window.find("chat-composer-input").visible());
    })
    .unwrap();
}

fn owned_label(
    window: &impl gpui_kit::test::TestWindowExt,
    id: impl Into<gpui_kit::ElementId>,
) -> String {
    window.find(id).label().unwrap_or_default().to_owned()
}

fn history_row(window: &impl gpui_kit::test::TestWindowExt, title: &str) -> usize {
    (0..12)
        .find(|&index| owned_label(window, ("chat-thread-title", index)).contains(title))
        .unwrap_or_else(|| panic!("missing conversation {title}"))
}

/// The history list clips rows below its pane. Scroll the same way a person would
/// before clicking, matching the shell history test.
fn click_history(window: &mut gpui_kit::Window, title: &str, cx: &mut gpui_kit::App) {
    let index = history_row(window, title);
    let id = ("chat-thread-row", index);
    for _ in 0..30 {
        let list = window.find("chat-history-list").bounds();
        let row = window.find(id).bounds();
        let above = row.top() < list.top();
        let below = row.bottom() > list.bottom();
        if !above && !below && window.find(id).visible() {
            break;
        }
        let dy = if above { px(48.) } else { px(-48.) };
        window.scroll(
            "chat-history-list",
            ScrollDelta::Pixels(point(px(0.), dy)),
            cx,
        );
        window.render_frame(cx);
    }
    assert!(
        window.find(id).visible(),
        "conversation {title} stayed off the history list"
    );
    window.click(id, cx);
}

/// The team page scrolls. Bring a control fully inside the page, then press it.
fn click_on_team_page(
    window: &mut gpui_kit::Window,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut gpui_kit::App,
) {
    let id = id.into();
    for _ in 0..40 {
        let page = window.find("sovra-team-view").bounds();
        let target = window.find(id.clone()).bounds();
        let above = target.top() < page.top();
        let below = target.bottom() > page.bottom();
        if !above && !below && window.find(id.clone()).visible() {
            break;
        }
        let dy = if above { px(48.) } else { px(-48.) };
        window.scroll(
            "sovra-team-view",
            ScrollDelta::Pixels(point(px(0.), dy)),
            cx,
        );
        window.render_frame(cx);
    }
    let page = window.find("sovra-team-view").bounds();
    let target = window.find(id.clone()).bounds();
    assert!(
        target.top() >= page.top() && target.bottom() <= page.bottom(),
        "{id:?} stayed off the team page: {target:?} page {page:?}"
    );
    window.click(id, cx);
}

/// The icon rail at 640 scrolls. Bring the tab into view, then press it.
fn click_nav(window: &mut gpui_kit::Window, id: &'static str, cx: &mut gpui_kit::App) {
    if let Some(rail) = window.try_find("chat-history-rail-collapsed") {
        for _ in 0..40 {
            let (rail, icon) = (rail.bounds(), window.find(id).bounds());
            if icon.top() >= rail.top()
                && icon.bottom() <= rail.bottom()
                && window.find(id).visible()
            {
                break;
            }
            let dy = if icon.top() < rail.top() { 40. } else { -40. };
            window.scroll(
                "chat-history-rail-collapsed",
                ScrollDelta::Pixels(point(px(0.), px(dy))),
                cx,
            );
            window.render_frame(cx);
        }
    }
    window.click(id, cx);
}

#[gpui_kit::test]
fn team_button_leaves_a_chat_that_already_has_a_message(cx: &mut TestAppContext) {
    let (narrow, _narrow_shell, _narrow_commands, _narrow_events) = open(cx, 640., 420.);
    cx.update_window(narrow.into(), |_, window, cx| {
        window.render_frame(cx);
        click_nav(window, "nav-team", cx);
        window.render_frame(cx);
        let page = window.find("sovra-team-view");
        assert!(page.visible(), "Team stays reachable at 640×420: {page:?}");
        let ask = window.find("team-ask");
        assert!(ask.visible(), "{ask:?}");
        assert!(ask.bounds().right() <= page.bounds().right());
        let start = window.find("team-start-group");
        assert!(start.visible(), "{start:?}");
        assert!(start.bounds().left() >= page.bounds().left());
    })
    .unwrap();

    let (window, shell, commands, _events) = open(cx, 1280., 820.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Just chatting with Sovra", window, cx);
        });
    })
    .unwrap();
    settle(cx);
    let _ordinary = drain(&commands);
    let original = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("composer-team-mode", cx);
    })
    .unwrap();
    settle(cx);
    shell.read_with(cx, |shell, _| {
        let state = shell.workspace();
        let kept = state
            .threads
            .iter()
            .find(|thread| thread.id == original)
            .expect("the chat that already had a message");
        assert!(kept.room.is_ordinary(), "{:?}", kept.room);
        assert!(
            kept.messages
                .iter()
                .any(|message| message.text == "Just chatting with Sovra"),
            "{:?}",
            kept.messages
        );
        assert_ne!(state.session_id, original);
        assert!(state.room.is_group() && !state.room.ask_everyone());
        assert_eq!(state.room, TeamRoom::all_helpers());
    });

    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Team, cx));
        window.render_frame(cx);
        click_on_team_page(window, ("team-member-pick", 1usize), cx);
        click_on_team_page(window, ("team-member-pick", 2usize), cx);
        assert_eq!(
            owned_label(window, ("team-member-pick", 1usize)),
            "In this group"
        );
        assert_eq!(
            owned_label(window, ("team-member-pick", 2usize)),
            "In this group"
        );
        click_on_team_page(window, "team-start-group", cx);
    })
    .unwrap();
    settle(cx);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.workspace().room, TeamRoom::group(["morgan", "priya"]));
    });
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("What should we have for dinner?", window, cx);
        });
    })
    .unwrap();
    settle(cx);
    let sent = drain(&commands);
    let run = sent
        .iter()
        .find_map(|command| match command {
            Command::Teams(TeamCommand::Run {
                session_id,
                prompt,
                room,
                ..
            }) => Some((session_id, prompt, room)),
            _ => None,
        })
        .expect("choosing two helpers starts a team turn");
    assert_eq!(run.1, "What should we have for dinner?");
    assert_eq!(run.2, &TeamRoom::group(["morgan", "priya"]));
    assert!(!run.2.ask_everyone());
    assert_ne!(run.0, &original);
    shell.read_with(cx, |shell, _| {
        let state = shell.workspace();
        assert_eq!(state.session_id, *run.0);
        assert_eq!(state.room, TeamRoom::group(["morgan", "priya"]));
        let kept = state
            .threads
            .iter()
            .find(|thread| thread.id == original)
            .expect("the original chat");
        assert!(kept.room.is_ordinary());
        assert!(
            kept.messages
                .iter()
                .any(|message| message.text == "Just chatting with Sovra")
        );
    });
}

const START_HELPERS: &str = "Start a new chat with my helpers";
const SOVRA_ONLY: &str = "Helpers are in this chat. Click so only Sovra answers";

fn session_id(shell: &Entity<Shell>, cx: &mut TestAppContext) -> String {
    shell.read_with(cx, |shell, _| shell.workspace().session_id.clone())
}

fn ordinary_chat_unchanged(shell: &Entity<Shell>, cx: &mut TestAppContext, original: &str) {
    shell.read_with(cx, |shell, _| {
        let kept = shell
            .workspace()
            .threads
            .iter()
            .find(|thread| thread.id == original)
            .expect("the chat that already had a message");
        assert!(kept.room.is_ordinary(), "{:?}", kept.room);
        assert!(
            kept.messages
                .iter()
                .any(|message| message.text == "Just chatting with Sovra"),
            "{:?}",
            kept.messages
        );
    });
}

fn room_is(shell: &Entity<Shell>, cx: &mut TestAppContext, id: &str, room: &TeamRoom) {
    shell.read_with(cx, |shell, _| {
        let state = shell.workspace();
        assert_eq!(state.session_id, id);
        assert_eq!(&state.room, room);
        assert!(!state.room.ask_everyone());
        let open = state
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .expect("the open chat");
        assert_eq!(&open.room, room);
    });
}

/// History click would be ambiguous once two chats share a title, so this
/// returns by id, then opens Team so the next press is the real control.
fn return_to_ordinary_team_page(
    window: &mut gpui_kit::Window,
    shell: &Entity<Shell>,
    original: &str,
    cx: &mut gpui_kit::App,
) {
    shell.update(cx, |shell, cx| {
        shell.select_thread(original, cx);
        shell.select_tab(NavTab::Team, cx);
    });
    window.render_frame(cx);
}

#[gpui_kit::test]
fn each_helpers_click_opens_another_chat(cx: &mut TestAppContext) {
    let (window, shell, _commands, _events) = open(cx, 1280., 820.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Just chatting with Sovra", window, cx);
        });
    })
    .unwrap();
    settle(cx);
    let original = session_id(&shell, cx);

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(owned_label(window, "composer-team-mode"), START_HELPERS);
        window.click("composer-team-mode", cx);
    })
    .unwrap();
    settle(cx);
    let helpers_a = session_id(&shell, cx);
    assert_ne!(helpers_a, original);
    room_is(&shell, cx, &helpers_a, &TeamRoom::all_helpers());
    ordinary_chat_unchanged(&shell, cx, &original);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(owned_label(window, "composer-team-mode"), SOVRA_ONLY);
        shell.update(cx, |shell, cx| shell.select_thread(&original, cx));
        window.render_frame(cx);
        assert_eq!(owned_label(window, "composer-team-mode"), START_HELPERS);
        window.click("composer-team-mode", cx);
    })
    .unwrap();
    settle(cx);
    let helpers_b = session_id(&shell, cx);
    assert_ne!(helpers_b, helpers_a);
    assert_ne!(helpers_b, original);
    room_is(&shell, cx, &helpers_b, &TeamRoom::all_helpers());
    ordinary_chat_unchanged(&shell, cx, &original);

    cx.update_window(window.into(), |_, window, cx| {
        return_to_ordinary_team_page(window, &shell, &original, cx);
        click_on_team_page(window, "team-ask", cx);
    })
    .unwrap();
    settle(cx);
    let ask_a = session_id(&shell, cx);
    assert_ne!(ask_a, helpers_a);
    assert_ne!(ask_a, helpers_b);
    room_is(&shell, cx, &ask_a, &TeamRoom::all_helpers());
    cx.update_window(window.into(), |_, window, cx| {
        return_to_ordinary_team_page(window, &shell, &original, cx);
        click_on_team_page(window, "team-ask", cx);
    })
    .unwrap();
    settle(cx);
    let ask_b = session_id(&shell, cx);
    assert_ne!(ask_b, ask_a);
    room_is(&shell, cx, &ask_b, &TeamRoom::all_helpers());
    ordinary_chat_unchanged(&shell, cx, &original);

    cx.update_window(window.into(), |_, window, cx| {
        return_to_ordinary_team_page(window, &shell, &original, cx);
        click_on_team_page(window, ("team-member-chat", 4usize), cx);
    })
    .unwrap();
    settle(cx);
    let sam_a = session_id(&shell, cx);
    room_is(&shell, cx, &sam_a, &TeamRoom::direct("sam"));
    cx.update_window(window.into(), |_, window, cx| {
        return_to_ordinary_team_page(window, &shell, &original, cx);
        click_on_team_page(window, ("team-member-chat", 4usize), cx);
    })
    .unwrap();
    settle(cx);
    let sam_b = session_id(&shell, cx);
    assert_ne!(sam_b, sam_a);
    room_is(&shell, cx, &sam_b, &TeamRoom::direct("sam"));
    shell.read_with(cx, |shell, _| {
        let first = shell
            .workspace()
            .threads
            .iter()
            .find(|thread| thread.id == sam_a)
            .expect("the first chat with Sam");
        assert_eq!(first.room, TeamRoom::direct("sam"));
        assert_eq!(first.messages.len(), 0);
    });
    ordinary_chat_unchanged(&shell, cx, &original);

    let mut group_ids = Vec::new();
    for _ in 0..2 {
        cx.update_window(window.into(), |_, window, cx| {
            return_to_ordinary_team_page(window, &shell, &original, cx);
            click_on_team_page(window, ("team-member-pick", 1usize), cx);
            click_on_team_page(window, ("team-member-pick", 2usize), cx);
            click_on_team_page(window, "team-start-group", cx);
        })
        .unwrap();
        settle(cx);
        let id = session_id(&shell, cx);
        room_is(&shell, cx, &id, &TeamRoom::group(["morgan", "priya"]));
        assert!(
            !group_ids.contains(&id),
            "group start focused an older chat: {id} already in {group_ids:?}"
        );
        group_ids.push(id);
    }
    let group_b = group_ids.last().expect("two group starts").clone();
    ordinary_chat_unchanged(&shell, cx, &original);

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(owned_label(window, "composer-team-mode"), SOVRA_ONLY);
        window.click("composer-team-mode", cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(session_id(&shell, cx), group_b);
    shell.read_with(cx, |shell, _| {
        assert!(
            shell.workspace().room.is_ordinary(),
            "{:?}",
            shell.workspace().room
        );
        let open = shell
            .workspace()
            .threads
            .iter()
            .find(|thread| thread.id == group_b)
            .expect("the chat that turned back to Sovra");
        assert!(open.room.is_ordinary());
    });
    ordinary_chat_unchanged(&shell, cx, &original);

    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_thread(&original, cx));
        window.render_frame(cx);
        assert_eq!(owned_label(window, "composer-team-mode"), START_HELPERS);
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Team, cx));
        window.render_frame(cx);
        click_on_team_page(window, ("team-member-ask", 2usize), cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(session_id(&shell, cx), original);
    shell.read_with(cx, |shell, _| {
        assert!(shell.workspace().room.is_ordinary());
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find_by_label("Message Sovra").value(),
            Some("@priya ")
        );
    })
    .unwrap();
    ordinary_chat_unchanged(&shell, cx, &original);
}

fn says_jargon(label: &str) -> bool {
    label.split(|c: char| !c.is_alphanumeric()).any(|word| {
        matches!(
            word.to_ascii_lowercase().as_str(),
            "persona" | "agent" | "subagent"
        )
    })
}

#[gpui_kit::test]
fn one_helper_and_a_group_keep_their_own_members(cx: &mut TestAppContext) {
    let (window, shell, _commands, events) = open(cx, 1280., 820.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Just chatting with Sovra", window, cx);
            shell.select_tab(NavTab::Team, cx);
        });
        window.render_frame(cx);
        window.click("team-start-group", cx);
        window.render_frame(cx);
        assert_eq!(
            owned_label(window, "team-group-hint"),
            "Pick at least two helpers."
        );
        window.click(("team-member-chat", 4usize), cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            owned_label(window, "composer-team-note"),
            "Sam will answer this chat."
        );
        assert!(window.try_find("composer-ask-everyone").is_none());
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Team, cx));
        window.render_frame(cx);
        window.click(("team-member-pick", 1usize), cx);
        window.click(("team-member-pick", 2usize), cx);
        window.click("team-start-group", cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("composer-ask-everyone").checked(), Some(false));
        window.click("composer-ask-everyone", cx);
        window.render_frame(cx);
        assert_eq!(window.find("composer-ask-everyone").checked(), Some(true));
        let group_note = owned_label(window, "composer-team-note");
        assert!(group_note.contains("will all answer"), "{group_note}");
        assert!(!group_note.contains("Sam"), "{group_note}");
        click_history(window, "Chat with Sam", cx);
        window.render_frame(cx);
        assert_eq!(
            owned_label(window, "composer-team-note"),
            "Sam will answer this chat."
        );
        assert!(window.try_find("composer-ask-everyone").is_none());
        click_history(window, "Morgan and Priya", cx);
        window.render_frame(cx);
        assert_eq!(window.find("composer-ask-everyone").checked(), Some(true));
    })
    .unwrap();
    let session = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    let members = default_team();
    let mut activity = TeamActivity::new(1, &members, &["kai".into()]);
    activity.set("kai", SpeakerState::Passed);
    events
        .send_blocking(Event::AgUi {
            session_id: session,
            event: AgUiEvent::ActivitySnapshot {
                message_id: "pass-card".into(),
                activity: LiveActivity::Team(activity),
            },
        })
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let passed = owned_label(window, "team-member-done-0-kai");
        assert!(passed.contains("Kai"), "{passed}");
        assert!(passed.contains("Nothing to add"), "{passed}");
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Team, cx));
        window.render_frame(cx);
        for label in [
            owned_label(window, "team-page-intro"),
            owned_label(window, "team-how-it-works"),
            owned_label(window, ("team-member", 7usize)),
        ] {
            assert!(!says_jargon(&label), "{label}");
        }
    })
    .unwrap();
    let (sam_id, group_id, threads) = shell.read_with(cx, |shell, _| {
        let state = shell.workspace();
        let sam = state
            .threads
            .iter()
            .find(|thread| thread.title == "Chat with Sam")
            .expect("Sam chat")
            .id
            .clone();
        let group = state
            .threads
            .iter()
            .find(|thread| thread.title == "Morgan and Priya")
            .expect("group chat")
            .id
            .clone();
        (sam, group, state.threads.clone())
    });
    let (reopened, restored, _, restored_events) = open(cx, 1280., 820.);
    restored_events
        .send_blocking(Event::ChatHistoryLoaded { threads })
        .unwrap();
    settle(cx);
    cx.update_window(reopened.into(), |_, window, cx| {
        restored.update(cx, |shell, cx| shell.select_thread(&sam_id, cx));
        window.render_frame(cx);
        assert!(restored.read(cx).workspace().room.is_direct());
        assert_eq!(
            owned_label(window, "composer-team-note"),
            "Sam will answer this chat."
        );
        restored.update(cx, |shell, cx| shell.select_thread(&group_id, cx));
        window.render_frame(cx);
        assert!(restored.read(cx).workspace().room.ask_everyone());
        assert_eq!(window.find("composer-ask-everyone").checked(), Some(true));
        let note = owned_label(window, "composer-team-note");
        assert!(!note.contains("Sam"), "{note}");
        assert!(!says_jargon(&note), "{note}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn deep_research_board_and_document_card_stay_in_chat(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 820.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text(
                "/deep-research Compare heat pumps vs gas boilers",
                window,
                cx,
            );
        });
    })
    .unwrap();
    let session = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    // Deep research is a normal prompt route (skill), not a Team page navigation.
    assert!(
        !drain(&commands)
            .iter()
            .any(|command| matches!(command, Command::Teams(TeamCommand::Run { .. })))
    );
    let mut board = ResearchBoard::planning("Compare heat pumps vs gas boilers");
    let goals: Vec<_> = parse_research_plan(
        "- [RESEARCH] Investigate heat pump efficiency ratings today\n\
         - [RESEARCH] Compare running costs versus gas boilers now\n\
         - [DELIVERABLE] Write a homeowner summary for the family",
    )
    .into_iter()
    .map(ParsedResearchGoal::into_goal)
    .collect();
    board.with_plan(goals);
    board.set("goal-0", ResearchGoalState::Working);
    board.set("goal-1", ResearchGoalState::Working);
    for event in [
        AgUiEvent::RunStarted {
            run_id: "research-1".into(),
        },
        AgUiEvent::ActivitySnapshot {
            message_id: "research-1-board".into(),
            activity: LiveActivity::Research(board.clone()),
        },
    ] {
        events
            .send_blocking(Event::AgUi {
                session_id: session.clone(),
                event,
            })
            .unwrap();
    }
    settle(cx);
    board.set("goal-0", ResearchGoalState::Done);
    board.set("goal-1", ResearchGoalState::Done);
    board.begin_writing();
    board.set("goal-2", ResearchGoalState::Done);
    board.finish("Your research document is ready");
    for event in [
        AgUiEvent::ActivitySnapshot {
            message_id: "research-1-board".into(),
            activity: LiveActivity::Research(board),
        },
        AgUiEvent::TextMessageStart {
            message_id: "research-1-closing".into(),
            subagent_run_id: None,
        },
        AgUiEvent::TextMessageContent {
            message_id: "research-1-closing".into(),
            delta: "Your research document is ready. Open it below.".into(),
        },
        AgUiEvent::TextMessageEnd {
            message_id: "research-1-closing".into(),
        },
        AgUiEvent::RunFinished {
            run_id: "research-1".into(),
        },
    ] {
        events
            .send_blocking(Event::AgUi {
                session_id: session.clone(),
                event,
            })
            .unwrap();
    }
    events
        .send_blocking(Event::AgUiSurface {
            session_id: session,
            widget: AgUiWidget::ArtifactSaved {
                artifact_id: "doc-heat-pumps".into(),
                title: "Compare heat pumps vs gas boilers".into(),
                artifact_kind: ArtifactKind::Document,
            },
        })
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = shell.read(cx).workspace();
        assert_eq!(
            state.active_tab,
            NavTab::Chat,
            "no separate Team/Goals window"
        );
        assert!(!state.is_generating);
        let board_msg = state
            .messages
            .iter()
            .position(|m| m.id == "research-1-board")
            .expect("research board message");
        assert!(
            window.find(("research-board", board_msg * 10)).visible(),
            "research board should render in chat"
        );
        assert!(
            window
                .find(("research-board-lanes", board_msg * 10))
                .visible(),
            "kanban lanes should be visible"
        );
        assert!(
            window
                .find(("research-board-status", board_msg * 10))
                .visible(),
            "board status should be visible"
        );
        let closing = state
            .messages
            .iter()
            .find(|m| m.id == "research-1-closing")
            .expect("closing note");
        assert!(closing.text.contains("research document is ready"));
        // Widget ElementIds use message_index × 10 + widget_index (see chat_view).
        let artifact_index = state
            .messages
            .iter()
            .enumerate()
            .find_map(|(message_index, message)| {
                message
                    .widgets
                    .iter()
                    .position(|widget| {
                        matches!(
                            widget,
                            AgUiWidget::ArtifactSaved {
                                artifact_id,
                                artifact_kind: ArtifactKind::Document,
                                ..
                            } if artifact_id == "doc-heat-pumps"
                        )
                    })
                    .map(|widget_index| message_index * 10 + widget_index)
            })
            .expect("document artifact card on a chat message");
        assert!(
            window.find(("artifact-saved", artifact_index)).visible(),
            "document card must render in chat"
        );
        assert!(
            window
                .find(("artifact-saved-open", artifact_index))
                .visible(),
            "Open control on the document card must be visible"
        );
        assert!(window.find("chat-composer-input").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn saving_a_reply_and_browsing_the_library(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 820.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Write a shopping list", window, cx);
        });
    })
    .unwrap();
    let session = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    drain(&commands);
    for event in [
        Event::AgentResponseStarted {
            session_id: session.clone(),
            model: sovra_core::ResponseModel {
                model_id: "m".into(),
                name: "Model".into(),
                provider: "Test".into(),
            },
        },
        Event::AgentChunk {
            session_id: session.clone(),
            text: "# Shopping\n\n- [ ] Milk\n- [ ] Eggs".into(),
        },
        Event::AgentCompleted {
            session_id: session.clone(),
        },
    ] {
        events.send_blocking(event).unwrap();
    }
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(("save-message", 1usize), cx);
    })
    .unwrap();
    let draft = drain(&commands)
        .into_iter()
        .find_map(|command| match command {
            Command::Artifacts(ArtifactCommand::Save(draft)) => Some(draft),
            _ => None,
        })
        .expect("Save sends the reply to Artifacts");
    assert_eq!(draft.kind, ArtifactKind::Checklist);
    assert_eq!(draft.title, "Shopping");
    assert_eq!(draft.source.session_id, session);

    let mut saved = artifact("list", ArtifactKind::Checklist, &draft.content, 3);
    saved.source.message_id = draft.source.message_id;
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![
            saved,
            artifact("code", ArtifactKind::Code, "print('hi')", 2),
            artifact("page", ArtifactKind::Webpage, "<h1>Hi</h1>", 1),
        ])))
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(("save-message", 1usize)).label(),
            Some("Saved to Artifacts")
        );
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
        window.render_frame(cx);
        assert!(window.find(("artifact-card", 2usize)).visible());
        window.click(("artifact-kind", 3usize), cx);
        window.render_frame(cx);
        assert!(window.find(("artifact-card", 0usize)).visible());
        assert!(
            window.find_all(("artifact-card", 1usize)).is_empty(),
            "the filter hides other kinds"
        );
        window.click("artifact-kind-all", cx);
        window.render_frame(cx);
        window.click(("artifact-open", 2usize), cx);
        window.render_frame(cx);
        assert!(window.find("artifact-detail").visible());
        assert!(window.find("artifact-open-browser").visible());
        window.click("artifact-open-browser", cx);
        window.click("artifact-delete", cx);
        window.render_frame(cx);
        window.click("artifact-delete", cx);
    })
    .unwrap();
    let sent = drain(&commands);
    assert!(sent.contains(&Command::Artifacts(ArtifactCommand::Open {
        id: "page".into()
    })));
    assert!(sent.contains(&Command::Artifacts(ArtifactCommand::Delete {
        id: "page".into()
    })));
}

#[gpui_kit::test]
fn artifacts_page_fits_minimum_wide_and_double_size(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (1280., 820., 200)] {
        let (window, shell, _commands, events) = open(cx, width, height);
        events
            .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![artifact(
                "doc",
                ArtifactKind::Document,
                "# A long letter\n\nDear friend, this is a long paragraph that must wrap inside the page rather than overflow it on any screen.",
                1,
            )])))
            .unwrap();
        settle(cx);
        cx.update(|cx| design::set_interface_scale(InterfaceScale::from_percent(scale), cx));
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.open_artifact("doc", cx));
            window.render_frame(cx);
            let page = window.find("sovra-artifacts-view").bounds();
            for id in ["artifact-copy", "artifact-preview"] {
                let element = window.find(id);
                assert!(element.visible(), "{width}x{height}@{scale}: {id}");
                assert!(element.bounds().right() <= page.right(), "{id} overflows");
            }
        })
        .unwrap();
        cx.update(|cx| design::set_interface_scale(InterfaceScale::default(), cx));
    }
}

#[gpui_kit::test]
fn saved_website_has_run_and_stop_controls(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 820.);
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Saved(Box::new(artifact(
            "website",
            ArtifactKind::Webpage,
            "<h1>Garden</h1>",
            1,
        )))))
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.open_artifact("website", cx));
        window.render_frame(cx);
        window.click("artifact-deploy", cx);
    })
    .unwrap();
    assert!(
        drain(&commands).contains(&Command::Artifacts(ArtifactCommand::Deploy {
            id: "website".into()
        }))
    );
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Deployed {
            id: "website".into(),
            url: "http://127.0.0.1:1234/".into(),
        }))
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("artifact-website-open").visible());
        window.click("artifact-website-stop", cx);
    })
    .unwrap();
    assert!(
        drain(&commands).contains(&Command::Artifacts(ArtifactCommand::StopDeployment {
            id: "website".into()
        }))
    );
}

#[gpui_kit::test]
fn unfinished_source_never_offers_to_run_an_older_website(cx: &mut TestAppContext) {
    let (window, _shell, _commands, events) = open(cx, 1280., 820.);
    let session = sovra_core::WorkspaceState::new(0).session_id;
    let mut old = artifact("older", ArtifactKind::Webpage, "<h1>Garden old</h1>", 1);
    old.source.session_id.clone_from(&session);
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Saved(Box::new(old))))
        .unwrap();
    events
        .send_blocking(Event::AgUi {
            session_id: session,
            event: AgUiEvent::ActivitySnapshot {
                message_id: "source".into(),
                activity: LiveActivity::CodeWriting {
                    artifact_id: None,
                    language: "html".into(),
                    content: "<h1>Garden".into(),
                },
            },
        })
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("code-writing-panel").visible());
        assert!(
            window.try_find("code-run-website").is_none(),
            "unfinished source is not the older saved website"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn no_results_can_clear_combined_filters_without_losing_saved_work(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 2560., 1440.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
        window.render_frame(cx);
        assert!(window.find("artifacts-empty").visible());
        assert!(window.try_find("artifact-search").is_none());
        assert!(window.try_find("artifact-favorites").is_none());
        assert!(window.try_find("artifact-page-next").is_none());
        window.click("artifacts-open-chat", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);
    })
    .unwrap();
    let mut garden = artifact("garden", ArtifactKind::Document, "Planting list", 1);
    garden.collection = "Garden".into();
    garden.pinned = true;
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![
            garden,
            artifact("recipe", ArtifactKind::Note, "Bread recipe", 2),
        ])))
        .unwrap();
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
        window.render_frame(cx);
        window.click(("artifact-folder", 1usize), cx);
        window.click("artifact-favorites", cx);
        window.click(("artifact-kind", 0usize), cx);
        window.click("artifact-search", cx);
        window.input("no such saved work", cx);
        window.render_frame(cx);
        assert!(window.find_all(("artifact-card", 0usize)).is_empty());
        window.click("artifact-clear-filters", cx);
        window.render_frame(cx);
        assert!(window.find(("artifact-card", 0usize)).visible());
        assert!(window.find(("artifact-card", 1usize)).visible());
        assert!(window.try_find("artifact-clear-filters").is_none());
    })
    .unwrap();
    assert!(
        !drain(&commands)
            .iter()
            .any(|c| matches!(c, Command::Artifacts(_)))
    );
}

#[gpui_kit::test]
fn large_library_pages_folders_and_favorites(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 2560., 1440.);
    let saved: Vec<_> = (0..30)
        .map(|i| {
            let mut item = artifact(
                &format!("item-{i:02}"),
                ArtifactKind::Document,
                "A saved document",
                i,
            );
            item.collection = if i < 5 {
                "Garden".into()
            } else {
                String::new()
            };
            item.pinned = i == 0;
            item
        })
        .collect();
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(saved)))
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
        window.render_frame(cx);
        assert!(!window.find_all(("artifact-card", 23usize)).is_empty());
        assert!(window.find_all(("artifact-card", 24usize)).is_empty());
        window.click("artifact-page-next", cx);
        window.render_frame(cx);
        assert!(!window.find_all(("artifact-card", 24usize)).is_empty());
        assert!(window.find_all(("artifact-card", 0usize)).is_empty());
        window.click(("artifact-folder", 1usize), cx);
        window.render_frame(cx);
        assert!(!window.find_all(("artifact-card", 4usize)).is_empty());
        assert!(window.find_all(("artifact-card", 5usize)).is_empty());
        window.click("artifact-favorites", cx);
        window.render_frame(cx);
        assert!(!window.find_all(("artifact-card", 0usize)).is_empty());
        assert!(window.find_all(("artifact-card", 1usize)).is_empty());
        window.click(("artifact-open", 0usize), cx);
        window.render_frame(cx);
        window.click("artifact-favorite", cx);
    })
    .unwrap();
    assert!(drain(&commands).iter().any(|command| matches!(command, Command::Artifacts(ArtifactCommand::Organize { collection, pinned: false, .. }) if collection == "Garden")));
}

#[gpui_kit::test]
fn framework_project_opens_approval_chat_and_selects_source_files(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 820.);
    let project = serde_json::json!({"files":{"package.json":"{\"scripts\":{\"build\":\"vite build\"}}", "src/App.jsx":"export default () => <h1>Garden</h1>"}}).to_string();
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Saved(Box::new(artifact(
            "react",
            ArtifactKind::Project,
            &project,
            1,
        )))))
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.open_artifact("react", cx));
        window.render_frame(cx);
        window.click(("artifact-project-file", 1usize), cx);
        window.render_frame(cx);
        assert!(!window.find_all("artifact-project-source").is_empty());
        window.click("artifact-build-project", cx);
    })
    .unwrap();
    assert!(drain(&commands).iter().any(|command| matches!(command, Command::Artifacts(ArtifactCommand::RunProject { id, session_id }) if id == "react" && !session_id.is_empty())));
}

#[gpui_kit::test]
fn a_saved_web_page_card_opens_the_browser_and_shows_where(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 820.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Make me a counter page", window, cx);
        });
    })
    .unwrap();
    let session = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    for event in [
        Event::AgentResponseStarted {
            session_id: session.clone(),
            model: sovra_core::ResponseModel {
                model_id: "m".into(),
                name: "Model".into(),
                provider: "Test".into(),
            },
        },
        Event::AgentChunk {
            session_id: session.clone(),
            text: "Here is your counter page.".into(),
        },
        Event::AgentCompleted {
            session_id: session.clone(),
        },
        Event::AgUiSurface {
            session_id: session,
            widget: AgUiWidget::ArtifactSaved {
                artifact_id: "page-counter".into(),
                title: "Counter".into(),
                artifact_kind: ArtifactKind::Webpage,
            },
        },
    ] {
        events.send_blocking(event).unwrap();
    }
    settle(cx);
    drain(&commands);
    let index = shell.read_with(cx, |shell, _| {
        shell
            .workspace()
            .messages
            .iter()
            .enumerate()
            .find_map(|(message_index, message)| {
                message
                    .widgets
                    .iter()
                    .position(|widget| matches!(widget, AgUiWidget::ArtifactSaved { .. }))
                    .map(|widget_index| message_index * 10 + widget_index)
            })
            .expect("web page card on a chat message")
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(("artifact-saved-browser", index)).label(),
            Some("Open in browser")
        );
        assert!(window.try_find(("artifact-saved-address", index)).is_none());
        window.click(("artifact-saved-browser", index), cx);
    })
    .unwrap();
    assert!(
        drain(&commands).iter().any(|command| matches!(
            command,
            Command::Artifacts(ArtifactCommand::Open { id }) if id == "page-counter"
        )),
        "the main button opens the page in the browser"
    );
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Deployed {
            id: "page-counter".into(),
            url: "http://127.0.0.1:24680/".into(),
        }))
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(("artifact-saved-address", index)).visible());
        assert!(window.find(("artifact-saved-copy-link", index)).visible());
        assert_eq!(
            shell.read(cx).workspace().active_tab,
            NavTab::Chat,
            "opening the page keeps the person in chat"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn leaving_an_edited_artifact_requires_a_choice_and_keeps_the_draft(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 1000.);
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![
            artifact("doc", ArtifactKind::Document, "Original content", 1),
            artifact("other", ArtifactKind::Document, "Other content", 2),
        ])))
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.open_artifact("doc", cx));
        window.render_frame(cx);
        window.click("artifact-edit", cx);
        window.click("artifact-edit-title", cx);
        window.press("end", cx);
        window.input(" changed", cx);
        window.click("artifact-back", cx);
        assert!(window.find("artifact-unsaved-choice").visible());
        window.click("artifact-keep-editing", cx);
        assert!(window.find("artifact-edit-form").visible());
        window.click("artifact-edit-save", cx);
    })
    .unwrap();
    assert!(drain(&commands).iter().any(|command| matches!(command,
        Command::Artifacts(ArtifactCommand::Edit { id, title, .. }) if id == "doc" && title.contains("changed")
    )));
}

#[gpui_kit::test]
fn cancel_and_open_another_artifact_protect_changes_but_clean_edits_leave_freely(
    cx: &mut TestAppContext,
) {
    let (window, shell, commands, events) = open(cx, 1280., 1000.);
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![
            artifact("doc", ArtifactKind::Document, "Original content", 1),
            artifact("other", ArtifactKind::Document, "Other content", 2),
        ])))
        .unwrap();
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.open_artifact("doc", cx));
        window.render_frame(cx);
        window.click("artifact-edit", cx);
        window.click("artifact-edit-cancel", cx);
        assert!(window.try_find("artifact-unsaved-choice").is_none());
        window.click("artifact-edit", cx);
        window.click("artifact-edit-content", cx);
        window.input("Changed", cx);
        window.click("artifact-edit-cancel", cx);
        assert!(window.find("artifact-unsaved-choice").visible());
        window.click("artifact-keep-editing", cx);
        shell.update(cx, |shell, cx| shell.open_artifact("other", cx));
        window.render_frame(cx);
        assert!(window.find("artifact-unsaved-choice").visible());
        window.click("artifact-discard-changes", cx);
        window.click("artifact-edit", cx);
        window.click("artifact-edit-save", cx);
    })
    .unwrap();
    let edits: Vec<_> = drain(&commands)
        .into_iter()
        .filter_map(|command| match command {
            Command::Artifacts(ArtifactCommand::Edit { id, title, .. }) => Some((id, title)),
            _ => None,
        })
        .collect();
    assert_eq!(edits, vec![("other".into(), "other title".into())]);
}

#[gpui_kit::test]
fn artifact_save_waits_for_confirmation_and_preserves_changes_after_failure(
    cx: &mut TestAppContext,
) {
    let (window, shell, commands, events) = open(cx, 1280., 1000.);
    let original = artifact("doc", ArtifactKind::Document, "Original content", 1);
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![
            original.clone(),
        ])))
        .unwrap();
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.open_artifact("doc", cx));
        window.render_frame(cx);
        window.click("artifact-edit", cx);
        window.click("artifact-edit-title", cx);
        window.input("Changed", cx);
        window.click("artifact-edit-save", cx);
        assert!(window.find("artifact-edit-form").visible());
    })
    .unwrap();
    let first = drain(&commands)
        .into_iter()
        .find(|command| matches!(command, Command::Artifacts(ArtifactCommand::Edit { .. })))
        .unwrap();
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Error(
            "Test storage unavailable".into(),
        )))
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("artifact-save-error").visible());
        window.click("artifact-edit-save", cx);
        assert!(window.find("artifact-edit-form").visible());
    })
    .unwrap();
    assert!(drain(&commands).contains(&first));
    let mut saved = original;
    if let Command::Artifacts(ArtifactCommand::Edit { title, content, .. }) = first {
        saved.title = title;
        saved.content = content;
    }
    saved.updated_at = Timestamp(2);
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Saved(Box::new(saved))))
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("artifact-edit-form").is_none());
        assert!(window.find("artifact-detail").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn completed_artifact_save_does_not_hide_newer_typing(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 1000.);
    let original = artifact("doc", ArtifactKind::Document, "Original content", 1);
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![
            original.clone(),
        ])))
        .unwrap();
    settle(cx);
    drain(&commands);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.open_artifact("doc", cx));
        window.render_frame(cx);
        window.click("artifact-edit", cx);
        window.click("artifact-edit-save", cx);
        window.click("artifact-edit-title", cx);
        window.input("Newer", cx);
    })
    .unwrap();
    drain(&commands);
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Saved(Box::new(original))))
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("artifact-edit-form").visible());
        window.click("artifact-edit-save", cx);
    })
    .unwrap();
    assert!(drain(&commands).iter().any(|command| matches!(command,
        Command::Artifacts(ArtifactCommand::Edit {title, ..}) if title.contains("Newer")
    )));
}
